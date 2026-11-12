# VIVANT · 匿名终端聊天室

简体中文 | [English](README.en.md)

[![CI](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml/badge.svg)](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml)

一条 SSH 命令进入聊天室。不用注册、不用密码、不用装 App。

```
ssh -p 2222 guest@term.freedom8964.com
```

```
 VIVANT                         Alice · 在线 Online 3 · 运行 Up 4d 12h · 18:34 CST
房间 ROOMS ▸#general 3   #chinese 0 +2   #english 0   #technology 0   #anonymous 0
[18:32] <Alice> 大家晚上好！
[18:33] <Ghost> Hello from New York!
[18:33] <Anon42> 有人在学编程吗？@Alice
[18:34] [私信 DM ← Ghost] hi
── #general · 3 人在线 here · 大厅 · Lobby · 什么都可以聊 ─────────────────────────
> 说点什么… Type a message…  /help 帮助 help
Enter 发送 send · ↑↓ 历史 history · PgUp/PgDn 滚动 scroll · Ctrl+C 退出 quit   ● 已连接 CONNECTED
```

所有提示、帮助和按键说明都同时显示简体中文和英文。

## 怎么连

**网页入口**：打开 [https://chat.nya.je/](https://chat.nya.je/) 即可在浏览器中进入聊天室。

**电脑**（Windows / macOS / Linux 自带 ssh）：运行上面那条命令。第一次连接会问是否信任服务器，输入 `yes`。

**iPhone / iPad**：

- **iSH Shell**：App Store 免费下载，先运行 `apk add openssh-client`，再运行上面的命令。
- **Termius**：新建主机，地址 `term.freedom8964.com`，端口 `2222`，用户名 `guest`，密码留空。

网络只放行 443 时，把 `-p 2222` 换成 `-p 443`（需部署者开启）。用户名随便填，没有密码。屏幕太窄或显示乱码时，把用户名换成 `plain`，进入纯文本模式。更多连接方式（Tor、脚本、免确认指纹）见[连接与使用](docs/usage.md)；接入 AI 智能体见 [AI 智能体接入](docs/agents.md)。

## 常用命令

直接打字就是发言。

```
/rooms               有哪些房间
/join chinese        进入房间
/msg <昵称> <内容>   私聊
/nick <名字>         改昵称
/who                 谁在这里
/help all            全部命令
```

## 隐私

- 没有账号；你的 SSH 公钥不会发给服务器；日志里没有 IP、昵称和聊天内容；消息只在内存里，过期自动删除。
- **服务器能看到聊天内容**，别发能认出你的信息。在聊天室里输入 `/privacy` 可以查看这台服务器存什么、不存什么，`/forget` 删除你发过的消息。
- 想连 IP 也不让服务器看到：通过 Tor 连接（需要部署者开启）。

详细说明见[安全与匿名性](docs/security.md)。

## 部署自己的聊天室

在一台 Linux VPS 上运行：

```bash
curl -fsSL https://raw.githubusercontent.com/Freecode100Year/vivant/main/install.sh | sudo bash
```

装好后用 `sudo vivant-ctl status | upgrade | uninstall` 管理；连不上时运行 `sudo vivant-ctl doctor`，它会逐项检查并告诉你怎么修。别忘了在云厂商的防火墙里放行 TCP 2222；域名在 Cloudflare 上要设为 DNS only（灰云）。也可以用 Docker 部署，见[部署与配置](docs/deploy.md#docker可选)。

**服务器连不上 GitHub：** 下载发布页里的离线一键安装包，在自己电脑上运行 `bash vivant-<版本>-linux-offline.sh --to root@服务器` 即可部署，见[离线安装](docs/deploy.md#离线安装服务器连不上-github)。想在 Windows / macOS 上临时跑一个（测试、局域网），发布页也有对应的程序，见[部署与配置](docs/deploy.md#windows--macos)。

**用安卓手机当服务器：** 装上 `vivant-android-arm64.apk`，点一下「启动」，同一 Wi-Fi 或手机热点下的人就能连进来；还能用 EasyTier 或 FRP 让外网的人进来，见[安卓手机当服务器](docs/android.md)。

**从 Private-Talk Terminal 升级：** VIVANT 以前叫 Private-Talk Terminal（`ptterm`）。在旧服务器上运行一次上面的命令即可自动迁移，主机密钥、配置、封禁、洋葱地址都会保留，详见[部署与配置](docs/deploy.md#从-private-talk-terminal-迁移)。

单核 1GB 的 VPS 大约能承载 2000 人同时在线。安装参数、管理员、配置项见[部署与配置](docs/deploy.md)。

## 文档

- [连接与使用](docs/usage.md)：各平台连接方法、iPhone 详细步骤、全部命令、Tor
- [AI 智能体接入](docs/agents.md)：`bot@` JSON 模式、示例程序
- [安全与匿名性](docs/security.md)：部署者能看到什么、不能保证什么
- [安卓手机当服务器](docs/android.md)：APK 安装、热点 / 局域网连接、自己构建
- [部署与配置](docs/deploy.md)：一键安装参数、迁移、管理员、配置项、Docker
- [性能](docs/performance.md)：压测数据和设计
- [开发](docs/development.md)：构建、测试、发布、代码结构、路线图

Rust 编写，单个静态二进制，无数据库，MIT 许可。
