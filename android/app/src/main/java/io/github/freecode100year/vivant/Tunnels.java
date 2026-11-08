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
 * Internet access through the user's own frps server (bundled frpc). It
 * forwards to the server's local relay listener, so every visitor reaches the
 * chat server as 127.0.0.1: frpc never learns a visitor's address (no Proxy
 * Protocol, ever), and visitors never learn the phone's. frps itself does see
 * visitors' IPs — the app says so; Tor is the mode where nobody does.
 *
 * (EasyTier was removed: its nodes share their public and interface IPs with
 * every other node in the network, so visitors could learn the phone's IP and
 * each other's.)
 */
final class Tunnels {
    private Tunnels() {}

    static List<String> command(Context c) throws IOException {
        List<String> cmd = new ArrayList<>();
        cmd.add(new File(c.getApplicationInfo().nativeLibraryDir, "libfrpc.so").getAbsolutePath());
        File conf = new File(c.getFilesDir(), "frpc.toml");
        Files.write(conf.toPath(), frpConfig(c).getBytes(StandardCharsets.UTF_8));
        cmd.add("-c");
        cmd.add(conf.getAbsolutePath());
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
        // deliberately no transport.proxyProtocolVersion: it would hand visitors' IPs to the phone
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

    // 2026-11-08 03:39:39.016 [I] [client/control.go:174] [816ebbb0ac97ff72] [vivant-ab] start proxy success
    private static final Pattern FRP_LINE = Pattern.compile("^\\S+ \\S+ \\[(\\w)\\] \\[[^\\]]+\\] (?:\\[[0-9a-f]+\\] )?(.*)$");

    /** Returns the line to show, or null to hide it. Updates ServerState. */
    static String onLine(String raw) {
        Matcher m = FRP_LINE.matcher(raw);
        if (!m.matches()) return null;
        String level = m.group(1), msg = m.group(2);
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
        if (msg.length() > 180) msg = msg.substring(0, 180) + "…";
        return new SimpleDateFormat("HH:mm:ss", Locale.ROOT).format(new Date()) + " [frp] " + level + " " + msg;
    }
}
