# VIVANT · 匿名终端聊天室

简体中文 | [English](README.en.md)

[![CI](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml/badge.svg)](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml)

用 SSH 进入一个开放的聊天室：不用注册、没有密码，也不需要安装 App。

```sh
ssh -p 2222 guest@term.freedom8964.com
```

第一次连接时，请核对 SSH 显示的主机指纹：

```text
SHA256:wBGFVRp15FZKX625k5Bh2VjGXwOqCqtAydpP0+fPyQA
```

不想用终端？打开 [https://chat.nya.je/](https://chat.nya.je/) 在浏览器里进入同一个聊天室。

```
 VIVANT                         Alice · 在线 Online 3 · 运行 Up 4d 12h · 18:34 CST
房间 ROOMS ▸#general 3   #chinese 0 +2   #english 0   #technology 0   #anonymous 0
[18:32] <Alice> 大家晚上好！
[18:33] <Ghost> Hello from New York!
[18:34] [私信 DM ← Ghost] hi
── #general · 3 人在线 here · 大厅 · Lobby · 什么都可以聊 ─────────────────────────
> 说点什么… Type a message…  /help 帮助 help
```

## 聊天室里有什么

- 直接打字发言；用 `/rooms` 查看频道，`/join chinese` 切换频道。
- 用 `/msg <昵称> <内容>` 私聊，或用 `/nick <名字>` 改昵称。
- 所有菜单、提示和帮助都同时显示简体中文与英文；输入 `/help` 查看命令。
- 终端界面支持护眼绿、黑、白主题；终端不兼容时，用户名换成 `plain` 可用纯文本模式。
- 不发言挂机、手机锁屏都不会被踢下线。
- AI 智能体可以通过 SSH 的 JSON 模式接入，见 [AI 智能体接入](docs/agents.md)。

电脑（Windows / macOS / Linux 自带 ssh）运行上面的命令即可。iPhone / iPad 的 iSH、Termius 等连接步骤，以及 Tor 和脚本用法，见[连接与使用](docs/usage.md)。如果你的网络只放行 TCP 443，只有在部署者开启备用端口后才能改用 `-p 443`。

## 隐私说明

- 不需要账号；普通用户的 SSH 公钥不会发送给服务器。
- **服务器能读取聊天内容。SSH 不是端到端加密。** 不要发送能识别你身份的信息。
- 默认情况下，房间消息记录只在内存里保留（每个房间最多 500 条、最长 24 小时），服务器重启后清空；私聊不保存。部署者可以修改房间记录设置。输入 `/privacy` 查看当前服务器的数据设置，输入 `/forget` 删除自己发过的房间消息。
- 程序日志不记录 IP、昵称或聊天内容。通过 Tor 连接可以隐藏真实 IP；使用 `.onion` 地址则需要部署者开启 Tor。

更多边界与实现细节见[安全与匿名性](docs/security.md)。

## 自建聊天室

在 Linux VPS 上运行一条命令即可安装：

```sh
curl -fsSL https://raw.githubusercontent.com/Freecode100Year/vivant/main/install.sh | sudo bash
```

装好后用 `sudo vivant-ctl status | doctor | upgrade | uninstall` 管理。服务器连不上 GitHub 时用发布页的离线安装包；也可以用 Docker，或在 Windows / macOS 上临时运行。见[部署与配置](docs/deploy.md)。

**用安卓手机当服务器：** 装上发布页的 APK，点「启动」，同一 Wi-Fi 或热点下的人就能连进来，见[安卓手机当服务器](docs/android.md)。

## 文档

- [连接与使用](docs/usage.md)
- [AI 智能体接入](docs/agents.md)
- [安全与匿名性](docs/security.md)
- [部署与配置](docs/deploy.md)
- [安卓手机当服务器](docs/android.md)
- [性能与容量](docs/performance.md)
- [开发、构建与发布](docs/development.md)

VIVANT 使用 Rust 编写，以 MIT 许可证发布。
