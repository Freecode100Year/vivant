package io.github.freecode100year.vivant;

import android.content.Context;

import java.io.File;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Internet access for the phone's chat room through the bundled EasyTier
 * (P2P mesh, no server of your own; visitors run EasyTier too). It forwards to
 * the server's local relay listener, so tunnelled users share generous limits
 * instead of all counting as one address (127.0.0.1).
 */
final class Tunnels {
    private Tunnels() {}

    static String binary(Context c) {
        return new File(c.getApplicationInfo().nativeLibraryDir, "libeasytier.so").getAbsolutePath();
    }

    static List<String> command(Context c) {
        List<String> cmd = new ArrayList<>();
        cmd.add(binary(c));
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
        return cmd;
    }

    private static void add(List<String> l, String... a) {
        for (String s : a) l.add(s);
    }

    // ---- output: shorten for the log view and track the connection state --

    // 2026-11-08T08:34:14.514Z INFO  CORE::INSTANCE::CONNECTION: [uuid] new peer added peer_id=1
    private static final Pattern ET_LINE = Pattern.compile("^\\d{4}-\\d\\d-\\d\\dT\\S+\\s+(\\w+)\\s+\\S+?:\\s+(?:\\[[0-9a-f-]{36}\\]\\s+)?(.*)$");

    /** Returns the line to show, or null to hide it (EasyTier's startup config dump). Updates ServerState. */
    static String onLine(String raw) {
        Matcher m = ET_LINE.matcher(raw);
        if (!m.matches()) return null;
        String level = m.group(1);
        String msg = m.group(2);
        if (msg.contains("new peer added") || msg.contains("new peer connection added")) {
            ServerState.tunnel = ServerState.TUNNEL_UP;
            ServerState.tunnelError = null;
        }
        if (msg.length() > 180) msg = msg.substring(0, 180) + "…";
        String time = new SimpleDateFormat("HH:mm:ss", Locale.ROOT).format(new Date());
        return time + " [easytier] " + level + " " + msg;
    }
}
