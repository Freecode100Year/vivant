# Android phone as a server

[← Back to README](../README.en.md) · [简体中文](android.md) | English

Turn an Android phone into a VIVANT chat server: install the APK, tap **Start server**, and anyone on the same Wi-Fi or on the phone's hotspot can join over SSH. Good for meetups, classrooms, offline places and small ad-hoc groups. The phone runs exactly the same Rust server as a VPS.

## Install

1. Download `vivant-android.apk` from Releases, or from the Artifacts of the latest successful run of [Actions → Android](https://github.com/Freecode100Year/vivant/actions/workflows/android.yml).
2. Open it on the phone and allow "install unknown apps" when asked.
3. Needs Android 8.0+; ARM64, ARMv7 and x86_64 are included.

## Use

1. Open VIVANT and tap **Start server**. The first time it asks for notification permission (for the ongoing notification; the server runs either way).
2. Tap **Disable battery optimisation** once — otherwise many phones (Xiaomi, Huawei, OPPO, vivo, Samsung…) kill the server when the screen is off. Some also need "autostart" / "run in background" enabled in system settings.
3. **How to connect** lists the phone's addresses; others run any of the lines:

   ```
   ssh -p 2222 guest@192.168.1.23
   ```

   On iPhone / iPad use an SSH app such as Termius or Blink: that address, port 2222, user `guest`, no password.
4. The **host key fingerprint** shown is what SSH prints on first connect — compare it in person to be sure you reached this phone.

Stop with **Stop** or from the notification. With **Start on boot** checked the server comes back after a reboot.

## Networking

- **LAN / hotspot**: the reliable way. Turn on the hotspot; people on it use the hotspot address (usually `192.168.x.1` or `10.x.x.x`).
- **Mobile data**: carriers rarely give a public IPv4 (CGNAT), so the internet can't reach it. With public IPv6 the IPv6 address is listed; the other side needs IPv6 too.
- Ports must be above 1024 (normal Android apps can't listen on 22, 443, …).

## Settings

| Setting | Meaning |
| --- | --- |
| Port | Default 2222. `port + 1` is used for the internal status endpoint (127.0.0.1 only). |
| Max online users | Default 100. A phone can carry a few hundred, at the cost of battery and heat. |
| Public rooms | The server's `-rooms`, e.g. `lobby:Lobby,tech:Tech`; empty = default rooms. |
| Extra flags | Passed straight to the server, e.g. `-history-ttl 24h -msg-rate 2`; all flags in [Deployment and configuration](deploy.en.md). |

## Privacy

Same as on a VPS: no accounts, no IPs / nicknames / messages in the log, chat kept in memory only and gone when the server stops. The app reports nothing anywhere; the Tor exit list download is off on phones (`-tor-exits=false`) to save data. The host key lives in the app's private storage and is deleted with the app (you'd get a new fingerprint).

## Build it yourself

Needs Rust, the Android NDK (r25+), JDK 17, and Gradle 8.14+ or Android Studio:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
ANDROID_NDK_HOME=/path/to/ndk android/build-native.sh     # -> android/app/src/main/jniLibs/*/libvivant.so
gradle -p android assembleRelease                          # -> android/app/build/outputs/apk/release/app-release.apk
```

The server is packed as `libvivant.so` (really a plain executable) so Android extracts it to the app's native library directory — the only place an app may execute programs from; a foreground service runs it as a child process.

**Signing:** without configuration the APK is debug-signed, with a different key on every CI run, so uninstall the old one before updating. For stable signing add the repository secrets `ANDROID_KEYSTORE_BASE64` (base64 of a `.jks`), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` (optionally `ANDROID_KEY_PASSWORD`). Locally use `VIVANT_KEYSTORE`, `VIVANT_KEYSTORE_PASSWORD`, `VIVANT_KEY_ALIAS`, `VIVANT_KEY_PASSWORD`.
