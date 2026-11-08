package io.github.freecode100year.vivant;

import android.os.Handler;
import android.os.SystemClock;

import java.io.BufferedReader;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;

/** One bundled program run as a child process, with its output fed line by line to a listener. */
final class Child {
    interface Listener {
        /** Called on the output thread. */
        void onLine(String line);

        /** Called on the main thread, only for a process that wasn't stopped with {@link #stop()}. */
        void onExit(int code, long uptimeMs);
    }

    private final Handler main;
    private Process proc;

    Child(Handler main) { this.main = main; }

    boolean alive() { return proc != null; }

    void start(List<String> cmd, File dir, Map<String, String> env, Listener l) throws IOException {
        ProcessBuilder pb = new ProcessBuilder(cmd).redirectErrorStream(true).directory(dir);
        pb.environment().putAll(env);
        final Process p = pb.start();
        final long started = SystemClock.elapsedRealtime();
        proc = p;
        Thread reader = new Thread(() -> {
            try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream(), StandardCharsets.UTF_8))) {
                String line;
                while ((line = r.readLine()) != null) l.onLine(line);
            } catch (IOException ignored) {
            }
            int code;
            try {
                code = p.waitFor();
            } catch (InterruptedException e) {
                code = -1;
            }
            final int c = code;
            main.post(() -> {
                if (proc != p) return; // stopped on purpose
                proc = null;
                l.onExit(c, SystemClock.elapsedRealtime() - started);
            });
        }, "child-output");
        reader.setDaemon(true);
        reader.start();
    }

    /** SIGTERM (clean shutdown), then SIGKILL if it's still there after 3 s. */
    void stop() {
        final Process p = proc;
        proc = null;
        if (p == null) return;
        p.destroy();
        Thread t = new Thread(() -> {
            try {
                if (!p.waitFor(3, TimeUnit.SECONDS)) p.destroyForcibly();
            } catch (InterruptedException e) {
                p.destroyForcibly();
            }
        });
        t.setDaemon(true);
        t.start();
    }
}
