# xrs

**The featherweight Xray client. One binary, zero bloat, total control.**

[![Release](https://img.shields.io/github/v/release/mahdjalili/xrs)](https://github.com/mahdjalili/xrs/releases)
[![License](https://img.shields.io/github/license/mahdjalili/xrs)](LICENSE)

![xrs terminal interface](assets/tui.png)

xrs is a blazing-fast proxy client built on Xray-core. It lives in your terminal, sips memory, and gets out of your way: connect, switch servers, and manage routing without ever touching a heavy GUI app.

## Why xrs

| | xrs | Typical GUI clients |
|---|---|---|
| Binary size | **~3.4 MB** | 100–300 MB |
| Idle memory (TUI) | **~5 MB** | 200–350 MB |
| Status check | **~4 ms** | n/a |
| Dependencies | **None** (single static binary) | Qt / Electron runtimes |

## Features

- **⚡ Instant everything:** connect, switch nodes, and toggle the proxy in milliseconds, from the terminal or a single keypress.
- **🖥️ Beautiful terminal UI:** just run `xrs`. Tabs for servers, routing and subscriptions, a filterable and sortable server table with live latency, a details pane, mouse support, and a `?` shortcut sheet. Nothing blocks: connecting, latency tests and subscription syncs run in the background. It picks up your terminal and desktop theme colors automatically.
- **🔒 Full-tunnel TUN mode:** route your entire system through the proxy at the network layer, not just apps that respect proxy settings. One-time setup, then a single `T` keypress.
- **🧭 Routing rules you control:** bypass lists, ad & malware blocking, and custom domain/IP rules. Ships with an Iran-routing preset (domestic traffic goes direct, zero proxy lag). Toggle any rule live, no restarts to configure.
- **🔗 Links & subscriptions:** paste a single `vless://`, `vmess://`, `trojan://` or `ss://` link, or plug in a subscription URL with auto-refresh.
- **🔄 Runs in the background:** optional systemd service keeps you connected across logins.
- **🧩 Bar widget:** an optional native top-bar dropdown with an on/off switch, one-click server switching, and quick actions.

## Install

Grab the tarball for your machine from the [latest release](https://github.com/mahdjalili/xrs/releases) (`linux-amd64` for most PCs, `linux-arm64` for ARM boards like Raspberry Pi):

```bash
tar xzf xrs-*-linux-amd64.tar.gz
install -Dm755 xrs-*-linux-amd64/xrs ~/.local/bin/xrs
xrs install-xray   # fetches the Xray engine + routing data (first run only)
xrs setup-tun      # one-time setup for TUN mode (asks for password once)
```

No root needed. Everything lives in your home directory.

## Quick start

```bash
xrs                          # open the interactive interface
xrs node add "vless://..."   # add a server from a share link
xrs sub add "https://..."    # ...or add a whole subscription
xrs node select              # pick a server (interactive list)
xrs toggle                   # connect / disconnect
xrs tun on                   # full-system tunnel mode
```

## For developers

```bash
cargo build --release      # optimized binary at target/release/xrs
cargo clippy               # lint gate: no unwrap(), no unsafe code
```

Tagged `v*` pushes build signed Linux tarballs (`amd64` + `arm64`) via GitHub Actions and publish them as releases. See [`.github/workflows/build.yml`](.github/workflows/build.yml).

## License

MIT, see [LICENSE](LICENSE).
