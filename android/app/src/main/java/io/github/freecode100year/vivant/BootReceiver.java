package io.github.freecode100year.vivant;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** Starts the server after a reboot (if enabled) or an app update (if it was running). */
public class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context c, Intent intent) {
        String a = intent.getAction();
        boolean boot = Intent.ACTION_BOOT_COMPLETED.equals(a);
        boolean updated = Intent.ACTION_MY_PACKAGE_REPLACED.equals(a);
        if ((boot && Prefs.autostart(c)) || (updated && Prefs.running(c))) {
            try {
                c.startForegroundService(new Intent(c, VivantService.class).setAction(VivantService.ACTION_START));
            } catch (RuntimeException e) {
                ServerState.log("autostart failed: " + e);
            }
        }
    }
}
