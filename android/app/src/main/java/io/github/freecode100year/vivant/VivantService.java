package io.github.freecode100year.vivant;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;

import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.File;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;

/**
 * Foreground service that runs the VIVANT server (the Rust binary shipped as
 * libvivant.so) as a child process, keeps the CPU and Wi-Fi awake while it
 * runs, restarts it if it crashes, and polls its /healthz for the online count.
 */
public class VivantService extends Service {
    static final String ACTION_START = "io.github.freecode100year.vivant.START";
    static final String ACTION_STOP = "io.github.freecode100year.vivant.STOP";
    private static final String CHANNEL = "vivant";
    private static final int NOTIF_ID = 1;
    private static final int MAX_QUICK_FAILS = 3;

    private final Handler main = new Handler(Looper.getMainLooper());
    private Process proc;
    private long startedAt;
    private boolean wanted;
    private int quickFails;
    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private Thread poller;

    @Override
    public IBinder onBind(Intent intent) { return null; }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        if (ACTION_STOP.equals(action)) {
            Prefs.setRunning(this, false);
            shutdown();
            return START_NOT_STICKY;
        }
        // restarted by the system after being killed: only come back if the user left it on
        if (intent == null && !Prefs.running(this)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        goForeground();
        Prefs.setRunning(this, true);
        wanted = true;
        quickFails = 0;
        ServerState.error = null;
        acquireLocks();
        if (proc == null) launch();
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        wanted = false;
        killProcess();
        releaseLocks();
        ServerState.running = false;
        super.onDestroy();
    }

    // ---- process -------------------------------------------------------

    private void launch() {
        if (!wanted || proc != null) return;
        int port = Prefs.port(this);
        File bin = new File(getApplicationInfo().nativeLibraryDir, "libvivant.so");
        List<String> cmd = new ArrayList<>();
        cmd.add(bin.getAbsolutePath());
        cmd.add("-addr");
        cmd.add(":" + port);
        cmd.add("-host-key");
        cmd.add(new File(getFilesDir(), "host_ed25519").getAbsolutePath());
        cmd.add("-health-addr");
        cmd.add("127.0.0.1:" + (port + 1));
        cmd.add("-max-clients");
        cmd.add(String.valueOf(Prefs.maxClients(this)));
        // a phone usually has no use for the Tor exit list and it costs mobile data
        cmd.add("-tor-exits=false");
        String rooms = Prefs.rooms(this).trim();
        if (!rooms.isEmpty()) {
            cmd.add("-rooms");
            cmd.add(rooms);
        }
        cmd.addAll(splitArgs(Prefs.extraArgs(this)));

        ProcessBuilder pb = new ProcessBuilder(cmd).redirectErrorStream(true).directory(getFilesDir());
        Map<String, String> env = pb.environment();
        env.put("HOME", getFilesDir().getAbsolutePath());
        env.put("TMPDIR", getCacheDir().getAbsolutePath());
        final Process p;
        try {
            p = pb.start();
        } catch (Exception e) {
            ServerState.log("无法启动 · cannot start: " + e.getMessage());
            fail(getString(R.string.err_start, e.getMessage()));
            return;
        }
        proc = p;
        startedAt = SystemClock.elapsedRealtime();
        ServerState.running = true;
        ServerState.port = port;
        ServerState.online = 0;
        updateNotification();

        Thread reader = new Thread(() -> {
            try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while ((line = r.readLine()) != null) ServerState.log(line);
            } catch (Exception ignored) {
            }
            int code;
            try {
                code = p.waitFor();
            } catch (InterruptedException e) {
                code = -1;
            }
            final int c = code;
            main.post(() -> onExit(p, c));
        }, "vivant-log");
        reader.setDaemon(true);
        reader.start();
        startPoller(p, port);
    }

    private void onExit(Process p, int code) {
        if (p != proc) return; // a process we stopped ourselves
        proc = null;
        ServerState.running = false;
        ServerState.online = -1;
        if (!wanted) return;
        ServerState.log("服务器退出 · server exited, code " + code);
        if (SystemClock.elapsedRealtime() - startedAt < 10_000) quickFails++;
        else quickFails = 0;
        if (quickFails >= MAX_QUICK_FAILS) {
            fail(getString(R.string.err_crash, code));
            return;
        }
        updateNotification();
        main.postDelayed(this::launch, 2000);
    }

    /** Gives up: leaves the error for the screen and stops the service. */
    private void fail(String why) {
        ServerState.error = why;
        Prefs.setRunning(this, false);
        shutdown();
    }

    private void shutdown() {
        wanted = false;
        killProcess();
        releaseLocks();
        ServerState.running = false;
        ServerState.online = -1;
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    private void killProcess() {
        main.removeCallbacksAndMessages(null);
        if (poller != null) poller.interrupt();
        poller = null;
        final Process p = proc;
        proc = null;
        if (p == null) return;
        p.destroy(); // SIGTERM: the server shuts down cleanly
        Thread t = new Thread(() -> {
            try {
                if (!p.waitFor(3, TimeUnit.SECONDS)) p.destroyForcibly();
            } catch (InterruptedException ignored) {
                p.destroyForcibly();
            }
        });
        t.setDaemon(true);
        t.start();
        ServerState.log("已停止 · stopped");
    }

    // ---- /healthz ------------------------------------------------------

    private void startPoller(Process p, int port) {
        if (poller != null) poller.interrupt();
        poller = new Thread(() -> {
            int lastOnline = -2;
            while (!Thread.currentThread().isInterrupted() && p.isAlive()) {
                try {
                    Thread.sleep(3000);
                } catch (InterruptedException e) {
                    return;
                }
                JSONObject st = health(port + 1);
                if (st == null) continue;
                ServerState.online = st.optInt("online", 0);
                ServerState.rooms = st.optInt("rooms", 0);
                if (ServerState.online != lastOnline) {
                    lastOnline = ServerState.online;
                    main.post(this::updateNotification);
                }
            }
        }, "vivant-health");
        poller.setDaemon(true);
        poller.start();
    }

    private static JSONObject health(int port) {
        try (Socket s = new Socket()) {
            s.connect(new InetSocketAddress("127.0.0.1", port), 1000);
            s.setSoTimeout(2000);
            OutputStream o = s.getOutputStream();
            o.write("GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n".getBytes(StandardCharsets.US_ASCII));
            o.flush();
            InputStream in = s.getInputStream();
            byte[] buf = new byte[4096];
            int n = 0, k;
            while (n < buf.length && (k = in.read(buf, n, buf.length - n)) > 0) n += k;
            String resp = new String(buf, 0, n, StandardCharsets.UTF_8);
            int i = resp.indexOf("\r\n\r\n");
            return i < 0 ? null : new JSONObject(resp.substring(i + 4).trim());
        } catch (Exception e) {
            return null;
        }
    }

    // ---- foreground / locks ---------------------------------------------

    private void goForeground() {
        NotificationManager nm = getSystemService(NotificationManager.class);
        if (nm.getNotificationChannel(CHANNEL) == null) {
            NotificationChannel ch = new NotificationChannel(CHANNEL, getString(R.string.channel_name), NotificationManager.IMPORTANCE_LOW);
            ch.setShowBadge(false);
            nm.createNotificationChannel(ch);
        }
        Notification n = buildNotification();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        } else {
            startForeground(NOTIF_ID, n);
        }
    }

    private void updateNotification() {
        if (!wanted) return;
        getSystemService(NotificationManager.class).notify(NOTIF_ID, buildNotification());
    }

    private Notification buildNotification() {
        PendingIntent open = PendingIntent.getActivity(this, 0,
                new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(this, 1,
                new Intent(this, VivantService.class).setAction(ACTION_STOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        int port = Prefs.port(this);
        String text = ServerState.running
                ? getString(R.string.notif_text, port, Math.max(ServerState.online, 0))
                : getString(R.string.notif_starting);
        return new Notification.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat)
                .setContentTitle(getString(R.string.notif_title))
                .setContentText(text)
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setShowWhen(false)
                .addAction(new Notification.Action.Builder(null, getString(R.string.stop), stop).build())
                .build();
    }

    @SuppressWarnings("deprecation")
    private void acquireLocks() {
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "vivant:server");
            wakeLock.setReferenceCounted(false);
        }
        if (!wakeLock.isHeld()) wakeLock.acquire();
        WifiManager wm = getApplicationContext().getSystemService(WifiManager.class);
        if (wm != null) {
            if (wifiLock == null) {
                wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "vivant:server");
                wifiLock.setReferenceCounted(false);
            }
            if (!wifiLock.isHeld()) wifiLock.acquire();
        }
    }

    private void releaseLocks() {
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
    }

    /** Splits "extra arguments" like a shell would for spaces and "double" / 'single' quotes. */
    static List<String> splitArgs(String s) {
        List<String> out = new ArrayList<>();
        StringBuilder cur = new StringBuilder();
        boolean in = false;
        char q = 0;
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (q != 0) {
                if (c == q) q = 0;
                else cur.append(c);
            } else if (c == '"' || c == '\'') {
                q = c;
                in = true;
            } else if (Character.isWhitespace(c)) {
                if (in) {
                    out.add(cur.toString());
                    cur.setLength(0);
                    in = false;
                }
            } else {
                cur.append(c);
                in = true;
            }
        }
        if (in) out.add(cur.toString());
        return out;
    }
}
