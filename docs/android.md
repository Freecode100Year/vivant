# 安卓手机当服务器

[← 返回 README](../README.md) · 简体中文 | [English](android.en.md)

把一台安卓手机变成 VIVANT 聊天室服务器：装上 APK，点「启动服务器」，同一 Wi-Fi 或连上这台手机热点的人就能用 SSH 进来。适合聚会、教室、断网环境、临时小群。手机里跑的是和 VPS 上完全相同的 Rust 服务器程序。

## 安装

1. 下载 `vivant-android.apk`：发布页（Releases），或 [Actions → Android](https://github.com/Freecode100Year/vivant/actions/workflows/android.yml) 最新一次成功构建的 Artifacts。
2. 在手机上打开 APK，按提示允许「安装未知来源应用」。
3. 需要 Android 8.0 及以上；支持 ARM64、ARMv7、x86_64。

## 使用

1. 打开 VIVANT，点 **启动服务器**。第一次会请求通知权限（用来显示常驻通知，可以拒绝，服务器照样运行）。
2. 建议点一次 **关闭电池优化**，否则部分手机（小米、华为、OPPO、vivo 等）会在锁屏后把服务器杀掉。国产系统还可能需要在系统设置里给 VIVANT 打开「自启动」「后台运行」。
3. 「怎么连接」里列出了这台手机的地址，其他设备运行其中一条即可：

   ```
   ssh -p 2222 guest@192.168.1.23
   ```

   iPhone / iPad 用 Termius、Blink 等 SSH App，填地址、端口 2222、用户名 `guest`，不填密码。
4. 页面上的 **主机指纹** 就是对方第一次连接时 SSH 显示的指纹，可以当面核对，确认连的是这台手机。

停止：点 **停止**，或在通知栏里点「停止」。勾选 **开机自动启动** 后，重启手机会自动开服。

## 网络说明

- **局域网 / 热点**：最可靠。手机开热点，别人连上热点后用热点地址（通常是 `192.168.x.1` 或 `10.x.x.x`）。
- **手机流量**：运营商一般不给公网 IPv4（CGNAT），外网连不进来。有公网 IPv6 时列表里会显示 IPv6 地址，对方也有 IPv6 才能连。
- 端口只能用 1024 以上（安卓普通 App 不能监听 22、443 这类端口）。

## 设置

| 设置 | 说明 |
| --- | --- |
| 端口 | 默认 2222。`端口 + 1` 会被用作本机内部的状态接口（只监听 127.0.0.1）。 |
| 最多同时在线 | 默认 100，手机性能足够撑起几百人，但电量和发热会上升。 |
| 公共房间 | 同服务器的 `-rooms`，例如 `lobby:大厅,tech:技术`；留空用默认房间。 |
| 额外参数 | 直接传给服务器的命令行参数，例如 `-history-ttl 24h -msg-rate 2`，全部参数见[部署与配置](deploy.md)。 |

## 隐私

和 VPS 版一样：没有账号、日志里没有 IP / 昵称 / 消息，聊天只在内存里，停止服务器就全部清空。App 不联网上报任何东西；手机上默认关闭了 Tor 出口列表下载（`-tor-exits=false`），省流量。主机密钥保存在 App 私有目录，卸载 App 会一起删除（下次就是新指纹）。

## 自己构建

需要 Rust、Android NDK（r25+）、JDK 17、Gradle 8.14+ 或 Android Studio：

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
ANDROID_NDK_HOME=/path/to/ndk android/build-native.sh     # 生成 android/app/src/main/jniLibs/*/libvivant.so
gradle -p android assembleRelease                          # 生成 android/app/build/outputs/apk/release/app-release.apk
```

服务器程序以 `libvivant.so` 的名字打进 APK（它其实是普通可执行文件），这样安卓会把它解压到 App 的原生库目录——App 唯一允许执行程序的地方；前台服务用子进程启动它。

**签名：** 不设置时用调试密钥签名，每次 CI 构建的密钥不同，升级前要先卸载旧版。想要稳定签名，在仓库 Secrets 里添加 `ANDROID_KEYSTORE_BASE64`（`.jks` 文件的 base64）、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`（可选 `ANDROID_KEY_PASSWORD`）。本地构建用环境变量 `VIVANT_KEYSTORE`、`VIVANT_KEYSTORE_PASSWORD`、`VIVANT_KEY_ALIAS`、`VIVANT_KEY_PASSWORD`。
