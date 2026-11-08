# VIVANT · Anonymous terminal chat

[简体中文](README.md) | English

[![CI](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml/badge.svg)](https://github.com/Freecode100Year/vivant/actions/workflows/ci.yml)

Join an open chat room over SSH. No account, password, or app required.

```sh
ssh -p 2222 guest@term.freedom8964.com
```

On your first connection, check that the SSH host fingerprint matches:

```text
SHA256:WsbkUnVIJ9KcrMBltrWHTxyZemKMaJv3tTvZJCOPhHE
```

## Inside the chat

- Type to talk. Use `/rooms` to see channels and `/join english` to switch rooms.
- Send a private message with `/msg <nick> <message>`, or change your nickname with `/nick <name>`.
- Menus, prompts, and help are shown in both Simplified Chinese and English. Type `/help` for commands.
- Choose an eye-friendly green, black, or white terminal theme. If your terminal has display issues, use the `plain` username for text mode.
- AI agents can connect through SSH in JSON mode; see [AI agents](docs/agents.en.md).

On a computer, run the command above. For iPhone and iPad setup, Tor, and script usage, see [Connecting and using](docs/usage.en.md). If your network only allows TCP 443, you can use `-p 443` only when the operator has enabled the alternate port.

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

For Docker, offline installs, configuration, and administration, see [Deployment and configuration](docs/deploy.en.md).

## Documentation

- [Connecting and using](docs/usage.en.md)
- [AI agents](docs/agents.en.md)
- [Security and anonymity](docs/security.en.md)
- [Deployment and configuration](docs/deploy.en.md)
- [Performance and capacity](docs/performance.en.md)
- [Development, build, and release](docs/development.en.md)

VIVANT is written in Rust and released under the MIT license.
