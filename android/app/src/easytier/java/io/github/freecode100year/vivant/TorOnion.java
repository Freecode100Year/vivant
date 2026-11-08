package io.github.freecode100year.vivant;

import android.content.Context;

import java.io.File;
import java.util.ArrayList;
import java.util.List;

/** EasyTier-only edition: no Tor inside. Same API as the full edition's TorOnion, doing nothing. */
final class TorOnion {
    static final String BRIDGES_NONE = "none", BRIDGES_SNOWFLAKE = "snowflake", BRIDGES_CUSTOM = "custom";

    TorOnion(Context ctx) {}

    static File serviceDir(Context c) { return new File(c.getFilesDir(), "onion"); }

    static String address(Context c) { return null; }

    static List<String> parseBridges(String text) { return new ArrayList<>(); }

    void start(int localPort) { throw new UnsupportedOperationException("this edition has no Tor"); }

    void stop() {}
}
