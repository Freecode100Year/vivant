package io.github.freecode100year.vivant;

import android.content.Context;
import android.content.SharedPreferences;

import java.security.SecureRandom;

/** User settings, kept in the app's private SharedPreferences. */
final class Prefs {
    static final int DEFAULT_PORT = 2222;
    static final int DEFAULT_MAX_CLIENTS = 100;

    static final String TUNNEL_OFF = "off";
    static final String TUNNEL_FRP = "frp";
    static final String TUNNEL_TOR = "tor";

    private Prefs() {}

    static SharedPreferences sp(Context c) {
        return c.getSharedPreferences("vivant", Context.MODE_PRIVATE);
    }

    // chat server
    static int port(Context c) { return sp(c).getInt("port", DEFAULT_PORT); }
    /** Local-only status endpoint. */
    static int healthPort(Context c) { return port(c) + 1; }
    /** Local-only listener the tunnels forward to; everyone through it shares generous limits. */
    static int relayPort(Context c) { return port(c) + 2; }
    /** Local-only listener the onion service forwards to. */
    static int onionLocalPort(Context c) { return port(c) + 3; }
    static int maxClients(Context c) { return sp(c).getInt("max_clients", DEFAULT_MAX_CLIENTS); }
    static String rooms(Context c) { return sp(c).getString("rooms", ""); }
    static String extraArgs(Context c) { return sp(c).getString("extra_args", ""); }
    static boolean autostart(Context c) { return sp(c).getBoolean("autostart", false); }
    /** True while the user wants the server up (survives process death, used after reboot/update). */
    static boolean running(Context c) { return sp(c).getBoolean("running", false); }
    static void setRunning(Context c, boolean on) { sp(c).edit().putBoolean("running", on).apply(); }

    // internet access
    static String tunnel(Context c) {
        String t = sp(c).getString("tunnel", TUNNEL_OFF);
        return TUNNEL_TOR.equals(t) || TUNNEL_FRP.equals(t) ? t : TUNNEL_OFF; // EasyTier was removed
    }
    static String torBridges(Context c) { return sp(c).getString("tor_bridges", TorOnion.BRIDGES_NONE); }
    static String torCustomBridges(Context c) { return sp(c).getString("tor_custom_bridges", ""); }
    /**
     * With internet access on, the chat server listens on 127.0.0.1 only unless
     * the user also wants direct LAN / hotspot connections (which reveal local
     * addresses to the phone and to each other).
     */
    static boolean allowLan(Context c) { return sp(c).getBoolean("allow_lan", false); }
    static boolean lanOpen(Context c) { return TUNNEL_OFF.equals(tunnel(c)) || allowLan(c); }
    static String frpServer(Context c) { return sp(c).getString("frp_server", "").trim(); }
    static int frpServerPort(Context c) { return sp(c).getInt("frp_server_port", 7000); }
    static String frpToken(Context c) { return sp(c).getString("frp_token", ""); }
    static int frpRemotePort(Context c) { return sp(c).getInt("frp_remote_port", DEFAULT_PORT); }
    static String frpProxyName(Context c) { return generated(c, "frp_name", "vivant-", 6); }

    private static String generated(Context c, String key, String prefix, int n) {
        String v = sp(c).getString(key, null);
        if (v == null || v.isEmpty()) {
            v = prefix + random(n);
            sp(c).edit().putString(key, v).apply();
        }
        return v;
    }

    private static final String ALPHABET = "abcdefghijkmnpqrstuvwxyz23456789"; // no look-alikes
    private static final SecureRandom RNG = new SecureRandom();

    static String random(int n) {
        StringBuilder sb = new StringBuilder(n);
        for (int i = 0; i < n; i++) sb.append(ALPHABET.charAt(RNG.nextInt(ALPHABET.length())));
        return sb.toString();
    }
}
