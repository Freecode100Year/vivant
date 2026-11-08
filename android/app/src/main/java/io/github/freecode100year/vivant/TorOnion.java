package io.github.freecode100year.vivant;

import android.content.BroadcastReceiver;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.ServiceConnection;
import android.os.IBinder;

import androidx.localbroadcastmanager.content.LocalBroadcastManager;

import net.freehaven.tor.control.RawEventListener;
import net.freehaven.tor.control.TorControlConnection;

import org.torproject.jni.TorService;

import java.io.BufferedReader;
import java.io.File;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import IPtProxy.Controller;
import IPtProxy.IPtProxy;
import IPtProxy.OnTransportEvents;

/**
 * Publishes the chat room as a Tor onion service from the phone itself.
 * Nobody learns anybody's IP address: visitors reach the phone through Tor,
 * the phone reaches Tor through Tor, and the chat server only ever sees
 * 127.0.0.1. Optional Snowflake or custom (obfs4 / webtunnel / …) bridges get
 * the phone onto Tor from censored networks.
 *
 * Tor runs in this app's process (tor-android, the build Orbot uses); the
 * bridges come from IPtProxy (lyrebird + snowflake), also as used by Orbot.
 */
final class TorOnion {
    static final String BRIDGES_NONE = "none", BRIDGES_SNOWFLAKE = "snowflake", BRIDGES_CUSTOM = "custom";
    /** Virtual port of the onion service: visitors type plain `ssh guest@x.onion`. */
    static final int ONION_PORT = 22;

    private static final Pattern BOOTSTRAP = Pattern.compile("Bootstrapped (\\d+)%");

    private final Context ctx;
    private ServiceConnection conn;
    private BroadcastReceiver receiver;
    private Controller pt;
    private final Set<String> transports = new LinkedHashSet<>();
    private volatile boolean stopped;

    TorOnion(Context ctx) { this.ctx = ctx.getApplicationContext(); }

    static File serviceDir(Context c) { return new File(c.getFilesDir(), "onion"); }

    /** The .onion address, once Tor has created the service keys (kept across restarts). */
    static String address(Context c) {
        try {
            String h = new String(Files.readAllBytes(new File(serviceDir(c), "hostname").toPath()), StandardCharsets.US_ASCII).trim();
            return h.endsWith(".onion") ? h : null;
        } catch (Exception e) {
            return null;
        }
    }

    void start(int localPort) throws Exception {
        stopped = false;
        ServerState.tunnel = ServerState.TUNNEL_CONNECTING;
        ServerState.torProgress = 0;

        // tor refuses a service directory others can read
        File hs = serviceDir(ctx);
        //noinspection ResultOfMethodCallIgnored
        hs.mkdirs();
        hs.setReadable(false, false);
        hs.setWritable(false, false);
        hs.setExecutable(false, false);
        hs.setReadable(true, true);
        hs.setWritable(true, true);
        hs.setExecutable(true, true);

        List<String> rc = new ArrayList<>(Arrays.asList(
                // no SOCKS / HTTP proxy: other apps on the phone can't use (or probe) this Tor
                "SOCKSPort 0",
                "HTTPTunnelPort 0",
                "HiddenServiceDir " + hs.getAbsolutePath(),
                "HiddenServicePort " + ONION_PORT + " 127.0.0.1:" + localPort,
                "HiddenServiceMaxStreams 600",
                "HiddenServiceMaxStreamsCloseCircuit 1",
                // scrub addresses from anything tor logs, and log to stdout
                // (nothing on Android) instead of the system log (logcat)
                "SafeLogging 1",
                "Log notice stdout"));
        rc.addAll(bridgeConfig());
        Files.write(TorService.getTorrc(ctx).toPath(), (String.join("\n", rc) + "\n").getBytes(StandardCharsets.UTF_8));

        receiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context c, Intent i) {
                if (TorService.ACTION_ERROR.equals(i.getAction())) {
                    String t = i.getStringExtra(Intent.EXTRA_TEXT);
                    ServerState.log(stamp() + " [tor] ERROR " + t);
                    ServerState.tunnel = ServerState.TUNNEL_FAILED;
                    ServerState.tunnelError = "Tor: " + t;
                } else if (TorService.STATUS_ON.equals(i.getStringExtra(TorService.EXTRA_STATUS))) {
                    // connected to Tor; the service is reachable once its descriptor is uploaded
                    if (ServerState.torProgress < 100) ServerState.torProgress = 100;
                }
            }
        };
        IntentFilter f = new IntentFilter(TorService.ACTION_STATUS);
        f.addAction(TorService.ACTION_ERROR);
        LocalBroadcastManager.getInstance(ctx).registerReceiver(receiver, f);

        conn = new ServiceConnection() {
            @Override
            public void onServiceConnected(ComponentName name, IBinder binder) {
                TorService tor = ((TorService.LocalBinder) binder).getService();
                Thread t = new Thread(() -> listen(tor), "tor-events");
                t.setDaemon(true);
                t.start();
            }

            @Override
            public void onServiceDisconnected(ComponentName name) {
                if (!stopped) {
                    ServerState.tunnel = ServerState.TUNNEL_FAILED;
                    ServerState.tunnelError = "Tor 意外停止 · Tor stopped unexpectedly";
                }
            }
        };
        if (!ctx.bindService(new Intent(ctx, TorService.class), conn, Context.BIND_AUTO_CREATE)) {
            throw new IllegalStateException("bindService(TorService) failed");
        }
        ServerState.log(stamp() + " [tor] starting" + (transports.isEmpty() ? "" : " via " + String.join(", ", transports)));
    }

    /** Follows Tor's log and onion-service events through the control connection. */
    private void listen(TorService tor) {
        TorControlConnection c = null;
        for (int i = 0; i < 240 && !stopped && (c = tor.getTorControlConnection()) == null; i++) {
            try {
                Thread.sleep(500);
            } catch (InterruptedException e) {
                return;
            }
        }
        if (c == null || stopped) return;
        try {
            c.addRawEventListener(new RawEventListener() {
                @Override
                public void onEvent(String keyword, String data) {
                    onTorEvent(keyword, data);
                }
            });
            // keep STATUS_CLIENT: TorService needs it to report "ON"
            c.setEvents(Arrays.asList("STATUS_CLIENT", "NOTICE", "WARN", "ERR", "HS_DESC"));
        } catch (Exception e) {
            ServerState.log(stamp() + " [tor] control: " + e.getMessage());
        }
    }

    private void onTorEvent(String keyword, String data) {
        if (data == null) data = "";
        if ("HS_DESC".equals(keyword)) {
            // "UPLOADED <address> ..." — the onion service is now reachable
            if (data.startsWith("UPLOADED ")) {
                if (ServerState.tunnel != ServerState.TUNNEL_UP) ServerState.log(stamp() + " [tor] onion service published");
                ServerState.tunnel = ServerState.TUNNEL_UP;
                ServerState.tunnelError = null;
            }
            return;
        }
        if ("STATUS_CLIENT".equals(keyword)) return;
        Matcher m = BOOTSTRAP.matcher(data);
        if (m.find()) ServerState.torProgress = Integer.parseInt(m.group(1));
        String msg = data.length() > 180 ? data.substring(0, 180) + "…" : data;
        ServerState.log(stamp() + " [tor] " + keyword + " " + msg);
    }

    /** UseBridges / ClientTransportPlugin / Bridge lines; starts the needed pluggable transports. */
    private List<String> bridgeConfig() throws Exception {
        List<String> lines = bridgeLines();
        List<String> rc = new ArrayList<>();
        if (lines.isEmpty()) return rc;
        transports.clear();
        for (String l : lines) {
            String t = transportOf(l);
            if (t != null) transports.add(t);
        }
        if (!transports.isEmpty()) {
            File state = new File(ctx.getFilesDir(), "pt_state");
            //noinspection ResultOfMethodCallIgnored
            state.mkdirs();
            pt = new Controller(state.getAbsolutePath(), true, false, "WARN", new OnTransportEvents() {
                @Override
                public void connected(String name) {}

                @Override
                public void error(String name, Exception e) {
                    if (e != null) ServerState.log(stamp() + " [" + name + "] " + e.getMessage());
                }

                @Override
                public void stopped(String name, Exception e) {}
            });
            for (String t : transports) {
                if (IPtProxy.Snowflake.equals(t)) configureSnowflake(lines);
                pt.start(t, "");
                rc.add("ClientTransportPlugin " + t + " socks5 127.0.0.1:" + pt.port(t));
            }
        }
        rc.add("UseBridges 1");
        for (String l : lines) rc.add("Bridge " + l);
        return rc;
    }

    /** The snowflake client takes broker, fronts and ICE servers from the controller, not the bridge line. */
    private void configureSnowflake(List<String> lines) {
        for (String l : lines) {
            if (!IPtProxy.Snowflake.equals(transportOf(l))) continue;
            for (String part : l.split("\\s+")) {
                int eq = part.indexOf('=');
                if (eq < 0) continue;
                String k = part.substring(0, eq), v = part.substring(eq + 1);
                if (k.equals("url")) pt.setSnowflakeBrokerUrl(v);
                else if (k.equals("fronts") || k.equals("front")) pt.setSnowflakeFrontDomains(v);
                else if (k.equals("ice")) pt.setSnowflakeIceServers(v);
            }
            return;
        }
    }

    private List<String> bridgeLines() {
        String mode = Prefs.torBridges(ctx);
        List<String> out = new ArrayList<>();
        if (BRIDGES_SNOWFLAKE.equals(mode)) {
            try (BufferedReader r = new BufferedReader(new InputStreamReader(ctx.getAssets().open("snowflake-bridges.txt"), StandardCharsets.UTF_8))) {
                String l;
                while ((l = r.readLine()) != null) {
                    l = l.trim();
                    if (!l.isEmpty() && !l.startsWith("#")) out.add(l);
                }
            } catch (Exception e) {
                ServerState.log(stamp() + " [tor] no built-in bridges: " + e.getMessage());
            }
        } else if (BRIDGES_CUSTOM.equals(mode)) {
            out.addAll(parseBridges(Prefs.torCustomBridges(ctx)));
        }
        return out;
    }

    /** One bridge per line, as given by bridges.torproject.org or Tor Browser ("Bridge " prefix optional). */
    static List<String> parseBridges(String text) {
        List<String> out = new ArrayList<>();
        for (String l : text.split("\\r?\\n")) {
            l = l.trim();
            if (l.regionMatches(true, 0, "Bridge ", 0, 7)) l = l.substring(7).trim();
            if (!l.isEmpty() && !l.startsWith("#")) out.add(l);
        }
        return out;
    }

    /** The pluggable transport a bridge line needs, or null for a plain (vanilla) bridge. */
    static String transportOf(String line) {
        String first = line.split("\\s+")[0];
        switch (first) {
            case "obfs4": return IPtProxy.Obfs4;
            case "webtunnel": return IPtProxy.Webtunnel;
            case "snowflake": return IPtProxy.Snowflake;
            case "meek_lite": case "meek": return IPtProxy.MeekLite;
            default: return null;
        }
    }

    void stop() {
        stopped = true;
        if (receiver != null) {
            LocalBroadcastManager.getInstance(ctx).unregisterReceiver(receiver);
            receiver = null;
        }
        if (conn != null) {
            try {
                ctx.unbindService(conn); // unbinding stops tor
            } catch (IllegalArgumentException ignored) {
            }
            conn = null;
        }
        if (pt != null) {
            for (String t : transports) {
                try {
                    pt.stop(t);
                } catch (Exception ignored) {
                }
            }
            pt = null;
        }
        transports.clear();
    }

    private static String stamp() {
        return new java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.ROOT).format(new java.util.Date());
    }
}
