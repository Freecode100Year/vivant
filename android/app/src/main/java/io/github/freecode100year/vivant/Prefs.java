package io.github.freecode100year.vivant;

import android.content.Context;
import android.content.SharedPreferences;

import java.security.SecureRandom;
import java.util.UUID;

/** User settings, kept in the app's private SharedPreferences. */
final class Prefs {
    static final int DEFAULT_PORT = 2222;
    static final int DEFAULT_MAX_CLIENTS = 100;
    static final String DEFAULT_ET_PEER = "tcp://public.easytier.top:11010";

    static final String TUNNEL_OFF = "off";
    static final String TUNNEL_EASYTIER = "easytier";

    /** This phone's address inside the EasyTier network. */
    static final String ET_IP = "10.144.144.1";

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
    static int maxClients(Context c) { return sp(c).getInt("max_clients", DEFAULT_MAX_CLIENTS); }
    static String rooms(Context c) { return sp(c).getString("rooms", ""); }
    static String extraArgs(Context c) { return sp(c).getString("extra_args", ""); }
    static boolean autostart(Context c) { return sp(c).getBoolean("autostart", false); }
    /** True while the user wants the server up (survives process death, used after reboot/update). */
    static boolean running(Context c) { return sp(c).getBoolean("running", false); }
    static void setRunning(Context c, boolean on) { sp(c).edit().putBoolean("running", on).apply(); }

    // internet access
    /** off or easytier. FRP was removed; a phone that had chosen it starts with internet access off. */
    static String tunnel(Context c) {
        return TUNNEL_EASYTIER.equals(sp(c).getString("tunnel", TUNNEL_OFF)) ? TUNNEL_EASYTIER : TUNNEL_OFF;
    }
    static String etName(Context c) { return generated(c, "et_name", "vivant-", 10); }
    static String etSecret(Context c) { return generated(c, "et_secret", "", 16); }
    static String etPeer(Context c) {
        String p = sp(c).getString("et_peer", "").trim();
        return p.isEmpty() ? DEFAULT_ET_PEER : p;
    }
    static String etMachineId(Context c) {
        String id = sp(c).getString("et_machine_id", null);
        if (id == null) {
            id = UUID.randomUUID().toString();
            sp(c).edit().putString("et_machine_id", id).apply();
        }
        return id;
    }

    /** A new random EasyTier network name and password. */
    static void regenerateEasyTier(Context c) {
        sp(c).edit().putString("et_name", "vivant-" + random(10)).putString("et_secret", random(16)).apply();
    }

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
