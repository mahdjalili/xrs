# xrs

**xrs** - an xray cli first ultra fast lightweight client.

Built in Rust with zero bloat. Operates as a fast, universal Linux CLI and interactive TUI, paired with a native top-bar drop-down widget and a systemd background service.

---

## Key Features

1. **CLI First & Ultra-Fast:**
   - Single 3.4 MB static binary with sub-millisecond execution and minimal RAM footprint (< 4 MB).
   - Machine-readable `--json` status output for scripts, bar widgets, and desktop panels.

2. **Systemd Background Service:**
   - Native user-level systemd service (`systemd --user`).
   - Automatically starts on login/boot and manages background lifecycle cleanly.
   - Commands: `xrs service install`, `xrs service start`, `xrs service stop`, `xrs service restart`, `xrs service status`.

3. **Full System-Level TUN Mode (L3 Routing):**
   - Configures the virtual network interface (`xrs-tun`) and applies policy routing tables (`table 1001`) with loop-prevention packet marks (`mark: 255`).
   - Run `xrs setup-tun` once to set `cap_net_admin` so TUN works without needing root password prompts.

4. **Manageable Routing Rules (Not Hardcoded):**
   - Create, enable, disable, and delete custom routing rules directly in CLI and TUI.
   - Includes preset installer for [chocolate4u/Iran-v2ray-rules](https://github.com/chocolate4u/Iran-v2ray-rules) (`geosite:ir`, `regexp:.*\.ir$`, `geoip:ir`, `geoip:private`).
   - Custom rules can be added via `xrs route add` or by editing `~/.config/xrs/routes.json`.

5. **Universal Linux TUI (`xrs` or `xrs tui`):**
   - Works on **any** Linux distribution (Arch, Fedora, Ubuntu, Debian, etc.).
   - Dynamically adapts to system and terminal colors, with clean terminal transparency fallback.
   - Keyboard controls:
     - `[Enter]`: Select active node & connect
     - `[Space]`: Toggle proxy on / off
     - `[T]`: Toggle full system **TUN mode**
     - `[R]`: Open **Routing Rules Manager** modal
     - `[P]`: Test latency (ping) to all nodes
     - `[U]`: Refresh all subscriptions
     - `[A]`: Add single config link modal
     - `[X]`: Stop daemon
     - `[Q]` or `[Esc]`: Quit TUI

6. **Top-Bar Widget Plugin (`plugin/`):**
   - Left-click icon `󰖂` to drop down the native panel:
     - ON / OFF `ToggleSwitch`
     - Active connection info and TUN mode toggle
     - One-click node switcher list
     - Quick buttons: `⚡ Ping`, `🔄 Update`, `🖥 TUI`
   - Right-click icon `󰖂` for instant connect/disconnect.
   - Colors and borders inherit dynamically from the active desktop theme.

---

## Quick Reference

```bash
# Launch interactive TUI
xrs

# View status & JSON output
xrs status
xrs status --json

# Start / Stop / Toggle proxy daemon
xrs start
xrs toggle
xrs stop

# Background Systemd Service
xrs service install      # Install and enable systemd user service
xrs service start        # Start service in background
xrs service stop         # Stop background service
xrs service restart      # Restart background service
xrs service status       # View systemd service status
xrs service uninstall    # Disable and remove systemd service

# TUN Mode
xrs setup-tun            # Grant network capability to xray binary (run once)
xrs tun on               # Enable TUN mode (full system-level VPN)
xrs tun off              # Disable TUN mode (revert to system proxy)
xrs tun status

# Routing Rules Management
xrs route list           # List all rules with [ENABLED] / [DISABLED]
xrs route toggle <name>  # Toggle rule on/off (e.g. xrs route toggle iran_bypass)
xrs route setup-iran     # Reinstall chocolate4u Iran routing preset
xrs route add "Work" --direct-domains "work.com,internal.net" --direct-ips "10.0.0.0/8"
xrs route remove <id>

# Proxy Nodes & Subscriptions
xrs node add "vless://..."     # Add single config link (vless, vmess, trojan, ss)
xrs node list                  # List nodes with latency
xrs node ping                  # Test ping to all nodes
xrs node select 1              # Select active node by index or ID
xrs node remove <id>           # Remove proxy node
xrs sub add "https://..."      # Add subscription URL
xrs sub update                 # Refresh subscriptions
```
