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
 * Internet access through a bundled tunnel client other than Tor. Both
 * forward to the chat server's local relay listener, so every visitor reaches
 * it as 127.0.0.1 and the server never learns a visitor's address.
 *
 * EasyTier: relay only, through the user's own relay node. The phone's build
 * carries easytier-no-ip.patch (it never reports its addresses to other
 * nodes), and P2P, hole punching, STUN, IPv6 and listeners are all off, so
 * the phone and visitors never learn each other's IP. Visitors run stock
 * EasyTier with the matching flags (--stun-servers with no value, P2P off).
 *
 * frpc: the user's own frps; frps sees visitors' IPs, frpc never does (no
 * Proxy Protocol, ever).
 *
 * Nothing these clients print is kept with an address in it: every IP-looking
 * string is masked before a line reaches the in-memory log.
 */
final class Tunnels {
    private Tunnels() {}

    static List<String> command(Context c, String kind) throws IOException {
        List<String> cmd = new ArrayList<>();
        if (Prefs.TUNNEL_EASYTIER.equals(kind)) {
            cmd.add(new File(c.getApplicationInfo().nativeLibraryDir, "libeasytier.so").getAbsolutePath());
            add(cmd, "--no-tun", "-i", Prefs.ET_IP,
                    "--network-name", Prefs.etName(c),
                    "--network-secret", Prefs.etSecret(c),
                    "--hostname", "vivant",
                    "--machine-id", Prefs.etMachineId(c),
                    // relay only: no listener, no P2P, no hole punching
                    "--no-listener",
                    "--disable-p2p",
                    "--disable-udp-hole-punching",
                    "--disable-tcp-hole-punching",
                    // no STUN (it would learn, and share, the public IP) and no IPv6
                    "--stun-servers",
                    "--disable-ipv6",
                    "--bind-device", "false",
                    "--file-log-level", "off",
                    "--console-log-level", "info");
            for (String p : Prefs.etPeer(c).split("[,\\s]+")) {
                if (!p.isEmpty()) add(cmd, "-p", p);
            }
        } else {
            cmd.add(new File(c.getApplicationInfo().nativeLibraryDir, "libfrpc.so").getAbsolutePath());
            File conf = new File(c.getFilesDir(), "frpc.toml");
            Files.write(conf.toPath(), frpConfig(c).getBytes(StandardCharsets.UTF_8));
            add(cmd, "-c", conf.getAbsolutePath());
        }
        return cmd;
    }

    /** What a visitor runs (stock EasyTier) to reach this phone, before `ssh -p 2222 guest@127.0.0.1`. */
    static String easytierVisitorCommand(String name, String secret, String peer, int relayPort) {
        return "easytier-core --no-tun -d --no-listener --disable-p2p --stun-servers"
                + " --network-name " + name
                + " --network-secret " + secret
                + " -p " + peer
                + " --port-forward tcp://127.0.0.1:2222/" + Prefs.ET_IP + ":" + relayPort;
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

    private static void add(List<String> l, String... a) {
        for (String s : a) l.add(s);
    }

    // ---- output ------------------------------------------------------------

    // 2026-11-08T08:34:14.514Z INFO  CORE::INSTANCE::CONNECTION: [uuid] new peer added peer_id=1
    private static final Pattern ET_LINE = Pattern.compile("^\\d{4}-\\d\\d-\\d\\dT\\S+\\s+(\\w+)\\s+\\S+?:\\s+(?:\\[[0-9a-f-]{36}\\]\\s+)?(.*)$");
    // 2026-11-08 03:39:39.016 [I] [client/control.go:174] [816ebbb0ac97ff72] [vivant-ab] start proxy success
    private static final Pattern FRP_LINE = Pattern.compile("^\\S+ \\S+ \\[(\\w)\\] \\[[^\\]]+\\] (?:\\[[0-9a-f]+\\] )?(.*)$");
    private static final Pattern IPV4 = Pattern.compile("(?<![\\d.])(?:\\d{1,3}\\.){3}\\d{1,3}(?![\\d.])");
    private static final Pattern HEXCOLON = Pattern.compile("(?<![0-9A-Za-z:])[0-9A-Fa-f:]{3,}(?![0-9A-Za-z:])");

    /** Replaces every IP address in a line, so no address is ever kept or shown. */
    static String maskAddresses(String s) {
        StringBuilder out = new StringBuilder();
        Matcher m = IPV4.matcher(s);
        int last = 0;
        while (m.find()) {
            String ip = m.group();
            // loopback and this phone's virtual EasyTier address say nothing about anyone
            boolean keep = ip.startsWith("127.") || ip.startsWith(Prefs.ET_IP_PREFIX);
            out.append(s, last, m.start()).append(keep ? ip : "•.•.•.•");
            last = m.end();
        }
        s = out.append(s.substring(last)).toString();
        out.setLength(0);
        m = HEXCOLON.matcher(s);
        last = 0;
        while (m.find()) {
            String t = m.group();
            int colons = t.length() - t.replace(":", "").length();
            boolean v6 = (colons >= 3 || t.contains("::")) && t.chars().anyMatch(Character::isDigit) && !t.equals("::1");
            out.append(s, last, m.start()).append(v6 ? "•:•:•" : t);
            last = m.end();
        }
        return out.append(s.substring(last)).toString();
    }

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
        msg = maskAddresses(msg);
        if (msg.length() > 180) msg = msg.substring(0, 180) + "…";
        return new SimpleDateFormat("HH:mm:ss", Locale.ROOT).format(new Date()) + " [" + kind + "] " + level + " " + msg;
    }
}
