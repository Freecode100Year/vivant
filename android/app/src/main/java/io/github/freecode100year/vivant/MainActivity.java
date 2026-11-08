package io.github.freecode100year.vivant;

import android.Manifest;
import android.app.Activity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.ConnectivityManager;
import android.net.LinkAddress;
import android.net.LinkProperties;
import android.net.Network;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.provider.Settings;
import android.view.View;
import android.widget.Button;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.net.Inet4Address;
import java.net.Inet6Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

public class MainActivity extends Activity {
    private final Handler ui = new Handler(Looper.getMainLooper());
    private TextView status, online, connect, fingerprint, log, error;
    private Button toggle, battery;
    private EditText port, maxClients, rooms, extra;
    private CheckBox autostart;
    private ScrollView logScroll;
    private long shownLog = -1;
    private int tick;

    private final Runnable refresh = new Runnable() {
        @Override
        public void run() {
            render();
            ui.postDelayed(this, 1000);
        }
    };

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        setContentView(R.layout.activity_main);
        status = findViewById(R.id.status);
        online = findViewById(R.id.online);
        connect = findViewById(R.id.connect);
        fingerprint = findViewById(R.id.fingerprint);
        log = findViewById(R.id.log);
        logScroll = findViewById(R.id.log_scroll);
        error = findViewById(R.id.error);
        toggle = findViewById(R.id.toggle);
        battery = findViewById(R.id.battery);
        port = findViewById(R.id.port);
        maxClients = findViewById(R.id.max_clients);
        rooms = findViewById(R.id.rooms);
        extra = findViewById(R.id.extra);
        autostart = findViewById(R.id.autostart);
        try {
            ((TextView) findViewById(R.id.version)).setText("v" + getPackageManager().getPackageInfo(getPackageName(), 0).versionName);
        } catch (PackageManager.NameNotFoundException ignored) {
        }

        port.setText(String.valueOf(Prefs.port(this)));
        maxClients.setText(String.valueOf(Prefs.maxClients(this)));
        rooms.setText(Prefs.rooms(this));
        extra.setText(Prefs.extraArgs(this));
        autostart.setChecked(Prefs.autostart(this));
        autostart.setOnCheckedChangeListener((v, on) -> saveSettings(false));

        toggle.setOnClickListener(v -> {
            if (ServerState.running || Prefs.running(this)) stopServer();
            else startServer();
        });
        findViewById(R.id.copy).setOnClickListener(v -> {
            ClipboardManager cm = getSystemService(ClipboardManager.class);
            cm.setPrimaryClip(ClipData.newPlainText("ssh", connect.getText()));
            Toast.makeText(this, R.string.copied, Toast.LENGTH_SHORT).show();
        });
        battery.setOnClickListener(v -> {
            try {
                startActivity(new Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:" + getPackageName())));
            } catch (RuntimeException e) {
                startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS));
            }
        });
    }

    @Override
    protected void onResume() {
        super.onResume();
        shownLog = -1;
        tick = 0;
        ui.post(refresh);
    }

    @Override
    protected void onPause() {
        ui.removeCallbacks(refresh);
        super.onPause();
    }

    private void startServer() {
        if (!saveSettings(true)) return;
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
        ServerState.error = null;
        startForegroundService(new Intent(this, VivantService.class).setAction(VivantService.ACTION_START));
        ui.postDelayed(this::render, 300);
    }

    private void stopServer() {
        startService(new Intent(this, VivantService.class).setAction(VivantService.ACTION_STOP));
        ui.postDelayed(this::render, 300);
    }

    /** Validates and stores the settings; returns false (and says why) if they're invalid. */
    private boolean saveSettings(boolean strict) {
        int p = parse(port, -1), m = parse(maxClients, -1);
        boolean ok = p >= 1024 && p <= 65534;
        if (!ok) port.setError(getString(R.string.err_port));
        if (m < 1 || m > 100000) {
            maxClients.setError(getString(R.string.err_max));
            ok = false;
        }
        if (!ok) {
            if (!strict) {
                p = Prefs.port(this);
                m = Prefs.maxClients(this);
            } else {
                return false;
            }
        }
        Prefs.save(this, p, m, rooms.getText().toString().trim(), extra.getText().toString().trim(), autostart.isChecked());
        return true;
    }

    private static int parse(EditText e, int def) {
        try {
            return Integer.parseInt(e.getText().toString().trim());
        } catch (NumberFormatException ex) {
            return def;
        }
    }

    private void render() {
        boolean run = ServerState.running;
        boolean wanted = Prefs.running(this);
        if (run) {
            status.setText(R.string.status_running);
            status.setTextColor(getColor(R.color.ok));
        } else if (wanted) {
            status.setText(R.string.status_starting);
            status.setTextColor(getColor(R.color.warn));
        } else {
            status.setText(R.string.status_stopped);
            status.setTextColor(getColor(R.color.muted));
        }
        toggle.setText(run || wanted ? R.string.stop : R.string.start);
        online.setText(run && ServerState.online >= 0 ? getString(R.string.online_count, ServerState.online) : "");
        online.setVisibility(run ? View.VISIBLE : View.GONE);
        String err = ServerState.error;
        error.setText(err == null ? "" : err);
        error.setVisibility(err == null ? View.GONE : View.VISIBLE);
        boolean editable = !run && !wanted;
        for (View v : new View[]{port, maxClients, rooms, extra}) v.setEnabled(editable);

        if (tick++ % 5 == 0) {
            connect.setText(connectText(Prefs.port(this)));
            fingerprint.setText(hostFingerprint());
            PowerManager pm = getSystemService(PowerManager.class);
            battery.setVisibility(pm.isIgnoringBatteryOptimizations(getPackageName()) ? View.GONE : View.VISIBLE);
        }
        long v = ServerState.version();
        if (v != shownLog) {
            shownLog = v;
            String t = ServerState.logText();
            log.setText(t.isEmpty() ? getString(R.string.log_empty) : t);
            logScroll.post(() -> logScroll.fullScroll(View.FOCUS_DOWN));
        }
    }

    private String connectText(int p) {
        List<String> addrs = localAddresses();
        if (addrs.isEmpty()) return getString(R.string.no_network);
        StringBuilder sb = new StringBuilder();
        for (String a : addrs) {
            if (sb.length() > 0) sb.append('\n');
            sb.append("ssh -p ").append(p).append(" guest@").append(a);
        }
        return sb.toString();
    }

    /** IPv4 and global IPv6 addresses of every interface that is up (Wi-Fi, hotspot, Ethernet, mobile). */
    private List<String> localAddresses() {
        Set<String> v4 = new LinkedHashSet<>(), v6 = new LinkedHashSet<>();
        List<InetAddress> all = new ArrayList<>();
        try {
            for (NetworkInterface ni : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (ni.isUp() && !ni.isLoopback()) all.addAll(Collections.list(ni.getInetAddresses()));
            }
        } catch (Exception ignored) {
        }
        try {
            ConnectivityManager cm = getSystemService(ConnectivityManager.class);
            for (Network n : cm.getAllNetworks()) {
                LinkProperties lp = cm.getLinkProperties(n);
                if (lp == null) continue;
                for (LinkAddress la : lp.getLinkAddresses()) all.add(la.getAddress());
            }
        } catch (Exception ignored) {
        }
        for (InetAddress a : all) {
            if (a.isLoopbackAddress() || a.isLinkLocalAddress() || a.isAnyLocalAddress()) continue;
            if (a instanceof Inet4Address) {
                v4.add(a.getHostAddress());
            } else if (a instanceof Inet6Address) {
                byte first = a.getAddress()[0];
                if ((first & 0xfe) == 0xfc) continue; // unique local fc00::/7
                String h = a.getHostAddress();
                int pct = h.indexOf('%');
                v6.add(pct < 0 ? h : h.substring(0, pct));
            }
        }
        List<String> out = new ArrayList<>(v4);
        out.addAll(v6);
        return out;
    }

    /** SHA256 fingerprint of the host key, as ssh prints it, so users can verify they reached this phone. */
    private String hostFingerprint() {
        File pub = new File(getFilesDir(), "host_ed25519.pub");
        try {
            String[] parts = new String(Files.readAllBytes(pub.toPath()), StandardCharsets.UTF_8).trim().split("\\s+");
            byte[] blob = Base64.getDecoder().decode(parts[1]);
            byte[] d = MessageDigest.getInstance("SHA-256").digest(blob);
            String fp = Base64.getEncoder().withoutPadding().encodeToString(d);
            return getString(R.string.fingerprint, parts[0] + " SHA256:" + fp);
        } catch (Exception e) {
            return getString(R.string.fingerprint_none);
        }
    }
}
