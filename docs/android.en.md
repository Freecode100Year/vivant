# Android phone as a server

[← Back to README](../README.en.md) · [简体中文](android.md) | English

Turn an Android phone into a VIVANT chat server: install the APK, tap **Start**, and anyone on the same Wi-Fi or on the phone's hotspot can join over SSH; turn on **Internet access** and people elsewhere can join too. The phone runs exactly the same Rust server as a VPS.

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

## Internet access (NAT traversal)

Phones usually sit behind carrier NAT, so nobody outside can connect directly. Pick one under **Internet access**:

| Mode | What you need | How others join |
| --- | --- | --- |
| **Off** | — | same Wi-Fi / hotspot only |
| **EasyTier** (no server) | nothing; the app makes up a network name and password | they install [EasyTier](https://easytier.cn), run the line the app shows, then `ssh -p 2222 guest@127.0.0.1` |
| **FRP** (your own server) | a VPS with a public IP running frps | nothing to install: `ssh -p PUBLIC_PORT guest@SERVER` |

**EasyTier:** free P2P hole punching that copes with China's layered NATs, falling back to a public relay (`tcp://public.easytier.top:11010` by default; use your own node under Advanced). Send the network name and password to whoever should join; **New pair** retires the old ones. Visitors run something like:

```
easytier-core --no-tun -d --network-name vivant-xxxx --network-secret xxxx -p tcp://public.easytier.top:11010 --port-forward tcp://127.0.0.1:2222/10.144.144.1:2224
ssh -p 2222 guest@127.0.0.1
```

No admin rights needed. With the EasyTier GUI / mobile app, join the same network and `ssh -p 2224 guest@10.144.144.1` (use the port the app shows). The phone needs no VPN permission and no root.

**FRP:** run frps on a VPS (`bindPort`, optional `auth.token`), then fill in server, port, token and public port in the app. Open the public port in the VPS firewall. The link to frps uses TLS.

**Limits:** everyone coming through a tunnel looks like the phone itself to the server, so the app gives tunnels their own local entrance (`-relay-addr`) where they share the same generous limits as Tor users (300 at once, 120 new connections a minute) instead of one address's small quota.

**Privacy:** SSH is end-to-end encrypted, so relays and frps can't read the chat — but the public EasyTier relay and your frps do see visitors' IP addresses. If that matters, use your own EasyTier node or frps.

## Networking

- **LAN / hotspot**: the reliable way. Turn on the hotspot; people on it use the hotspot address (usually `192.168.x.1` or `10.x.x.x`).
- **Mobile data**: carriers rarely give a public IPv4 (CGNAT), so the internet can't reach it directly — use Internet access above. With public IPv6 the IPv6 address is listed; the other side needs IPv6 too.
- Ports must be above 1024 (normal Android apps can't listen on 22, 443, …).

## Settings

| Setting | Meaning |
| --- | --- |
| Port | Default 2222. `port + 1` (status) and `port + 2` (tunnel entrance) are used too, both 127.0.0.1 only. |
| Max online users | Default 100. A phone can carry a few hundred, at the cost of battery and heat. |
| Public rooms | The server's `-rooms`, e.g. `lobby:Lobby,tech:Tech`; empty = default rooms. |
| EasyTier nodes | Default `tcp://public.easytier.top:11010`; your own node(s), comma separated. |
| Extra flags | Passed straight to the server, e.g. `-history-ttl 24h -msg-rate 2`; all flags in [Deployment and configuration](deploy.en.md). |

## Privacy

Same as on a VPS: no accounts, no IPs / nicknames / messages in the log, chat kept in memory only and gone when the server stops. The app reports nothing anywhere; the Tor exit list download is off on phones (`-tor-exits=false`) to save data. The host key lives in the app's private storage and is deleted with the app (you'd get a new fingerprint).

## Build it yourself

Needs Rust, the Android NDK (r25+), JDK 17, and Gradle 8.14+ or Android Studio; the tunnel tools also need Go, protoc and cmake:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/ndk
android/build-native.sh               # VIVANT server -> jniLibs/*/libvivant.so
android/build-tunnels.sh easytier     # EasyTier -> jniLibs/*/libeasytier.so
android/build-tunnels.sh frpc         # frpc -> jniLibs/*/libfrpc.so
gradle -p android assembleRelease     # -> android/app/build/outputs/apk/release/
```

EasyTier and frp are built from their official sources at the versions pinned in `android/tunnels.env`, not from someone else's binaries. All three programs are packed as `lib*.so` (really plain executables) so Android extracts them to the app's native library directory — the only place an app may execute programs from; a foreground service runs them as child processes.

**Signing:** without configuration the APK is debug-signed, with a different key on every CI run, so uninstall the old one before updating. For stable signing add the repository secrets `ANDROID_KEYSTORE_BASE64` (base64 of a `.jks`), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` (optionally `ANDROID_KEY_PASSWORD`). Locally use `VIVANT_KEYSTORE`, `VIVANT_KEYSTORE_PASSWORD`, `VIVANT_KEY_ALIAS`, `VIVANT_KEY_PASSWORD`.
