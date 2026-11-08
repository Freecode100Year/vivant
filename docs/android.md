# 安卓手机当服务器

[← 返回 README](../README.md) · 简体中文 | [English](android.en.md)

把一台安卓手机变成 VIVANT 聊天室服务器：装上 APK，点「启动」，同一 Wi-Fi 或连上这台手机热点的人就能用 SSH 进来；打开「外网访问」后，不在身边的人也能进来。手机里跑的是和 VPS 上完全相同的 Rust 服务器程序。

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

## 外网访问（内网穿透）

手机一般在运营商的大内网里，外面直接连不进来。在「外网访问」里选一种：

| 方式 | 需要什么 | 别人怎么进 |
| --- | --- | --- |
| **关闭** | — | 只有同一 Wi-Fi / 热点 |
| **EasyTier**（不用服务器） | 什么都不用准备；App 自动生成网络名和密码 | 对方先装 [EasyTier](https://easytier.cn)，运行 App 里显示的那行命令，再 `ssh -p 2222 guest@127.0.0.1` |
| **FRP**（自己的服务器） | 一台有公网 IP、运行 frps 的 VPS | 什么都不用装，直接 `ssh -p 外网端口 guest@服务器地址` |

**EasyTier：** 免费、点对点打洞，国内复杂 NAT 也能通；打不通时自动走公共中继（默认 `tcp://public.easytier.top:11010`，可在高级设置里换成自己的节点）。把网络名和密码发给想进来的人，「换一组」可以随时作废旧的。对方运行的命令类似：

```
easytier-core --no-tun -d --network-name vivant-xxxx --network-secret xxxx -p tcp://public.easytier.top:11010 --port-forward tcp://127.0.0.1:2222/10.144.144.1:2224
ssh -p 2222 guest@127.0.0.1
```

不需要管理员权限。用 EasyTier 图形版 / 手机 App 的话，加入同名网络后直接 `ssh -p 2224 guest@10.144.144.1`（端口以 App 显示为准）。手机这边不建 VPN、不要 root，也不占用系统 VPN 位置。

**FRP：** 在 VPS 上装好 frps（`bindPort`，可选 `auth.token`），在 App 里填服务器地址、端口、令牌和外网端口。别忘了在 VPS 防火墙里放行外网端口。和 frps 之间默认走 TLS。

**限额：** 经穿透进来的连接在服务器看来都来自本机，所以 App 让它们走单独的本地入口（`-relay-addr`），共用一组和 Tor 用户相同的宽松限额（同时 300 人、每分钟 120 个新连接），而不是挤在一个地址的小限额里。

**隐私：** SSH 本身端到端加密，中继和 frps 看不到聊天内容；但 EasyTier 公共中继和你的 frps 能看到访客的 IP。对 IP 敏感的人，请用自己的 EasyTier 节点或 frps。

## 网络说明

- **局域网 / 热点**：最可靠。手机开热点，别人连上热点后用热点地址（通常是 `192.168.x.1` 或 `10.x.x.x`）。
- **手机流量**：运营商一般不给公网 IPv4（CGNAT），外网直接连不进来，请用上面的「外网访问」。有公网 IPv6 时列表里会显示 IPv6 地址，对方也有 IPv6 才能连。
- 端口只能用 1024 以上（安卓普通 App 不能监听 22、443 这类端口）。

## 设置

| 设置 | 说明 |
| --- | --- |
| 端口 | 默认 2222。`端口 + 1`（状态接口）和 `端口 + 2`（穿透入口）也会被占用，都只监听 127.0.0.1。 |
| 最多同时在线 | 默认 100，手机性能足够撑起几百人，但电量和发热会上升。 |
| 公共房间 | 同服务器的 `-rooms`，例如 `lobby:大厅,tech:技术`；留空用默认房间。 |
| EasyTier 节点 | 默认 `tcp://public.easytier.top:11010`，可填自己的节点，多个用逗号分隔。 |
| 额外参数 | 直接传给服务器的命令行参数，例如 `-history-ttl 24h -msg-rate 2`，全部参数见[部署与配置](deploy.md)。 |

## 隐私

和 VPS 版一样：没有账号、日志里没有 IP / 昵称 / 消息，聊天只在内存里，停止服务器就全部清空。App 不联网上报任何东西；手机上默认关闭了 Tor 出口列表下载（`-tor-exits=false`），省流量。主机密钥保存在 App 私有目录，卸载 App 会一起删除（下次就是新指纹）。

## 自己构建

需要 Rust、Android NDK（r25+）、JDK 17、Gradle 8.14+ 或 Android Studio；穿透工具另需 Go、protoc、cmake：

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/ndk
android/build-native.sh               # VIVANT 服务器 → jniLibs/*/libvivant.so
android/build-tunnels.sh easytier     # EasyTier → jniLibs/*/libeasytier.so
android/build-tunnels.sh frpc         # frpc → jniLibs/*/libfrpc.so
gradle -p android assembleRelease     # → android/app/build/outputs/apk/release/
```

EasyTier 和 frp 都从官方源码按 `android/tunnels.env` 里固定的版本编译，不用别人的预编译文件。三个程序都以 `lib*.so` 的名字打进 APK（其实是普通可执行文件），这样安卓会把它们解压到 App 的原生库目录——App 唯一允许执行程序的地方；前台服务用子进程启动它们。

**签名：** 不设置时用调试密钥签名，每次 CI 构建的密钥不同，升级前要先卸载旧版。想要稳定签名，在仓库 Secrets 里添加 `ANDROID_KEYSTORE_BASE64`（`.jks` 文件的 base64）、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`（可选 `ANDROID_KEY_PASSWORD`）。本地构建用环境变量 `VIVANT_KEYSTORE`、`VIVANT_KEYSTORE_PASSWORD`、`VIVANT_KEY_ALIAS`、`VIVANT_KEY_PASSWORD`。
