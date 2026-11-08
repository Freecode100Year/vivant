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

import org.json.JSONObject;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Foreground service that runs the VIVANT server (libvivant.so) and, if
 * internet access is on, a tunnel (libeasytier.so or libfrpc.so) as child
 * processes; keeps CPU and Wi-Fi awake, restarts them when they exit, and
 * polls /healthz for the online count.
 */
public class VivantService extends Service {
    static final String ACTION_START = "io.github.freecode100year.vivant.START";
    static final String ACTION_STOP = "io.github.freecode100year.vivant.STOP";
    private static final String CHANNEL = "vivant";
    private static final int NOTIF_ID = 1;
    private static final int MAX_QUICK_FAILS = 3;

    private final Handler main = new Handler(Looper.getMainLooper());
    private final Child server = new Child(main);
    private final Child tunnel = new Child(main);
    private TorOnion tor;
    private String tunnelKind = Prefs.TUNNEL_OFF;
    private boolean wanted;
    private int quickFails, tunnelFails;
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
        if (!wanted) {
            wanted = true;
            quickFails = 0;
            tunnelFails = 0;
            ServerState.error = null;
            acquireLocks();
            launchServer();
            tunnelKind = Prefs.tunnel(this);
            launchTunnel();
        }
        return START_STICKY;
    }

    @Override
    public void onDestroy() {
        stopAll();
        super.onDestroy();
    }

    // ---- chat server -----------------------------------------------------

    private void launchServer() {
        if (!wanted || server.alive()) return;
        int port = Prefs.port(this);
        List<String> cmd = new ArrayList<>();
        cmd.add(new File(getApplicationInfo().nativeLibraryDir, "libvivant.so").getAbsolutePath());
        boolean onion = Prefs.TUNNEL_TOR.equals(Prefs.tunnel(this));
        // Tor without LAN: nothing but the phone itself can reach the server directly
        add(cmd, "-addr", (onion && !Prefs.torAllowLan(this) ? "127.0.0.1:" : ":") + port,
                "-host-key", new File(getFilesDir(), "host_ed25519").getAbsolutePath(),
                "-health-addr", "127.0.0.1:" + Prefs.healthPort(this),
                "-relay-addr", "127.0.0.1:" + Prefs.relayPort(this),
                "-max-clients", String.valueOf(Prefs.maxClients(this)),
                // a phone usually has no use for the Tor exit list and it costs mobile data
                "-tor-exits=false");
        if (onion) add(cmd, "-onion-addr", "127.0.0.1:" + Prefs.onionLocalPort(this));
        String rooms = Prefs.rooms(this).trim();
        if (!rooms.isEmpty()) add(cmd, "-rooms", rooms);
        cmd.addAll(splitArgs(Prefs.extraArgs(this)));
        try {
            server.start(cmd, getFilesDir(), env(), new Child.Listener() {
                @Override
                public void onLine(String line) { ServerState.log(line); }

                @Override
                public void onExit(int code, long up) { serverExited(code, up); }
            });
        } catch (Exception e) {
            ServerState.log("无法启动 · cannot start: " + e.getMessage());
            fail(getString(R.string.err_start, e.getMessage()));
            return;
        }
        ServerState.running = true;
        ServerState.online = 0;
        startPoller(Prefs.healthPort(this));
        updateNotification();
    }

    private void serverExited(int code, long uptimeMs) {
        ServerState.running = false;
        ServerState.online = -1;
        if (!wanted) return;
        ServerState.log("服务器退出 · server exited, code " + code);
        quickFails = uptimeMs < 10_000 ? quickFails + 1 : 0;
        if (quickFails >= MAX_QUICK_FAILS) {
            fail(getString(R.string.err_crash, code));
            return;
        }
        updateNotification();
        main.postDelayed(this::launchServer, 2000);
    }

    // ---- tunnel ----------------------------------------------------------

    private void launchTunnel() {
        if (!wanted || tunnel.alive() || Prefs.TUNNEL_OFF.equals(tunnelKind)) {
            if (Prefs.TUNNEL_OFF.equals(tunnelKind)) ServerState.tunnel = ServerState.TUNNEL_OFF;
            return;
        }
        if (Prefs.TUNNEL_TOR.equals(tunnelKind)) {
            if (tor != null) return;
            tor = new TorOnion(this);
            try {
                tor.start(Prefs.onionLocalPort(this));
            } catch (Exception e) {
                tor.stop();
                tor = null;
                ServerState.tunnel = ServerState.TUNNEL_FAILED;
                ServerState.tunnelError = getString(R.string.err_tunnel_start, e.getMessage());
            }
            updateNotification();
            return;
        }
        final String kind = tunnelKind;
        ServerState.tunnel = ServerState.TUNNEL_CONNECTING;
        try {
            tunnel.start(Tunnels.command(this, kind), tunnelDir(), env(), new Child.Listener() {
                @Override
                public void onLine(String line) {
                    String shown = Tunnels.onLine(kind, line);
                    if (shown != null) ServerState.log(shown);
                    if (ServerState.tunnel == ServerState.TUNNEL_UP) tunnelFails = 0;
                }

                @Override
                public void onExit(int code, long up) { tunnelExited(code); }
            });
        } catch (Exception e) {
            ServerState.tunnel = ServerState.TUNNEL_FAILED;
            ServerState.tunnelError = getString(R.string.err_tunnel_start, e.getMessage());
        }
        updateNotification();
    }

    private void tunnelExited(int code) {
        if (!wanted) return;
        ServerState.log("[" + tunnelKind + "] 退出 · exited, code " + code + "，稍后重试 · retrying");
        if (ServerState.tunnel != ServerState.TUNNEL_FAILED) ServerState.tunnel = ServerState.TUNNEL_CONNECTING;
        // networks come and go on a phone: keep retrying, backing off to a minute
        long delay = Math.min(60_000L, 2000L << Math.min(tunnelFails++, 5));
        updateNotification();
        main.postDelayed(this::launchTunnel, delay);
    }

    private File tunnelDir() {
        File d = new File(getFilesDir(), "tunnel");
        //noinspection ResultOfMethodCallIgnored
        d.mkdirs();
        return d;
    }

    private Map<String, String> env() {
        Map<String, String> e = new HashMap<>();
        e.put("HOME", getFilesDir().getAbsolutePath());
        e.put("TMPDIR", getCacheDir().getAbsolutePath());
        return e;
    }

    // ---- stop ------------------------------------------------------------

    /** Gives up: leaves the error for the screen and stops the service. */
    private void fail(String why) {
        ServerState.error = why;
        Prefs.setRunning(this, false);
        shutdown();
    }

    private void shutdown() {
        stopAll();
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    private void stopAll() {
        boolean was = wanted || server.alive() || tunnel.alive();
        wanted = false;
        main.removeCallbacksAndMessages(null);
        if (poller != null) poller.interrupt();
        poller = null;
        tunnel.stop();
        if (tor != null) {
            tor.stop();
            tor = null;
        }
        server.stop();
        releaseLocks();
        ServerState.running = false;
        ServerState.online = -1;
        ServerState.tunnel = ServerState.TUNNEL_OFF;
        ServerState.tunnelError = null;
        if (was) ServerState.log("已停止 · stopped");
    }

    // ---- /healthz --------------------------------------------------------

    private void startPoller(int healthPort) {
        if (poller != null) poller.interrupt();
        poller = new Thread(() -> {
            int last = -2, lastTunnel = -2;
            while (!Thread.currentThread().isInterrupted()) {
                try {
                    Thread.sleep(3000);
                } catch (InterruptedException e) {
                    return;
                }
                JSONObject st = health(healthPort);
                if (st != null) ServerState.online = st.optInt("online", 0);
                if (ServerState.online != last || ServerState.tunnel != lastTunnel) {
                    last = ServerState.online;
                    lastTunnel = ServerState.tunnel;
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

    // ---- foreground / locks ------------------------------------------------

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
        String text = ServerState.running
                ? getString(R.string.notif_text, Math.max(ServerState.online, 0))
                : getString(R.string.notif_starting);
        if (ServerState.tunnel == ServerState.TUNNEL_UP) text += " · " + getString(R.string.notif_internet);
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

    private static void add(List<String> l, String... a) {
        for (String s : a) l.add(s);
    }

    /** Splits "extra flags" like a shell would for spaces and "double" / 'single' quotes. */
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
