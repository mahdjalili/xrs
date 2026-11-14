<p align="center">
  <img src="assets/logo.png" alt="xrs logo" width="160" />
</p>

# xrs

**The featherweight Xray client — one binary, your whole proxy, right in the terminal.**

[![Release](https://img.shields.io/github/v/release/mahdjalili/xrs)](https://github.com/mahdjalili/xrs/releases)
[![Build](https://github.com/mahdjalili/xrs/actions/workflows/build.yml/badge.svg?event=push)](https://github.com/mahdjalili/xrs/actions/workflows/build.yml)
[![License](https://img.shields.io/github/license/mahdjalili/xrs)](LICENSE)

[Install](#install) · [Quick start](#quick-start) · [Features](#features) · [Omarchy plugin](#omarchy-plugin) · [Development](#for-developers)

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
- **🌐 DNS you control:** set remote (proxied) and direct resolvers with `xrs dns`, or press `e` on the Routing tab.
- **🔄 Runs in the background when you ask:** `xrs start` installs, enables, and starts a systemd user service, so the proxy keeps running after the terminal closes — across logins, and across reboots where lingering is enabled. `xrs stop` stops it, `xrs service uninstall` removes it. Opening the interactive TUI alone starts nothing.
- **🧩 Omarchy plugin:** control xrs from your desktop bar with [omaxrs](https://github.com/mahdjalili/omaxrs).

## Quick start

```bash
xrs                          # open the interactive interface
xrs node add "vless://..."   # add a server from a share link
xrs sub add "https://..."    # ...or add a whole subscription
xrs node select              # pick a server (interactive list)
xrs toggle                   # connect / disconnect
xrs tun on                   # full-system tunnel mode (Linux)
xrs dns set --direct 178.22.122.100  # customize direct DNS; xrs dns shows both resolvers
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

`xrs install-xray` fetches the latest published Xray-core release (including prereleases) for your OS and architecture: Linux or macOS, AMD64 or ARM64. It requires a successful GitHub release lookup—there is no pinned-version fallback—and validates the downloaded executable before replacing the installed core. Existing Iran-preset routing data is preserved.

macOS binaries are unsigned (no Apple developer certificate), so Gatekeeper may block the first launch — right-click → Open once, or clear the flag with `xattr -d com.apple.quarantine xrs`. Every release tarball is signed with Sigstore keyless (cosign), so downloads can still be verified.

## Omarchy plugin

[omaxrs](https://github.com/mahdjalili/omaxrs) is an optional Omarchy Quattro bar plugin for xrs, with connection controls and server switching. See its README for installation.

## For developers

SOCKS and TUN inbounds sniff HTTP Host and TLS SNI for routing with
`routeOnly: true`: the original connection destination is preserved. This
supports services whose SNI is an internal routing name (for example,
`kube-teleport-proxy-alpn.teleport.example.com`) rather than a resolvable
connection endpoint. A direct-domain rule alone cannot prevent destination
rewriting when `routeOnly` is disabled. TLS verification is unchanged.

Domain rules still select direct, proxy, or block outbounds. DNS interception,
resolver selection, and the configured routing `domainStrategy` are unchanged;
strategies such as `IPIfNonMatch` may still resolve sniffed names for IP rule
matching. Direct connections to IPs no longer resolve the sniffed hostname via
the freedom outbound; proxies receive the original IP or requested hostname.
This means clients need a usable original destination: remote DNS resolution
of sniffed names can no longer compensate for a poisoned or unreachable local
DNS answer. SOCKS clients can request a hostname when proxy-side resolution is
needed.

xrs does not currently configure FakeDNS pools or the `fakedns` sniffer, and its
DNS address validator does not accept the `fakedns` resolver. Future FakeDNS
support must restore synthetic IPs to domains rather than dial them. Xray
26.9.30 explicitly exempts FakeDNS results and recognized fake IPs from
`routeOnly` destination preservation; see the
[dispatcher implementation](https://github.com/XTLS/Xray-core/blob/v26.9.30/app/dispatcher/default.go)
and [sniffing reference](https://xtls.github.io/en/config/inbound.html#sniffingobject).

```bash
cargo build --release      # optimized binary at target/release/xrs
cargo clippy               # lint gate: no unwrap(), no unsafe code
cargo test --locked
# Optional loopback integration test (Python 3 + an existing Xray binary):
XRAY_TEST_BINARY=/path/to/xray cargo test --locked tls_sni_preserves_destination -- --ignored --nocapture
```

The optional test uses generated SOCKS settings and a real TLS ClientHello to
check domain routing and original destination preservation through direct and
SOCKS proxy outbounds, under `AsIs` and `IPIfNonMatch`. It does not complete a
TLS handshake or exercise privileged TUN networking. Generated SOCKS and TUN
sniffing settings are also covered by the ordinary unit tests.

Tagged `v*` pushes build signed Linux (`amd64` + `arm64`) and macOS (`amd64` + `arm64`) tarballs via GitHub Actions and publish them as releases. See [`.github/workflows/build.yml`](.github/workflows/build.yml).

## License

MIT, see [LICENSE](LICENSE).

### Linux TUN system DNS

TUN mode requires an active `systemd-resolved.service` and `resolvectl`.
XRS configures only the TUN link it creates, after Xray startup and policy
routing succeed: DNS destination `1.1.1.1`, routing domain `~.`, and
`DefaultRoute=yes`, disables DNS-over-TLS on that link so queries enter the
port-53 interception rule, then flushes resolved's cache. `1.1.1.1` is an interception
destination on port 53, not a replacement for the configured upstream:
Xray's `tun-in` port-53 rule precedes custom/private-IP bypass rules and
sends queries to `dns-out`. Xray's existing remote/direct DNS selection,
Iran preset, and custom routing rules remain in effect. There is no DNS
listener assumed at `172.19.0.1`.

Setup failure aborts the connection and attempts rollback. Disconnect and
service SIGTERM shutdown remove the core/TUN and flush the cache; a journal
in the XRS state directory permits cleanup after interrupted activation or
abnormal termination on the next start. Only a surviving TUN with the recorded
name and kernel interface index is reverted. Deleted links lose resolved's
per-link state automatically. Cleanup failures are reported and retained for
retry. Reconnects and the daemon's missing-interface recovery reapply setup.
A preexisting interface with the requested name is rejected to avoid taking
over another program's DNS configuration. One-time TUN privilege setup now
also covers `resolvectl`; failed sudo falls back to polkit elevation with a
bounded wait. Check sudo/polkit permissions when running headlessly.

XRS does not modify physical-link DNS, `/etc/resolv.conf`, or NetworkManager
profiles. The system resolver must already use resolved (for example via its
stub or NSS); merely running resolved does not prove all applications use it.
`~.` is a default DNS routing domain, **not complete leak prevention**:
more-specific corporate/split-DNS domains still win, and another link with
`~.` can receive queries too. Application-specific DoH/DoT and applications
using their own resolvers are outside this integration. SOCKS/HTTP-only mode
retains its existing behavior. macOS has no systemd-resolved and XRS's TUN
routing is Linux-specific; macOS TUN now reports this explicitly. Use proxy
mode there; no macOS resolver rewrite is attempted.

Real-machine verification (requires Linux, resolved, and a working node):

1. Connect with TUN enabled. Substitute the configured interface name below
   if it differs from `xrs-tun`.
2. Run `resolvectl status xrs-tun`. Confirm DNS `1.1.1.1`, domain `~.`, and
   default route enabled. Record `resolvectl status enp2s0` before/after and
   confirm XRS did not change physical-link DNS.
3. Run `resolvectl query --cache=no www.youtube.com` (if `--cache=no` is
   unsupported, run `sudo resolvectl flush-caches` before querying).
4. Confirm the query reports link `xrs-tun` and public addresses rather than
   the observed private filtering addresses. Also test an AAAA query on an
   IPv6-capable network. Inspect other routing domains if another link wins.
5. Run `curl --noproxy '*' -I https://www.youtube.com` with proxy environment
   variables unset; confirm a normal HTTPS request succeeds.
6. Test a configured direct domain (including the Iran preset if enabled)
   and inspect Xray routing with temporary debug logging if needed; confirm
   both DNS and the connection still take the intended direct path. Test a
   corporate split-DNS name separately; its more-specific link should win.
7. Disconnect. Confirm the TUN's resolved state disappears, the physical
   DNS is unchanged, and uncached queries use the original network resolver.
8. Reconnect, switch nodes, and run `systemctl --user restart xrs.service`;
   repeat steps 2–5. Stop the service and verify teardown. In a controlled
   test, terminate it abnormally and start it again to verify journal
   recovery. Test unavailable resolved and denied elevation: startup must
   report an error rather than claim working TUN DNS.
