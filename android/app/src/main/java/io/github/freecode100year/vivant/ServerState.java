package io.github.freecode100year.vivant;

import java.util.ArrayDeque;
import java.util.regex.Pattern;

/** State shared between the service and the screen (same process). */
final class ServerState {
    private static final int MAX_LINES = 400;
    private static final ArrayDeque<String> LINES = new ArrayDeque<>();
    // "time=2026-11-08T03:09:00.123-05:00 level=INFO msg=..." -> "03:09:00 INFO msg=..."
    private static final Pattern TIME = Pattern.compile("^time=\\S*T(\\d\\d:\\d\\d:\\d\\d)\\S*\\s+level=");

    static volatile boolean running;
    static volatile int port;
    static volatile int online = -1;
    static volatile int rooms = -1;
    static volatile String error;
    private static long version;

    private ServerState() {}

    static synchronized void log(String line) {
        LINES.addLast(TIME.matcher(line).replaceFirst("$1 "));
        while (LINES.size() > MAX_LINES) LINES.removeFirst();
        version++;
    }

    static synchronized long version() { return version; }

    static synchronized String logText() {
        return String.join("\n", LINES);
    }
}
