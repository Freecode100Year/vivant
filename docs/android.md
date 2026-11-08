# 安卓手机当服务器

[← 返回 README](../README.md) · 简体中文 | [English](android.en.md)

把一台安卓手机变成 VIVANT 聊天室服务器：装上 APK，点「启动」，同一 Wi-Fi 或连上这台手机热点的人就能用 SSH 进来；打开「外网访问」（推荐 Tor，谁都看不到 IP）后，不在身边的人也能进来。手机里跑的是和 VPS 上完全相同的 Rust 服务器程序。

## 安装

1. 从发布页（Releases）下载：大多数手机用 `vivant-android-arm64.apk`；不确定就下 `vivant-android.apk`（通用版，体积大一些）。也可以在 [Actions → Android](https://github.com/Freecode100Year/vivant/actions/workflows/android.yml) 最新一次成功构建的 Artifacts 里找到。
2. 在手机上打开 APK，按提示允许「安装未知来源应用」。
3. 需要 Android 8.0 及以上；支持 ARM64、ARMv7、x86_64。

## 使用

1. 打开 VIVANT，点 **启动**。第一次会请求通知权限（用来显示常驻通知，可以拒绝，服务器照样运行）。
2. 建议点一次 **允许后台运行**（关闭电池优化），否则部分手机（小米、华为、OPPO、vivo 等）会在锁屏后把服务器杀掉。国产系统还可能需要在系统设置里给 VIVANT 打开「自启动」「后台运行」。
3. 「怎么进入」里列出了这台手机的地址，其他设备运行其中一条即可：

   ```
   ssh -p 2222 guest@192.168.1.23
   ```

   iPhone / iPad 用 Termius、Blink 等 SSH App，填地址、端口 2222、用户名 `guest`，不填密码。
4. 页面上的 **主机指纹** 就是对方第一次连接时 SSH 显示的指纹，可以当面核对，确认连的是这台手机。

停止：点 **停止**，或在通知栏里点「停止」。端口、房间、开机自启等在折叠起来的 **高级设置** 里。

## 外网访问（不暴露 IP）

手机一般在运营商的大内网里，外面直接连不进来。在「外网访问」里选一种。**要保护匿名用户，就用 Tor。**

| 方式 | 谁能看到 IP | 需要什么 | 别人怎么进 |
| --- | --- | --- | --- |
| **Tor 洋葱服务**（推荐） | **谁都看不到**：访客、这台手机、聊天服务器、Tor 中继都不知道访客的 IP，访客也不知道手机的 IP | 什么都不用准备 | 打开 Tor Browser 或 Orbot，`ssh guest@xxxx.onion` |
| EasyTier（自己的中继） | 手机和访客彼此看不到；中继转发时能看到连接来源，但用下面的命令运行时什么都不记录 | 一台你自己的 EasyTier 中继（海外 VPS） | 对方装 EasyTier，运行 App 里显示的命令，再 `ssh -p 2222 guest@127.0.0.1` |
| FRP（自己的服务器） | 只有 frps 能看到访客的 IP；手机和访客彼此看不到 | 一台运行 frps、你自己控制且不在中国的 VPS | 直接 `ssh -p 外网端口 guest@服务器` |

开启任何一种外网访问后，聊天服务器**只监听手机本机（127.0.0.1）**，局域网入口默认关闭；需要时可以勾选「同时允许局域网 / 热点直连」。

### Tor 洋葱服务

点「启动」后，手机内置的 Tor（和 Orbot 同一个版本）会连上 Tor 网络，并发布一个 `.onion` 地址，显示在「怎么进入」里。这个地址会一直保留，可以长期发给别人；停止后点「换一个新 .onion 地址」可以作废旧地址。

- **在中国等封锁 Tor 的地方：** 选「Snowflake 网桥」。Snowflake 是 Tor Browser 内置的网桥，伪装成普通的视频通话流量。如果 Snowflake 也连不上，去 [bridges.torproject.org](https://bridges.torproject.org)（或 Telegram 上的 @GetBridgesBot）拿 obfs4 / webtunnel 网桥，选「自定义网桥」粘贴进去，一行一个。
- 手机上的 Tor 不开 SOCKS 代理端口，其他 App 用不了它，也探测不到。
- 状态行会显示「Tor：连接中 N%」→「正在发布洋葱地址」→「已上线 ✓」。第一次通常要 30 秒到几分钟，用网桥会更久。

访客（同样建议在中国时使用 Snowflake 或 obfs4 网桥）：

```bash
# 电脑：先打开 Tor Browser（它的 SOCKS 端口是 9150）
ssh -o ProxyCommand='nc -x 127.0.0.1:9150 %h %p' guest@xxxx.onion
# Windows（Git for Windows 自带 connect.exe）
ssh -o ProxyCommand="connect -S 127.0.0.1:9150 %h %p" guest@xxxx.onion
# Linux / macOS 装了 tor 和 torsocks
torsocks ssh guest@xxxx.onion
```

安卓访客：装 Orbot，再用 Termux 运行 `torsocks ssh guest@xxxx.onion`（或在 Orbot 里开 VPN 模式后用任意 SSH App）。iPhone：用 Orbot（iOS 版）开 VPN 后，在 Termius 里填 `.onion` 地址、端口 22、用户名 `guest`。

`.onion` 地址本身就是服务器身份的证明，不需要另外核对主机指纹。

### EasyTier（自己的中继，不记录 IP）

EasyTier 正常情况下会把每个节点的公网 IP（STUN 探测到的）和网卡地址同步给网络里的所有节点，用来打洞直连。App 把这些都关掉了：

- 手机用的 EasyTier 打了一个小补丁（[`android/easytier-no-ip.patch`](../android/easytier-no-ip.patch)）：别的节点来查本机地址（`GetIpList`）时，一律回答「没有」。
- 关闭点对点、打洞、STUN、IPv6，也不开任何监听端口：手机只连你的中继，从不和访客直接相连。
- App 显示给访客的命令同样关掉 STUN 和点对点（`--stun-servers --disable-p2p --no-listener`），访客也不会把自己的公网 IP 同步出去。
- EasyTier 输出里任何像 IP 的字符串，在进入 App 日志前就被替换成 `•.•.•.•`。
- 没有默认的公共中继：公共中继是别人的机器，记不记录由不得你。

中继（海外 VPS 上运行，**不写任何日志**）：

```bash
easytier-core --network-name 任意 --network-secret 任意 -l tcp://0.0.0.0:11010 \
  --stun-servers --console-log-level off --file-log-level off
```

中继在转发的那一刻必然知道连接从哪里来（TCP 本身如此），关掉日志就不会留下记录。访客运行的命令（App 里会显示网络名、密码和你的中继）：

```
easytier-core --no-tun -d --no-listener --disable-p2p --stun-servers --network-name vivant-xxxx --network-secret xxxx -p tcp://你的VPS:11010 --port-forward tcp://127.0.0.1:2222/10.144.144.1:2224
ssh -p 2222 guest@127.0.0.1
```

不需要管理员权限。注意：访客如果不用这行命令、而是用 EasyTier 图形版默认设置加入，它会开着 STUN 和点对点，可能把自己的 IP 同步给网络里的其他人——请让访客照抄命令。

### FRP（自己的服务器）

在 VPS 上装好 frps（`bindPort`，可选 `auth.token`），在 App 里填服务器地址、端口、令牌和外网端口，并在 VPS 防火墙放行外网端口。访客什么都不用装，但 **frps 能看到每个访客的 IP**，所以只适合你完全信任那台服务器的场合。App 永远不开启 frp 的 Proxy Protocol，所以访客的 IP 只停留在 frps，不会传到手机上。

### 限额

经 Tor / EasyTier / FRP 进来的连接在服务器看来都来自本机，所以它们走单独的本地入口（Tor 用 `-onion-addr`，EasyTier / FRP 用 `-relay-addr`），共用一组宽松限额（同时 300 人、每分钟 120 个新连接），而不是挤在一个地址的小限额里。

## 访客 IP 不暴露：各层怎么保证

| 层 | 做法 |
| --- | --- |
| 进入手机的连接 | Tor / EasyTier / FRP 模式下，所有访客连接在手机上都显示为 `127.0.0.1`，聊天服务器从系统层面就拿不到访客 IP；局域网入口默认关闭 |
| 聊天服务器 | 不记录、不显示任何地址：日志里只有自己的监听地址，`/who` 等命令只显示昵称；限流用的是每次启动随机加盐的地址哈希，只在内存里 |
| Tor | 手机不开 SOCKS / HTTP 代理端口（其他 App 用不了、也探测不到）；`SafeLogging 1`，日志不进系统 logcat |
| EasyTier | 补丁后不回答地址查询；关闭点对点、打洞、STUN、IPv6、监听端口；日志里的地址打码；中继不写日志 |
| FRP | 不开 Proxy Protocol；frpc 本来就不知道访客 IP |
| App | 不调用 Android 日志（logcat），不写日志文件；「日志」只是内存里最近 400 行，停止或退出就没了；聊天记录只在内存里 |

自己验证（手机连电脑，开 USB 调试）：

```bash
adb logcat | grep -iE "vivant|tor"            # 有访客连入时，不应出现任何访客地址
adb shell netstat -tn | grep -E ":2222|:2225"  # Tor 模式下只应看到 127.0.0.1 的监听和连接
```

## 网络说明

- **局域网 / 热点**：最可靠。手机开热点，别人连上热点后用热点地址（通常是 `192.168.x.1` 或 `10.x.x.x`）。
- **手机流量**：运营商一般不给公网 IPv4（CGNAT），外网直接连不进来，请用上面的「外网访问」（Tor）。有公网 IPv6 时列表里会显示 IPv6 地址，对方也有 IPv6 才能连。
- 端口只能用 1024 以上（安卓普通 App 不能监听 22、443 这类端口）。

## 设置

| 设置 | 说明 |
| --- | --- |
| 端口 | 默认 2222。`端口 + 1`（状态接口）、`端口 + 2`（FRP 入口）、`端口 + 3`（Tor 入口）也会被占用，都只监听 127.0.0.1。 |
| 最多同时在线 | 默认 100，手机性能足够撑起几百人，但电量和发热会上升。 |
| 公共房间 | 同服务器的 `-rooms`，例如 `lobby:大厅,tech:技术`；留空用默认房间。 |
| 额外参数 | 直接传给服务器的命令行参数，例如 `-history-ttl 24h -msg-rate 2`，全部参数见[部署与配置](deploy.md)。 |

## 隐私

和 VPS 版一样：没有账号、日志里没有 IP / 昵称 / 消息，聊天只在内存里，停止服务器就全部清空。App 不联网上报任何东西；手机上默认关闭了 Tor 出口列表下载（`-tor-exits=false`），省流量。主机密钥保存在 App 私有目录，卸载 App 会一起删除（下次就是新指纹）。

**手机本身就是服务器：** 聊天内容在运行时就在这台手机的内存里，`.onion` 私钥也存在手机上。手机若被没收或检查，能证明「这个洋葱地址是这台手机发布的」。需要时先停止服务器（聊天随即清空），想换洋葱地址，停止后点「换一个新 .onion 地址」，旧地址立即作废。Tor 能隐藏 IP，但不能保护你发出的内容：别在聊天里说能认出你的事。

## 自己构建

需要 Rust、Android NDK（r25+）、JDK 24+、Gradle 9.8+ 或 Android Studio；EasyTier / frpc 另需 protoc、cmake、Go：

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/ndk
android/build-native.sh               # VIVANT 服务器 → jniLibs/*/libvivant.so
android/build-tunnels.sh easytier     # EasyTier（自动打补丁）→ jniLibs/*/libeasytier.so
android/build-tunnels.sh frpc         # frpc → jniLibs/*/libfrpc.so
gradle -p android assembleRelease     # → android/app/build/outputs/apk/release/
```

EasyTier 和 frpc 从官方源码按 `android/tunnels.env` 里固定的版本编译（EasyTier 打上 `easytier-no-ip.patch`）；Tor 和网桥用 Guardian Project（Orbot 的开发者）发布的 `tor-android` 和 IPtProxy，和 Orbot 完全相同。VIVANT 服务器、EasyTier 和 frpc 以 `lib*.so` 的名字打进 APK（其实是普通可执行文件），这样安卓会把它们解压到 App 的原生库目录——App 唯一允许执行程序的地方；前台服务用子进程启动它们。Tor 在 App 进程里运行。

**签名：** 不设置时用调试密钥签名，每次 CI 构建的密钥不同，升级前要先卸载旧版。想要稳定签名，在仓库 Secrets 里添加 `ANDROID_KEYSTORE_BASE64`（`.jks` 文件的 base64）、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`（可选 `ANDROID_KEY_PASSWORD`）。本地构建用环境变量 `VIVANT_KEYSTORE`、`VIVANT_KEYSTORE_PASSWORD`、`VIVANT_KEY_ALIAS`、`VIVANT_KEY_PASSWORD`。
