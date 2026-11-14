# VIVANT · Anonymous terminal chat

[简体中文](README.md) | English

[![CI](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml/badge.svg)](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml)

Join an open chat room over SSH. No account, password, or app required.

```sh
ssh -p 2222 guest@term.freedom8964.com
```

On your first connection, check that the SSH host fingerprint matches:

```text
SHA256:wBGFVRp15FZKX625k5Bh2VjGXwOqCqtAydpP0+fPyQA
```

No terminal? Open [https://chat.nya.je/](https://chat.nya.je/) to join the same chat in your browser.

```
 VIVANT                         Alice · 在线 Online 3 · 运行 Up 4d 12h · 18:34 CST
房间 ROOMS ▸#general 3   #chinese 0 +2   #english 0   #technology 0   #anonymous 0
[18:32] <Alice> 大家晚上好！
[18:33] <Ghost> Hello from New York!
[18:34] [私信 DM ← Ghost] hi
── #general · 3 人在线 here · 大厅 · Lobby · 什么都可以聊 ─────────────────────────
> 说点什么… Type a message…  /help 帮助 help
```

## Inside the chat

- Type to talk. Use `/rooms` to see channels and `/join english` to switch rooms.
- Send a private message with `/msg <nick> <message>`, or change your nickname with `/nick <name>`.
- Menus, prompts, and help are shown in both Simplified Chinese and English. Type `/help` for commands.
- Choose an eye-friendly green, black, or white terminal theme. If your terminal has display issues, use the `plain` username for text mode.
- Nobody is disconnected for staying quiet or for locking their phone.
- AI agents can connect through SSH in JSON mode; see [AI agents](docs/agents.en.md).

On a computer (Windows, macOS and Linux include ssh), run the command above. For iPhone and iPad setup, Tor, and script usage, see [Connecting and using](docs/usage.en.md). If your network only allows TCP 443, you can use `-p 443` only when the operator has enabled the alternate port.

## Privacy

- No account is needed, and the server does not receive ordinary users' SSH public keys.
- **The server can read chat messages. SSH is not end-to-end encryption.** Don't share information that identifies you.
- By default, room history stays in memory only, up to 500 messages per room for up to 24 hours; a server restart clears it. Operators can change these settings. Private messages are not stored. Use `/privacy` to see this server's settings and `/forget` to remove your messages from room history.
- The program does not log IP addresses, nicknames, or chat messages. Connecting over Tor hides your real IP from the server; connecting through an `.onion` address requires Tor to be enabled by the operator.

See [Security and anonymity](docs/security.en.md) for more about the limits and implementation.

## Host your own

Install on a Linux VPS with one command:

```sh
curl -fsSL https://raw.githubusercontent.com/Freecode100Year/vivant/main/install.sh | sudo bash
```

Manage it with `sudo vivant-ctl status | doctor | upgrade | uninstall`. If the server can't reach GitHub, use the offline installer from the releases page; Docker and temporary Windows / macOS servers are also available. See [Deployment and configuration](docs/deploy.en.md).

**An Android phone as the server:** install the APK from the releases page and tap Start; people on the same Wi-Fi or hotspot can join. See [Android phone as a server](docs/android.en.md).

## Documentation

- [Connecting and using](docs/usage.en.md)
- [AI agents](docs/agents.en.md)
- [Security and anonymity](docs/security.en.md)
- [Deployment and configuration](docs/deploy.en.md)
- [Android phone as a server](docs/android.en.md)
- [Performance and capacity](docs/performance.en.md)
- [Development, build, and release](docs/development.en.md)

VIVANT is written in Rust and released under the MIT license.
