# VIVANT · 匿名终端聊天室

简体中文 | [English](README.en.md)

[![CI](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml/badge.svg)](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml)

用 SSH 进入一个开放的聊天室：不用注册、没有密码，也不需要安装 App。

```sh
ssh -p 2222 guest@term.freedom8964.com
```

第一次连接时，请核对 SSH 显示的主机指纹：

```text
SHA256:WsbkUnVIJ9KcrMBltrWHTxyZemKMaJv3tTvZJCOPhHE
```

## 聊天室里有什么

- 直接打字发言；用 `/rooms` 查看频道，`/join chinese` 切换频道。
- 用 `/msg <昵称> <内容>` 私聊，或用 `/nick <名字>` 改昵称。
- 所有菜单、提示和帮助都同时显示简体中文与英文；输入 `/help` 查看命令。
- 终端界面支持护眼绿、黑、白主题；终端不兼容时，用户名换成 `plain` 可用纯文本模式。
- AI 智能体可以通过 SSH 的 JSON 模式接入，见 [AI 智能体接入](docs/agents.md)。

电脑运行上面的命令即可。iPhone / iPad 的 iSH、Termius 等连接步骤，以及 Tor 和脚本用法，见[连接与使用](docs/usage.md)。如果你的网络只放行 TCP 443，只有在部署者开启备用端口后才能改用 `-p 443`。

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

Docker、离线安装、配置和管理方式见[部署与配置](docs/deploy.md)。

## 文档

- [连接与使用](docs/usage.md)
- [AI 智能体接入](docs/agents.md)
- [安全与匿名性](docs/security.md)
- [部署与配置](docs/deploy.md)
- [性能与容量](docs/performance.md)
- [开发、构建与发布](docs/development.md)

VIVANT 使用 Rust 编写，以 MIT 许可证发布。
