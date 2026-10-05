#!/usr/bin/env bash
# README benchmark: measures every metric the comparison tables claim, against
# the current checkout's release build, and prints medians.
#
#   bench/bench.sh [RUNS]      # runs per metric, default 5; status check: 500
#
# Protocol (mirrors the methodology note in the README):
#   - binary size          stat of target/release/xrs
#   - idle memory          TUI open with 200 servers; RSS from /proc/<pid>/smaps_rollup
#   - idle CPU             same TUI; utime+stime deltas over a 30 s window
#   - connected memory     TUI + proxy core after three 20 MB downloads through
#                          the client's SOCKS port (loopback VLESS+WS server and
#                          loopback HTTP payload, so the tunnel is traversed)
#   - status check         500 runs of `xrs status --json`
#   - connect time         spawn -> SOCKS port accepting (reference only)
#
# Reports the median of RUNS for each metric (mean for the status check, the
# hyperfine-style figure the table quotes) and median for reference.
#
# Requires: python3, tmux, curl, an xray core at ~/.local/share/xrs/xray
# (create it with `xrs install-xray`). The script temporarily replaces the
# xrs config under ~/.config/xrs (a backup is restored on exit).
set -euo pipefail
cd "$(dirname "$0")/.."

RUNS="${1:-5}"
STATUS_RUNS=500
BIN=target/release/xrs
XRAY="${XRAY:-$HOME/.local/share/xrs/xray}"
UUID=8f3f4a2b-7c1d-4e5f-9a0b-1c2d3e4f5a6b
SRV_PORT=10001
HTTP_PORT=9999
CONFIG_DIR="$HOME/.config/xrs"
CONFIG="$CONFIG_DIR/config.json"
WORK="$(mktemp -d)"
SUMMARY="$WORK/summary.txt"

cleanup() {
  tmux kill-session -t bench-xrs 2>/dev/null || true
  pkill -x xray 2>/dev/null || true
  pkill -x xrs 2>/dev/null || true
  [[ -n "${HTTP_PID:-}" ]] && kill "$HTTP_PID" 2>/dev/null || true
  [[ -n "${SRV_PID:-}" ]] && kill "$SRV_PID" 2>/dev/null || true
  if [[ -f "$WORK/config.backup" ]]; then
    mkdir -p "$CONFIG_DIR"
    mv "$WORK/config.backup" "$CONFIG"
  fi
}
trap cleanup EXIT

[[ -x "$BIN" ]] || { echo "building release binary..."; cargo build --release --quiet; }
[[ -x "$XRAY" ]] || { echo "xray core missing at $XRAY — run 'xrs install-xray' first" >&2; exit 1; }
command -v tmux >/dev/null || { echo "tmux is required for the TUI metrics" >&2; exit 1; }
command -v curl >/dev/null || { echo "curl is required" >&2; exit 1; }

record() { echo "$1" >> "$SUMMARY"; }

# --- fixtures -------------------------------------------------------------

mkdir -p "$CONFIG_DIR"
if [[ -f "$CONFIG" ]]; then
  cp "$CONFIG" "$WORK/config.backup"
fi
pkill -x xray 2>/dev/null || true
pkill -x xrs 2>/dev/null || true
sleep 0.3

# 200 dummy servers for the TUI metrics (they are held in memory, not dialed).
python3 - "$CONFIG" <<'PY'
import json, sys
nodes = []
for i in range(200):
    nodes.append({
        "id": f"bench{i:03d}", "name": f"bench-{i:03d}", "protocol": "Vless",
        "server": "127.0.0.1", "port": 40000 + i, "secret": "x" * 36,
        "network": "tcp", "security": "none", "raw_link": f"vless://x@127.0.0.1:4000{i}",
    })
cfg = {
    "inbounds": {"socks_port": 10808, "http_port": 10809, "listen": "127.0.0.1"},
    "tun": {"enabled": False},
    # routing omitted: the default rules (AdBlock, geodata loaded) apply
    "active_node_id": nodes[0]["id"], "subscriptions": [], "nodes": nodes,
}
open(sys.argv[1], "w").write(json.dumps(cfg, indent=2))
PY

# Loopback proxy server: VLESS over WebSocket -> freedom. A custom routes.json
# without the geoip:private direct rule keeps loopback traffic inside the tunnel.
# Newer Xray cores blackhole private/loopback targets from VLESS inbounds by
# default, so finalRules explicitly allows the payload server only.
cat > "$WORK/server.json" <<EOF
{"log":{"loglevel":"warning"},"inbounds":[{"tag":"vless-in","port":$SRV_PORT,"listen":"127.0.0.1","protocol":"vless","settings":{"clients":[{"id":"$UUID","level":0}],"decryption":"none"},"streamSettings":{"network":"ws","wsSettings":{"path":"/ws"}}}],"outbounds":[{"tag":"direct","protocol":"freedom","settings":{"finalRules":[{"action":"allow","network":"tcp","ip":["127.0.0.1"],"port":"$HTTP_PORT"}]}}]}
EOF
mkdir -p "$CONFIG_DIR"
echo '{"direct":{"domains":[],"ips":[]},"proxy":{"domains":[],"ips":[]},"block":{"domains":[],"ips":[]}}' > "$CONFIG_DIR/routes.json"

head -c $((20 * 1024 * 1024)) /dev/urandom > "$WORK/payload.bin"
python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$WORK" >/dev/null 2>&1 &
HTTP_PID=$!
"$XRAY" run -c "$WORK/server.json" >/dev/null 2>&1 &
SRV_PID=$!
sleep 1

rss_kb() { awk '/^Rss:/ {print $2}' "/proc/$1/smaps_rollup" 2>/dev/null || echo 0; }
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }
clock_tck() { getconf CLK_TCK; }

# --- binary size ----------------------------------------------------------

size_kb=$(( $(stat -c %s "$BIN") / 1024 ))
record "binary_size_kb $size_kb"
echo "binary size: ${size_kb} kB"

# --- status check ---------------------------------------------------------

python3 - "$BIN" "$STATUS_RUNS" <<'PY'
import statistics, subprocess, sys, time
bin, n = sys.argv[1], int(sys.argv[2])
times = []
for _ in range(n):
    t0 = time.perf_counter()
    subprocess.run([bin, "status", "--json"], stdout=subprocess.DEVNULL, check=False)
    times.append((time.perf_counter() - t0) * 1000)
print(f"status_check_ms mean {statistics.mean(times):.2f} median {statistics.median(times):.2f} n {n}")
PY

# --- TUI idle memory + CPU (200 servers) ----------------------------------

for run in $(seq 1 "$RUNS"); do
  pkill -x xrs 2>/dev/null || true; sleep 0.3
  tmux new-session -d -s bench-xrs "$BIN"
  sleep 2
  TUI_PID="$(pgrep -n -x xrs)"
  sleep 8                       # settle: render, theme detection, table build
  record "idle_mem_kb $(rss_kb "$TUI_PID")"
  C0=$(cpu_ticks "$TUI_PID"); sleep 30; C1=$(cpu_ticks "$TUI_PID")
  CPU=$(python3 -c "print(f'{($C1 - $C0) / $(clock_tck) / 30 * 100:.3f}')")
  record "idle_cpu_pct $CPU"
  tmux kill-session -t bench-xrs 2>/dev/null || true
  pkill -x xrs 2>/dev/null || true
done

# --- connected memory + connect time --------------------------------------

# The bench node: a real VLESS+WS link to the loopback server (parsed by xrs).
echo | "$BIN" node add "vless://$UUID@127.0.0.1:$SRV_PORT?type=ws&path=%2Fws#bench" >/dev/null
# Point the active node at it: the dummy fixture secrets are not valid UUIDs
# and would abort the core at startup.
python3 - "$CONFIG" "$SRV_PORT" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
node = next(n for n in cfg["nodes"] if n["port"] == int(sys.argv[2]))
cfg["active_node_id"] = node["id"]
open(sys.argv[1], "w").write(json.dumps(cfg, indent=2))
PY

for run in $(seq 1 "$RUNS"); do
  # Only the client core is reset here (its pid file); the loopback VLESS
  # server is also named "xray" and must survive across runs.
  "$BIN" stop >/dev/null 2>&1 || true
  pkill -x xrs 2>/dev/null || true; sleep 0.3
  tmux new-session -d -s bench-xrs "$BIN"
  sleep 2
  T0=$(python3 -c 'import time; print(time.perf_counter())')
  "$BIN" start >/dev/null 2>&1
  until python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.05); sys.exit(0 if s.connect_ex(('127.0.0.1', 10808)) == 0 else 1)" 2>/dev/null; do
    sleep 0.02
  done
  CT=$(python3 -c "import time; print(f'{(time.perf_counter() - $T0) * 1000:.1f}')")
  record "connect_ms $CT"
  CORE_PID="$(cat "$HOME/.local/share/xrs/xray.pid")"
  TUI_PID="$(pgrep -n -x xrs)"
  for _ in 1 2 3; do
    curl -s --socks5-hostname 127.0.0.1:10808 --max-time 60 -o /dev/null "http://127.0.0.1:$HTTP_PORT/payload.bin"
  done
  record "client_mem_kb $(rss_kb "$TUI_PID")"
  record "core_mem_kb $(rss_kb "$CORE_PID")"
  record "combined_mem_kb $(( $(rss_kb "$TUI_PID") + $(rss_kb "$CORE_PID") ))"
  tmux kill-session -t bench-xrs 2>/dev/null || true
  pkill -x xrs 2>/dev/null || true
  "$BIN" stop >/dev/null 2>&1 || true
done

# --- medians ---------------------------------------------------------------

python3 - "$SUMMARY" <<'PY'
import statistics, sys
series: dict[str, list[float]] = {}
for line in open(sys.argv[1]):
    parts = line.split()
    if len(parts) >= 2:
        series.setdefault(parts[0], []).append(float(parts[1]))
print(f"\n=== medians over the recorded runs ===")
for name, values in series.items():
    mid = statistics.median(values)
    print(f"{name:18} median {mid:10.2f}   mean {statistics.mean(values):10.2f}   n {len(values)}")
PY
