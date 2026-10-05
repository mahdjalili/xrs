# xrs

**The featherweight Xray client — one binary, your whole proxy, right in the terminal.**

[![Release](https://img.shields.io/github/v/release/mahdjalili/xrs)](https://github.com/mahdjalili/xrs/releases)
[![Build](https://github.com/mahdjalili/xrs/actions/workflows/build.yml/badge.svg?event=push)](https://github.com/mahdjalili/xrs/actions/workflows/build.yml)
[![License](https://img.shields.io/github/license/mahdjalili/xrs)](LICENSE)

![xrs terminal interface](assets/tui.png)

xrs is a proxy client built on Xray-core that lives in your terminal and sips memory. Connect, switch servers, and manage routing in seconds — no heavyweight GUI, no runtime bloat, nothing left running but the proxy itself.

## Why xrs

| | xrs | Typical GUI clients |
|---|---|---|
| Binary size | **~2.4 MB** (2,507 kB) | 100–300 MB |
| Idle memory (TUI, 200 servers) | **~4.6 MB** | 200–350 MB |
| Connected memory (client + core) | **~44 MB** | 133–387 MB |
| Idle CPU (TUI) | **~0.1%** | n/a |
| Status check (`xrs status`) | **~1.5 ms** | n/a |
| Dependencies | **None** (one binary, needs only glibc) | Qt / Electron runtimes |

<sub>xrs figures measured for v0.10.0 on 2026-10-04 on a cloud VM — Ubuntu 24.04.4 LTS, kernel 6.12.94+, 4 vCPU (Intel Xeon), 16 GB RAM, x86_64 — with the stripped release build, as the median of 5 runs per metric (the status check is the mean 1.57 ms / median 1.52 ms of 500 runs of `xrs status --json`). Memory is resident set size from `/proc/<pid>/smaps_rollup`: idle is the TUI open on a 200-server fixture; connected is the client (TUI open) plus the Xray-core 26.9.30 process it spawned, after three 20 MB downloads through the client's SOCKS port, connected to a loopback VLESS+WebSocket server with the default routing rules (geodata loaded). Idle CPU was sampled over 30 s the same way. The GUI-client columns are the earlier desktop measurements described below, not re-run on the VM. Reproduce with [`bench/bench.sh`](bench/bench.sh).</sub>

<details>
<summary><strong>Connected memory, client plus core</strong></summary>

The ~4.6 MB idle figure is the xrs TUI on its own. A connected session also runs Xray-core. The xrs column below was measured for v0.10.0 on the cloud VM described above, connected to a local VLESS + WebSocket server (the same Xray-core 26.9.30 binary acting as the server); Throne, Hiddify, and v2rayN are the earlier same-methodology measurements from a desktop Ubuntu 24.04 machine — not re-run on the VM — at the exact versions listed.

| | **xrs 0.10.0** | Throne 1.3.2 | Hiddify 4.1.1 | v2rayN 7.24.9 |
|---|---:|---:|---:|---:|
| Client | 4.7 MB | 69.2 MB | in-process | 316.2 MB |
| Proxy core | 38.9 MB | 64.1 MB | in-process | 70.6 MB |
| **Combined** | **43.7 MB** | 133.2 MB | 378.2 MB | 386.8 MB |

<sub>Resident set size after proxying 60 MB (three 20 MB downloads through the client's SOCKS port), summed over the client (TUI open, 200-server fixture) and the core process it spawned. Hiddify 4.1.1 loads hiddify-core inside the app process (`hiddify-core.so`), so the UI and the core are a single 387,288 KiB RSS. v2rayN 7.24.9 ran its bundled Xray 26.7.28 (72,248 KiB) beside the Avalonia UI (323,836 KiB). Throne 1.3.2 served the traffic with sing-box via ThroneCore (65,600 KiB) next to the Qt UI (70,840 KiB).</sub>

</details>

## Features

- **⚡ Connect in seconds:** pick a server, toggle a rule, flip full-tunnel mode — all from the terminal or a single keypress.
- **🖥️ A terminal UI that stays out of the way:** run `xrs` for tabs of servers, routing, and subscriptions; a filterable, sortable table with real through-proxy latency; a details pane; mouse support; and a `?` shortcut sheet. Nothing blocks — connecting, latency tests, and subscription syncs run in the background, and colors follow your terminal and desktop theme.
- **🔒 Full-tunnel TUN mode (Linux):** route the entire system through the proxy at the network layer, not just apps that respect proxy settings. The one-time setup runs itself; after that it's a single `T` keypress.
- **🧭 Routing rules you control:** bypass lists, ad & malware blocking, and custom domain/IP rules. Toggle any rule and xrs applies it for you. The optional Iran-routing preset (`xrs route setup-iran`) sends domestic traffic direct — installed only when you ask for it.
- **🔗 Links & subscriptions:** paste a single `vless://`, `vmess://`, `trojan://`, or `ss://` link, or point at a subscription URL and let it sync.
- **🔄 Runs in the background when you ask:** `xrs start` installs, enables, and starts a systemd user service, so the proxy keeps running after the terminal closes — across logins, and across reboots where lingering is enabled. `xrs stop` stops it, `xrs service uninstall` removes it. Opening the interactive TUI alone starts nothing.
- **🧩 Bar widget:** an optional native top-bar dropdown with an on/off switch, one-click server switching, and quick actions.

## Quick start

```bash
xrs                          # open the interactive interface
xrs node add "vless://..."   # add a server from a share link
xrs sub add "https://..."    # ...or add a whole subscription
xrs node select              # pick a server (interactive list)
xrs toggle                   # connect / disconnect
xrs tun on                   # full-system tunnel mode (Linux)
```

`xrs start` installs and starts the background systemd user service, so the proxy keeps running with no terminal open (on machines without systemd it falls back to spawning Xray directly); `xrs stop` stops it. Opening the interactive TUI does not start the service — connecting there runs the core directly.

## Install

Grab the tarball for your machine from the [latest release](https://github.com/mahdjalili/xrs/releases) — Linux: `linux-amd64` for most PCs, `linux-arm64` for ARM boards like Raspberry Pi; macOS: `macos-arm64` for Apple Silicon, `macos-amd64` for Intel Macs.

**Linux:**

```bash
tar xzf xrs-*-linux-amd64.tar.gz
install -Dm755 xrs-*-linux-amd64/xrs ~/.local/bin/xrs
xrs install-xray   # fetches the latest Xray engine + official geo data (first run only)
```

**macOS:**

```bash
tar xzf xrs-*-macos-arm64.tar.gz
mkdir -p ~/.local/bin && cp xrs-*-macos-arm64/xrs ~/.local/bin/xrs
xrs install-xray   # fetches the latest Xray engine + official geo data (first run only)
```

No root needed on Linux; everything lives in your home directory. TUN mode's one-time setup (file capabilities + passwordless sudo for routing) runs automatically the first time it starts.

macOS binaries are unsigned (no Apple developer certificate), so Gatekeeper may block the first launch — right-click → Open once, or clear the flag with `xattr -d com.apple.quarantine xrs`. Every release tarball is signed with Sigstore keyless (cosign), so downloads can still be verified.

## For developers

```bash
cargo build --release      # optimized binary at target/release/xrs
cargo clippy               # lint gate: no unwrap(), no unsafe code
```

Tagged `v*` pushes build signed Linux (`amd64` + `arm64`) and macOS (`amd64` + `arm64`) tarballs via GitHub Actions and publish them as releases. See [`.github/workflows/build.yml`](.github/workflows/build.yml).

## License

MIT, see [LICENSE](LICENSE).
