package io.github.freecode100year.vivant;

import android.content.Context;
import android.content.SharedPreferences;

/** User settings, kept in the app's private SharedPreferences. */
final class Prefs {
    static final int DEFAULT_PORT = 2222;
    static final int DEFAULT_MAX_CLIENTS = 100;

    private Prefs() {}

    private static SharedPreferences sp(Context c) {
        return c.getSharedPreferences("vivant", Context.MODE_PRIVATE);
    }

    static int port(Context c) { return sp(c).getInt("port", DEFAULT_PORT); }
    static int maxClients(Context c) { return sp(c).getInt("max_clients", DEFAULT_MAX_CLIENTS); }
    static String rooms(Context c) { return sp(c).getString("rooms", ""); }
    static String extraArgs(Context c) { return sp(c).getString("extra_args", ""); }
    static boolean autostart(Context c) { return sp(c).getBoolean("autostart", false); }
    /** True while the user wants the server up (survives process death, used after reboot/update). */
    static boolean running(Context c) { return sp(c).getBoolean("running", false); }

    static void save(Context c, int port, int maxClients, String rooms, String extra, boolean autostart) {
        sp(c).edit()
                .putInt("port", port)
                .putInt("max_clients", maxClients)
                .putString("rooms", rooms)
                .putString("extra_args", extra)
                .putBoolean("autostart", autostart)
                .apply();
    }

    static void setRunning(Context c, boolean on) { sp(c).edit().putBoolean("running", on).apply(); }
}
