# Android phone as a server

[← Back to README](../README.en.md) · [简体中文](android.md) | English

Turn an Android phone into a VIVANT chat server: install the APK, tap **Start**, and anyone on the same Wi-Fi or on the phone's hotspot can join over SSH; turn on **Internet access** (Tor recommended: nobody sees any IP) and people elsewhere can join too. The phone runs exactly the same Rust server as a VPS.

## Install

1. From Releases: most phones want `vivant-android-arm64.apk`; if unsure take `vivant-android.apk` (universal, bigger). Also in the Artifacts of the latest successful run of [Actions → Android](https://github.com/Freecode100Year/vivant/actions/workflows/android.yml).
2. Open it on the phone and allow "install unknown apps" when asked.
3. Needs Android 8.0+; ARM64, ARMv7 and x86_64 are included.

## Use

1. Open VIVANT and tap **Start**. The first time it asks for notification permission (for the ongoing notification; the server runs either way).
2. Tap **Allow running in background** (battery optimisation) once — otherwise many phones (Xiaomi, Huawei, OPPO, vivo, Samsung…) kill the server when the screen is off. Some also need "autostart" / "run in background" enabled in system settings.
3. **How to join** lists the phone's addresses; others run any of the lines:

   ```
   ssh -p 2222 guest@192.168.1.23
   ```

   On iPhone / iPad use an SSH app such as Termius or Blink: that address, port 2222, user `guest`, no password.
4. The **host key fingerprint** shown is what SSH prints on first connect — compare it in person to be sure you reached this phone.

Stop with **Stop** or from the notification. Port, rooms, start-on-boot and the rest are under the folded **Advanced** section.

## Internet access (without exposing IPs)

Phones usually sit behind carrier NAT, so nobody outside can connect directly. Pick one under **Internet access**. **To protect identities, use Tor.**

| Mode | Who sees IP addresses | What you need | How others join |
| --- | --- | --- | --- |
| **Tor onion service** (recommended) | **Nobody**: visitors, this phone, the chat server and relays never learn each other's IP | nothing | open Tor Browser or Orbot, `ssh guest@xxxx.onion` |
| EasyTier (your own relay) | the relay node sees everyone's IP | an EasyTier node you control | they install EasyTier and run the line the app shows |
| FRP (your own server) | frps sees visitors' IPs | a VPS running frps | `ssh -p PUBLIC_PORT guest@SERVER` |

### Tor onion service

After **Start**, the built-in Tor (the same build Orbot uses) connects to the Tor network and publishes a `.onion` address, shown under **How to join**. The address stays the same, so it can be shared long-term; stop and tap **New .onion address** to retire it.

- **In China and other places that block Tor:** choose **Snowflake bridge** — Tor Browser's built-in bridge, disguised as ordinary video-call traffic. If Snowflake doesn't get through either, get obfs4 / webtunnel bridges from [bridges.torproject.org](https://bridges.torproject.org) (or @GetBridgesBot on Telegram) and paste them under **Custom bridges**, one per line.
- **LAN entrance off by default:** with Tor, the chat server only listens on the phone itself, so even people nearby come in through Tor and no local address is revealed. Tick **Also allow LAN / hotspot** if you want direct local access too.
- The phone's Tor opens no SOCKS port: other apps can neither use nor probe it.
- The status line goes "Tor: connecting N%" → "publishing the onion address" → "online ✓". The first time takes 30 seconds to a few minutes; longer with bridges.

Visitors (in China, also with Snowflake or obfs4 bridges):

```bash
# computer: open Tor Browser first (its SOCKS port is 9150)
ssh -o ProxyCommand='nc -x 127.0.0.1:9150 %h %p' guest@xxxx.onion
# Windows (connect.exe comes with Git for Windows)
ssh -o ProxyCommand="connect -S 127.0.0.1:9150 %h %p" guest@xxxx.onion
# Linux / macOS with tor and torsocks
torsocks ssh guest@xxxx.onion
```

Android visitors: Orbot plus Termux (`torsocks ssh guest@xxxx.onion`), or Orbot's VPN mode with any SSH app. iPhone: Orbot for iOS in VPN mode, then Termius with the `.onion` address, port 22, user `guest`.

The `.onion` address itself proves the server's identity; there's no host key fingerprint to check.

### EasyTier (your own relay)

To keep IPs hidden, the app's EasyTier mode **turns off P2P and hole punching** and has **no public relay by default**: the phone and visitors connect only to the relay you enter and never see each other's IP. The relay itself does see both sides' IPs, so only use a node you control, outside China (e.g. on a VPS abroad: `easytier-core --network-name any --network-secret any -l tcp://0.0.0.0:11010`). Visitors run something like:

```
easytier-core --no-tun -d --no-listener --disable-p2p --network-name vivant-xxxx --network-secret xxxx -p tcp://YOUR_VPS:11010 --port-forward tcp://127.0.0.1:2222/10.144.144.1:2224
ssh -p 2222 guest@127.0.0.1
```

### FRP (your own server)

Run frps on a VPS (`bindPort`, optional `auth.token`), fill in server, port, token and public port in the app, and open the public port in the VPS firewall. Visitors install nothing, but **frps sees every visitor's IP**.

### Limits

Everyone arriving through Tor or a tunnel looks like the phone itself to the server, so they come in through separate local entrances (`-onion-addr` for Tor, `-relay-addr` for EasyTier / FRP) and share generous limits (300 at once, 120 new connections a minute) instead of one address's small quota.

## Networking

- **LAN / hotspot**: the reliable way. Turn on the hotspot; people on it use the hotspot address (usually `192.168.x.1` or `10.x.x.x`).
- **Mobile data**: carriers rarely give a public IPv4 (CGNAT), so the internet can't reach it directly — use Internet access above. With public IPv6 the IPv6 address is listed; the other side needs IPv6 too.
- Ports must be above 1024 (normal Android apps can't listen on 22, 443, …).

## Settings

| Setting | Meaning |
| --- | --- |
| Port | Default 2222. `port + 1` (status), `port + 2` (tunnel entrance) and `port + 3` (Tor entrance) are used too, all 127.0.0.1 only. |
| Max online users | Default 100. A phone can carry a few hundred, at the cost of battery and heat. |
| Public rooms | The server's `-rooms`, e.g. `lobby:Lobby,tech:Tech`; empty = default rooms. |
| Extra flags | Passed straight to the server, e.g. `-history-ttl 24h -msg-rate 2`; all flags in [Deployment and configuration](deploy.en.md). |

## Privacy

Same as on a VPS: no accounts, no IPs / nicknames / messages in the log, chat kept in memory only and gone when the server stops. The app reports nothing anywhere; the Tor exit list download is off on phones (`-tor-exits=false`) to save data. The host key lives in the app's private storage and is deleted with the app (you'd get a new fingerprint).

**The phone is the server:** while running, the chat sits in this phone's memory, and the `.onion` private key is stored on it. If the phone is seized or inspected, it proves that this onion address was published from it. If needed, stop the server first (the chat is cleared). To get a new onion address, stop and tap **New .onion address**; the old one is retired at once. Tor hides IPs; it can't protect what you say — never post anything that identifies you.

## Build it yourself

Needs Rust, the Android NDK (r25+), JDK 24+, and Gradle 9.8+ or Android Studio; the tunnel tools also need Go, protoc and cmake:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/ndk
android/build-native.sh               # VIVANT server -> jniLibs/*/libvivant.so
android/build-tunnels.sh easytier     # EasyTier -> jniLibs/*/libeasytier.so
android/build-tunnels.sh frpc         # frpc -> jniLibs/*/libfrpc.so
gradle -p android assembleRelease     # -> android/app/build/outputs/apk/release/
```

EasyTier and frp are built from their official sources at the versions pinned in `android/tunnels.env`; Tor and the bridges are Guardian Project's (Orbot's developers) `tor-android` and IPtProxy, exactly as in Orbot. All three programs are packed as `lib*.so` (really plain executables) so Android extracts them to the app's native library directory — the only place an app may execute programs from; a foreground service runs them as child processes.

**Signing:** without configuration the APK is debug-signed, with a different key on every CI run, so uninstall the old one before updating. For stable signing add the repository secrets `ANDROID_KEYSTORE_BASE64` (base64 of a `.jks`), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` (optionally `ANDROID_KEY_PASSWORD`). Locally use `VIVANT_KEYSTORE`, `VIVANT_KEYSTORE_PASSWORD`, `VIVANT_KEY_ALIAS`, `VIVANT_KEY_PASSWORD`.
