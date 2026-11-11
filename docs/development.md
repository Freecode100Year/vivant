# 开发

[← 返回 README](../README.md) · 简体中文 | [English](development.en.md)

```bash
make test      # 单元测试（聊天逻辑、三种界面、SSH 层）
make lint      # rustfmt + clippy + shellcheck + 检查 unit 文件与安装脚本一致
make run       # 本地 127.0.0.1:2222
make dist      # 交叉编译 amd64 / arm64 / armv7（静态 musl，需要 cargo-zigbuild）+ SHA256SUMS
make bench     # 压测
```

用 Rust 编写（stable，1.85+），SSH 协议用 [russh](https://github.com/Eugeny/russh)，加密走 ring，不依赖 OpenSSL。v1.0 之前的版本是 Go 写的，命令行参数、环境变量、配置文件、主机密钥、封禁文件和 bot 协议都保持兼容，已安装的服务器直接 `vivant-ctl upgrade` 即可。

发布流程：修改 `VERSION` 文件（如 `1.0.1`）并推送到 main，或者推送 `vX.Y.Z` 标签，GitHub Actions 会自动测试、构建 Linux 三种架构（静态 musl）和 macOS / Windows 程序，发布 Release（附 `SHA256SUMS`）和 Docker 镜像。服务器上执行 `sudo vivant-ctl upgrade` 即可升级。每次推送 main 时，CI 会检查格式和 clippy、用 cargo audit 检查依赖有没有已公开的漏洞（发布前也会再查一次）、交叉编译所有平台，并在带 systemd 的 Ubuntu 上实际执行一遍安装脚本：从旧版 Private-Talk Terminal 迁移、安装、聊天、确认拒绝 exec、Tor、备用端口、封禁持久化、自检、改端口、卸载；另外构建并运行 Docker 镜像。

```
src/main.rs                 参数、健康检查、优雅退出
src/flags.rs                与 Go flag 包相同语法的参数解析（-addr :2222、-idle-timeout 30m）
install.sh                  一键安装 / 升级 / 卸载（也是 vivant-ctl）
src/chat/                   房间、身份、私聊、历史、限流、封禁、举报（与 SSH 无关）
src/ui/term.rs              完整界面：轻量终端渲染器
src/ui/keys.rs              按键解码（跨数据包的 UTF-8 和转义序列）
src/ui/plain.rs             纯文本行模式
src/ui/agent.rs             bot 模式：给 AI 智能体用的 JSON Lines
src/ui/format.rs            消息格式、时区、@提及
src/ui/style.rs             配色（WCAG AAA 对比度，测试强制检查）
src/server/                 SSH 网关、认证、连接限制、读取限速、握手超时、封禁持久化、Tor 出口列表
src/bin/vivant-bench.rs     压测工具
deploy/                     systemd 单元（由 install.sh --print-unit 生成）、配置示例、离线一键安装包的打包脚本
```

用户看得到的文字（聊天提示、`/help`、按键说明、安装脚本和自检输出、`vivant -h`）一律同时写简体中文和英文；源码注释保持英文。

## 路线图

- [x] SSH 无账号登录、随机昵称、公共频道、私人房间码、私聊、中文
- [x] 限流、管理员工具、举报、屏蔽、消息过期、权限隔离、systemd 沙箱
- [x] 一键安装 / 升级 / 卸载，CI，自动发布，压测
- [x] 普通用户不发送公钥，日志过滤地址，Tor 洋葱服务
- [x] 单核约 2000 名完整界面用户
- [x] `/privacy` 数据政策自查、`/forget` 删除自己的消息、禁止核心转储
- [x] 封禁持久化（可选）、`/bans` 与 `/unban`
- [x] 多房间未读提示、跨房间 @提及通知
- [x] 响铃、人多时折叠进出提示、备用 443 端口、主机指纹文档、Tor 出口共用限额
- [x] 改名 VIVANT；界面始终中英双语
- [x] `vivant-ctl doctor` 自检
- [x] 连接层读取限速、Docker 镜像（GHCR）
- [x] 握手超时（30 秒），知名端口提醒，外部监控文档
- [x] 用 Rust 重写（v1.0）：单核完整界面用户数、延迟和内存都明显改善，见[性能](performance.md)
- 不计划：在 SSH 聊天室本身里做端到端加密。这需要在用户设备上安装专用客户端，与“一条 ssh 命令就能进”的定位冲突。
