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
import android.widget.RadioGroup;
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
    private TextView status, online, tunnelStatus, error, connect, netHint, netConnect, fingerprint, log, advToggle, logToggle;
    private Button toggle, battery, etNew;
    private EditText port, maxClients, rooms, extra, etPeer, etName, etSecret;
    private CheckBox autostart;
    private RadioGroup tunnel;
    private View etGroup, advGroup;
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
        tunnelStatus = findViewById(R.id.tunnel_status);
        error = findViewById(R.id.error);
        connect = findViewById(R.id.connect);
        netHint = findViewById(R.id.net_hint);
        netConnect = findViewById(R.id.net_connect);
        fingerprint = findViewById(R.id.fingerprint);
        log = findViewById(R.id.log);
        logScroll = findViewById(R.id.log_scroll);
        advToggle = findViewById(R.id.adv_toggle);
        advGroup = findViewById(R.id.adv_group);
        logToggle = findViewById(R.id.log_toggle);
        toggle = findViewById(R.id.toggle);
        battery = findViewById(R.id.battery);
        etNew = findViewById(R.id.et_new);
        port = findViewById(R.id.port);
        maxClients = findViewById(R.id.max_clients);
        rooms = findViewById(R.id.rooms);
        extra = findViewById(R.id.extra);
        etPeer = findViewById(R.id.et_peer);
        etName = findViewById(R.id.et_name);
        etSecret = findViewById(R.id.et_secret);
        autostart = findViewById(R.id.autostart);
        tunnel = findViewById(R.id.tunnel);
        etGroup = findViewById(R.id.et_group);
        try {
            ((TextView) findViewById(R.id.version)).setText("v" + getPackageManager().getPackageInfo(getPackageName(), 0).versionName);
        } catch (PackageManager.NameNotFoundException ignored) {
        }

        loadFields();
        tunnel.setOnCheckedChangeListener((g, id) -> {
            Prefs.sp(this).edit().putString("tunnel", tunnelFromRadio()).apply();
            showTunnelGroups();
            tick = 0;
            render();
        });
        autostart.setOnCheckedChangeListener((v, on) -> Prefs.sp(this).edit().putBoolean("autostart", on).apply());
        toggle.setOnClickListener(v -> {
            if (ServerState.running || Prefs.running(this)) stopServer();
            else startServer();
        });
        etNew.setOnClickListener(v -> {
            Prefs.regenerateEasyTier(this);
            etName.setText(Prefs.etName(this));
            etSecret.setText(Prefs.etSecret(this));
            tick = 0;
        });
        findViewById(R.id.copy).setOnClickListener(v -> {
            String text = connect.getText().toString();
            if (netConnect.getVisibility() == View.VISIBLE) text += "\n\n" + netConnect.getText();
            getSystemService(ClipboardManager.class).setPrimaryClip(ClipData.newPlainText("ssh", text));
            Toast.makeText(this, R.string.copied, Toast.LENGTH_SHORT).show();
        });
        battery.setOnClickListener(v -> {
            try {
                startActivity(new Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:" + getPackageName())));
            } catch (RuntimeException e) {
                startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS));
            }
        });
        advToggle.setOnClickListener(v -> fold(advGroup, advToggle, R.string.adv_title, R.string.adv_title_open));
        logToggle.setOnClickListener(v -> {
            fold(logScroll, logToggle, R.string.log_title, R.string.log_title_open);
            shownLog = -1;
            render();
        });
    }

    private void fold(View group, TextView head, int closed, int open) {
        boolean show = group.getVisibility() != View.VISIBLE;
        group.setVisibility(show ? View.VISIBLE : View.GONE);
        head.setText(show ? open : closed);
    }

    private void loadFields() {
        port.setText(String.valueOf(Prefs.port(this)));
        maxClients.setText(String.valueOf(Prefs.maxClients(this)));
        rooms.setText(Prefs.rooms(this));
        extra.setText(Prefs.extraArgs(this));
        etPeer.setText(Prefs.sp(this).getString("et_peer", ""));
        autostart.setChecked(Prefs.autostart(this));
        etName.setText(Prefs.etName(this));
        etSecret.setText(Prefs.etSecret(this));
        String t = Prefs.tunnel(this);
        tunnel.check(Prefs.TUNNEL_EASYTIER.equals(t) ? R.id.tunnel_et : R.id.tunnel_off);
        showTunnelGroups();
    }

    private String tunnelFromRadio() {
        int id = tunnel.getCheckedRadioButtonId();
        return id == R.id.tunnel_et ? Prefs.TUNNEL_EASYTIER : Prefs.TUNNEL_OFF;
    }

    private void showTunnelGroups() {
        String t = tunnelFromRadio();
        etGroup.setVisibility(Prefs.TUNNEL_EASYTIER.equals(t) ? View.VISIBLE : View.GONE);
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
        if (!saveSettings()) return;
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
        ServerState.error = null;
        startForegroundService(new Intent(this, VivantService.class).setAction(VivantService.ACTION_START));
        tick = 0;
        ui.postDelayed(this::render, 300);
    }

    private void stopServer() {
        startService(new Intent(this, VivantService.class).setAction(VivantService.ACTION_STOP));
        ui.postDelayed(this::render, 300);
    }

    /** Validates and stores every field; on a problem, opens the right section and says why. */
    private boolean saveSettings() {
        int p = parse(port), m = parse(maxClients);
        boolean ok = true;
        if (p < 1024 || p > 65000) {
            port.setError(getString(R.string.err_port));
            ok = false;
        }
        if (m < 1 || m > 100000) {
            maxClients.setError(getString(R.string.err_max));
            ok = false;
        }
        if (!ok && advGroup.getVisibility() != View.VISIBLE) fold(advGroup, advToggle, R.string.adv_title, R.string.adv_title_open);
        String t = tunnelFromRadio();
        String name = etName.getText().toString().trim(), secret = etSecret.getText().toString().trim();
        if (Prefs.TUNNEL_EASYTIER.equals(t)) {
            if (name.isEmpty()) {
                etName.setError(getString(R.string.err_need));
                ok = false;
            }
            if (secret.isEmpty()) {
                etSecret.setError(getString(R.string.err_need));
                ok = false;
            }
        }
        if (!ok) return false;
        Prefs.sp(this).edit()
                .putInt("port", p)
                .putInt("max_clients", m)
                .putString("rooms", rooms.getText().toString().trim())
                .putString("extra_args", extra.getText().toString().trim())
                .putString("et_peer", etPeer.getText().toString().trim())
                .putBoolean("autostart", autostart.isChecked())
                .putString("tunnel", t)
                .putString("et_name", name)
                .putString("et_secret", secret)
                .apply();
        return true;
    }

    private static int parse(EditText e) {
        try {
            return Integer.parseInt(e.getText().toString().trim());
        } catch (NumberFormatException ex) {
            return -1;
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

        int ts = ServerState.tunnel;
        tunnelStatus.setVisibility(ts == ServerState.TUNNEL_OFF ? View.GONE : View.VISIBLE);
        if (ts == ServerState.TUNNEL_UP) {
            tunnelStatus.setText(R.string.tunnel_up);
            tunnelStatus.setTextColor(getColor(R.color.ok));
        } else if (ts == ServerState.TUNNEL_FAILED) {
            tunnelStatus.setText(R.string.tunnel_failed);
            tunnelStatus.setTextColor(getColor(R.color.err));
        } else {
            tunnelStatus.setText(R.string.tunnel_connecting);
            tunnelStatus.setTextColor(getColor(R.color.warn));
        }
        String err = ServerState.error != null ? ServerState.error : ServerState.tunnelError;
        error.setText(err == null ? "" : err);
        error.setVisibility(err == null ? View.GONE : View.VISIBLE);

        boolean editable = !run && !wanted;
        for (View v : new View[]{port, maxClients, rooms, extra, etPeer, etName, etSecret, etNew,
                findViewById(R.id.tunnel_off), findViewById(R.id.tunnel_et)}) {
            v.setEnabled(editable);
        }

        if (tick++ % 5 == 0) {
            connect.setText(lanText(Prefs.port(this)));
            renderInternet();
            fingerprint.setText(hostFingerprint());
            PowerManager pm = getSystemService(PowerManager.class);
            battery.setVisibility(pm.isIgnoringBatteryOptimizations(getPackageName()) ? View.GONE : View.VISIBLE);
        }
        if (logScroll.getVisibility() == View.VISIBLE) {
            long v = ServerState.version();
            if (v != shownLog) {
                shownLog = v;
                String t = ServerState.logText();
                log.setText(t.isEmpty() ? getString(R.string.log_empty) : t);
                logScroll.post(() -> logScroll.fullScroll(View.FOCUS_DOWN));
            }
        }
    }

    /** What people outside the LAN type, for the chosen kind of internet access. */
    private void renderInternet() {
        String t = tunnelFromRadio();
        int p = parse(port);
        int relay = p >= 1024 ? p + 2 : Prefs.relayPort(this);
        String cmd = null, hint = null;
        if (Prefs.TUNNEL_EASYTIER.equals(t)) {
            String peer = etPeer.getText().toString().trim();
            if (peer.isEmpty()) peer = Prefs.DEFAULT_ET_PEER;
            peer = peer.split("[,\\s]+")[0];
            cmd = "easytier-core --no-tun -d --network-name " + etName.getText().toString().trim()
                    + " --network-secret " + etSecret.getText().toString().trim()
                    + " -p " + peer
                    + " --port-forward tcp://127.0.0.1:2222/" + Prefs.ET_IP + ":" + relay
                    + "\n\nssh -p 2222 guest@127.0.0.1";
            hint = getString(R.string.net_hint_et) + "\n" + getString(R.string.net_hint_et_app, relay, Prefs.ET_IP);
        }
        netHint.setVisibility(cmd == null ? View.GONE : View.VISIBLE);
        netConnect.setVisibility(cmd == null ? View.GONE : View.VISIBLE);
        if (cmd != null) {
            netHint.setText(hint);
            netConnect.setText(cmd);
        }
    }

    private String lanText(int p) {
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
            String h = a.getHostAddress();
            if (Prefs.ET_IP.equals(h)) continue;
            if (a instanceof Inet4Address) {
                v4.add(h);
            } else if (a instanceof Inet6Address) {
                byte first = a.getAddress()[0];
                if ((first & 0xfe) == 0xfc) continue; // unique local fc00::/7
                int pct = h.indexOf('%');
                v6.add(pct < 0 ? h : h.substring(0, pct));
            }
        }
        List<String> out = new ArrayList<>(v4);
        out.addAll(v6);
        return out;
    }

    /** SHA256 fingerprint of the host key, as ssh prints it, so users can check they reached this phone. */
    private String hostFingerprint() {
        File pub = new File(getFilesDir(), "host_ed25519.pub");
        try {
            String[] parts = new String(Files.readAllBytes(pub.toPath()), StandardCharsets.UTF_8).trim().split("\\s+");
            byte[] blob = Base64.getDecoder().decode(parts[1]);
            byte[] d = MessageDigest.getInstance("SHA-256").digest(blob);
            return getString(R.string.fingerprint, "SHA256:" + Base64.getEncoder().withoutPadding().encodeToString(d));
        } catch (Exception e) {
            return getString(R.string.fingerprint_none);
        }
    }
}
