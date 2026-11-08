package io.github.freecode100year.vivant;

import android.content.Context;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Internet access for the phone's chat room through a bundled tunnel:
 * EasyTier (P2P mesh, no server of your own; visitors run EasyTier too) or
 * frpc (your own frps server; visitors just ssh to it). Both forward to the
 * server's local relay listener, so tunnelled users share generous limits
 * instead of all counting as one address (127.0.0.1).
 */
final class Tunnels {
    private Tunnels() {}

    static String binary(Context c, String kind) {
        String lib = Prefs.TUNNEL_EASYTIER.equals(kind) ? "libeasytier.so" : "libfrpc.so";
        return new File(c.getApplicationInfo().nativeLibraryDir, lib).getAbsolutePath();
    }

    static List<String> command(Context c, String kind) throws IOException {
        List<String> cmd = new ArrayList<>();
        cmd.add(binary(c, kind));
        if (Prefs.TUNNEL_EASYTIER.equals(kind)) {
            // no TUN device (that would need Android's VPN permission): other
            // nodes reach this phone's loopback ports at ET_IP through EasyTier's
            // own network stack
            add(cmd, "--no-tun", "-i", Prefs.ET_IP,
                    "--network-name", Prefs.etName(c),
                    "--network-secret", Prefs.etSecret(c),
                    "--hostname", "vivant",
                    "--machine-id", Prefs.etMachineId(c),
                    "--bind-device", "false",
                    "--file-log-level", "off",
                    "--console-log-level", "info");
            for (String p : Prefs.etPeer(c).split("[,\\s]+")) {
                if (!p.isEmpty()) add(cmd, "-p", p);
            }
        } else {
            File conf = new File(c.getFilesDir(), "frpc.toml");
            Files.write(conf.toPath(), frpConfig(c).getBytes(StandardCharsets.UTF_8));
            add(cmd, "-c", conf.getAbsolutePath());
        }
        return cmd;
    }

    static String frpConfig(Context c) {
        StringBuilder t = new StringBuilder();
        t.append("serverAddr = ").append(q(Prefs.frpServer(c))).append('\n');
        t.append("serverPort = ").append(Prefs.frpServerPort(c)).append('\n');
        if (!Prefs.frpToken(c).isEmpty()) t.append("auth.token = ").append(q(Prefs.frpToken(c))).append('\n');
        t.append("loginFailExit = false\n");
        t.append("log.to = \"console\"\nlog.level = \"info\"\nlog.disablePrintColor = true\n");
        t.append("transport.tls.enable = true\n\n");
        t.append("[[proxies]]\n");
        t.append("name = ").append(q(Prefs.frpProxyName(c))).append('\n');
        t.append("type = \"tcp\"\nlocalIP = \"127.0.0.1\"\n");
        t.append("localPort = ").append(Prefs.relayPort(c)).append('\n');
        t.append("remotePort = ").append(Prefs.frpRemotePort(c)).append('\n');
        return t.toString();
    }

    /** TOML basic string. */
    static String q(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (char ch : s.toCharArray()) {
            if (ch == '"' || ch == '\\') b.append('\\').append(ch);
            else if (ch < 0x20 || ch == 0x7f) b.append(String.format(Locale.ROOT, "\\u%04x", (int) ch));
            else b.append(ch);
        }
        return b.append('"').toString();
    }

    private static void add(List<String> l, String... a) {
        for (String s : a) l.add(s);
    }

    // ---- output: shorten for the log view and track the connection state --

    // 2026-11-08T08:34:14.514Z INFO  CORE::INSTANCE::CONNECTION: [uuid] new peer added peer_id=1
    private static final Pattern ET_LINE = Pattern.compile("^\\d{4}-\\d\\d-\\d\\dT\\S+\\s+(\\w+)\\s+\\S+?:\\s+(?:\\[[0-9a-f-]{36}\\]\\s+)?(.*)$");
    // 2026-11-08 03:39:39.016 [I] [client/control.go:174] [816ebbb0ac97ff72] [vivant-ab] start proxy success
    private static final Pattern FRP_LINE = Pattern.compile("^\\S+ \\S+ \\[(\\w)\\] \\[[^\\]]+\\] (?:\\[[0-9a-f]+\\] )?(.*)$");

    /** Returns the line to show, or null to hide it (EasyTier's startup config dump). Updates ServerState. */
    static String onLine(String kind, String raw) {
        String level, msg;
        if (Prefs.TUNNEL_EASYTIER.equals(kind)) {
            Matcher m = ET_LINE.matcher(raw);
            if (!m.matches()) return null;
            level = m.group(1);
            msg = m.group(2);
            if (msg.contains("new peer added") || msg.contains("new peer connection added")) {
                ServerState.tunnel = ServerState.TUNNEL_UP;
                ServerState.tunnelError = null;
            }
        } else {
            Matcher m = FRP_LINE.matcher(raw);
            if (!m.matches()) return null;
            level = m.group(1);
            msg = m.group(2);
            if (msg.contains("start proxy success")) {
                ServerState.tunnel = ServerState.TUNNEL_UP;
                ServerState.tunnelError = null;
            } else if (msg.contains("port already used") || msg.contains("port not allowed") || msg.contains("port unavailable")) {
                ServerState.tunnel = ServerState.TUNNEL_FAILED;
                ServerState.tunnelError = "frp: 外网端口不可用（被占用或服务器不允许）· remote port unavailable (in use or not allowed by the server)";
            } else if (msg.contains("token") || msg.contains("authorization failed")) {
                ServerState.tunnel = ServerState.TUNNEL_FAILED;
                ServerState.tunnelError = "frp: 令牌不对 · wrong token";
            } else if (msg.contains("login to the server failed") || msg.contains("connect to server error")) {
                ServerState.tunnel = ServerState.TUNNEL_CONNECTING;
                ServerState.tunnelError = "frp: 连不上服务器，正在重试 · can't reach the server, retrying";
            }
        }
        if (msg.length() > 180) msg = msg.substring(0, 180) + "…";
        String time = new SimpleDateFormat("HH:mm:ss", Locale.ROOT).format(new Date());
        return time + " [" + kind + "] " + level + " " + msg;
    }
}
