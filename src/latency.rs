use crate::model::ProxyNode;
use crate::storage::get_data_dir;
use crate::xray::{find_xray_binary, generate_probe_config};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a real (through-proxy) probe may take end-to-end.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP endpoint fetched through each node. A 204/empty response is enough;
/// what matters is that the request traversed the proxy path.
const PROBE_HOST: &str = "www.gstatic.com";
const PROBE_PORT: u16 = 80;
const PROBE_PATH: &str = "/generate_204";

/// How probe sockets leave the machine when doing a plain TCP fallback.
///
/// While TUN mode is up, every unmarked socket is policy-routed into the
/// tunnel, where Xray's userspace stack completes the TCP handshake locally
/// and every server appears to answer in ~0 ms. Binding the probe to the
/// physical uplink makes the tunnel table's `default dev <tun>` route miss
/// (its output interface no longer matches), so the kernel falls through to
/// the main table. Unlike `SO_MARK`, `SO_BINDTODEVICE` needs no capability on
/// Linux 5.7+.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeRoute {
    bind_device: Option<String>,
}

impl ProbeRoute {
    pub fn detect(tun_name: &str) -> Self {
        let tun_up = !tun_name.is_empty() && Path::new("/sys/class/net").join(tun_name).exists();
        if !tun_up {
            return Self::default();
        }
        let bind_device = fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|table| default_uplink(&table, tun_name));
        if bind_device.is_none() {
            log::warn!("TUN is up but no physical default route was found; latency may read ~0 ms");
        }
        Self { bind_device }
    }

    /// TCP handshake time to `host:port`. DNS resolution is excluded so the
    /// number reflects the network path rather than resolver speed.
    pub fn tcp_latency(&self, host: &str, port: u16) -> Option<u64> {
        let addr = resolve(host, port)?;
        let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP)).ok()?;
        // Only IPv4 is policy-routed into the tunnel, so v6 sockets stay unbound.
        if let (Some(dev), SocketAddr::V4(_)) = (&self.bind_device, addr) {
            bind_to_device(&socket, dev);
        }
        let target = SockAddr::from(addr);
        let start = Instant::now();
        socket.connect_timeout(&target, PROBE_TIMEOUT).ok()?;
        Some(start.elapsed().as_millis() as u64)
    }
}

/// SO_BINDTODEVICE is a Linux socket option; socket2 exposes `bind_device`
/// only there, and TUN mode (the reason probes bind to the uplink) is
/// Linux-only, so other platforms keep plain sockets.
#[cfg(target_os = "linux")]
fn bind_to_device(socket: &Socket, dev: &str) {
    if let Err(e) = socket.bind_device(Some(dev.as_bytes())) {
        log::debug!("SO_BINDTODEVICE {dev} failed: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
fn bind_to_device(_socket: &Socket, _dev: &str) {}

/// Prefer real through-proxy latency; fall back to a TUN-aware TCP handshake
/// when the Xray binary is missing so the TUI still has a signal.
pub fn measure(node: &ProxyNode, tun_name: &str) -> Option<u64> {
    if find_xray_binary().is_some() {
        real_latency(node, tun_name)
    } else {
        log::debug!("xray binary missing; falling back to TCP latency for {}", node.name);
        ProbeRoute::detect(tun_name).tcp_latency(&node.server, node.port)
    }
}

/// Real latency: stand up a throwaway Xray instance for `node`, then time an
/// HTTP GET through its local SOCKS inbound to a well-known URL. That measures
/// the full proxy path (dial + handshake + first response), not just a TCP
/// connect to the server port.
///
/// When TUN is up the probe outbound is fwmark'd so it leaves via the physical
/// uplink instead of looping into the tunnel.
pub fn real_latency(node: &ProxyNode, tun_name: &str) -> Option<u64> {
    let xray_bin = find_xray_binary()?;
    let _ = fs::create_dir_all(get_data_dir());
    let tun_up = !tun_name.is_empty() && Path::new("/sys/class/net").join(tun_name).exists();
    let port = free_tcp_port()?;
    let cfg_path = probe_config_path(port);
    let config = generate_probe_config(node, port, tun_up);
    let content = serde_json::to_string(&config).ok()?;
    fs::write(&cfg_path, content).ok()?;

    let mut child = match spawn_probe(&xray_bin, &cfg_path) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("probe xray spawn failed: {e}");
            let _ = fs::remove_file(&cfg_path);
            return None;
        }
    };

    let socks = SocketAddr::from(([127, 0, 0, 1], port));
    let ready = wait_for_port(socks, Duration::from_secs(2));
    let ms = if ready {
        http_via_socks5(socks, PROBE_HOST, PROBE_PORT, PROBE_PATH, PROBE_TIMEOUT)
    } else {
        log::debug!("probe xray on :{port} never became ready");
        None
    };

    stop_probe(&mut child);
    let _ = fs::remove_file(&cfg_path);
    ms
}

fn probe_config_path(port: u16) -> PathBuf {
    get_data_dir().join(format!("xray_probe_{port}.json"))
}

fn spawn_probe(xray_bin: &Path, cfg_path: &Path) -> Result<Child, String> {
    let asset_dir = get_data_dir();
    let log = fs::File::create(asset_dir.join("xray_probe.log")).map_err(|e| e.to_string())?;
    let stderr = log.try_clone().map_err(|e| e.to_string())?;
    Command::new(xray_bin)
        .arg("run")
        .arg("-c")
        .arg(cfg_path)
        .env("XRAY_LOCATION_ASSET", &asset_dir)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| e.to_string())
}

fn stop_probe(child: &mut Child) {
    let pid = child.id().to_string();
    let _ = Command::new("kill").args(["-TERM", &pid]).status();
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = Command::new("kill").args(["-KILL", &pid]).status();
                let _ = child.wait();
                break;
            }
        }
    }
}

fn free_tcp_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    listener.local_addr().ok().map(|a| a.port())
}

fn wait_for_port(addr: SocketAddr, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(50)) {
            let _ = stream.shutdown(Shutdown::Both);
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

/// SOCKS5 CONNECT + HTTP GET, timed from the first dial to the first response byte.
fn http_via_socks5(
    socks: SocketAddr,
    host: &str,
    port: u16,
    path: &str,
    timeout: Duration,
) -> Option<u64> {
    if host.len() > 255 {
        return None;
    }
    let start = Instant::now();
    let mut stream = TcpStream::connect_timeout(&socks, timeout).ok()?;
    let remaining = || timeout.checked_sub(start.elapsed()).unwrap_or(Duration::ZERO);
    if remaining().is_zero() {
        return None;
    }
    stream.set_read_timeout(Some(remaining())).ok()?;
    stream.set_write_timeout(Some(remaining())).ok()?;

    // greeting: VER=5, NMETHODS=1, METHOD=0 (no auth)
    stream.write_all(&[0x05, 0x01, 0x00]).ok()?;
    let mut greet = [0u8; 2];
    stream.read_exact(&mut greet).ok()?;
    if greet != [0x05, 0x00] {
        return None;
    }

    // CONNECT with domain ATYP
    let mut req = Vec::with_capacity(7 + host.len());
    req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
    req.extend_from_slice(host.as_bytes());
    req.push((port >> 8) as u8);
    req.push((port & 0xff) as u8);
    stream.write_all(&req).ok()?;

    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr).ok()?;
    if hdr[0] != 0x05 || hdr[1] != 0x00 {
        return None;
    }
    match hdr[3] {
        0x01 => {
            let mut skip = [0u8; 6];
            stream.read_exact(&mut skip).ok()?;
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).ok()?;
            let mut skip = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut skip).ok()?;
        }
        0x04 => {
            let mut skip = [0u8; 18];
            stream.read_exact(&mut skip).ok()?;
        }
        _ => return None,
    }

    if remaining().is_zero() {
        return None;
    }
    stream.set_read_timeout(Some(remaining())).ok()?;
    stream.set_write_timeout(Some(remaining())).ok()?;

    let http = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(http.as_bytes()).ok()?;
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).ok()?;
    if n < 12 || !buf.starts_with(b"HTTP/1.") {
        return None;
    }
    Some(start.elapsed().as_millis() as u64)
}

fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
    addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied()
}

/// Picks the lowest-metric IPv4 default route from `/proc/net/route` (which
/// lists the main table only) that does not point at the tunnel itself.
fn default_uplink(table: &str, tun_name: &str) -> Option<String> {
    const RTF_UP: u32 = 0x1;
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (iface, dest, flags, metric, mask) = (f.first()?, f.get(1)?, f.get(3)?, f.get(6)?, f.get(7)?);
            let flags = u32::from_str_radix(flags, 16).ok()?;
            let is_default = *dest == "00000000" && *mask == "00000000" && flags & RTF_UP != 0;
            (is_default && *iface != tun_name && *iface != "lo").then(|| (metric.parse::<u32>().unwrap_or(u32::MAX), iface.to_string()))
        })
        .min()
        .map(|(_, iface)| iface)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0
eth0\t00000000\t0100000A\t0003\t0\t0\t100\t00000000\t0\t0\t0
xrs-tun\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0
eth0\t0000000A\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";

    #[test]
    fn picks_lowest_metric_physical_default_route() {
        assert_eq!(default_uplink(TABLE, "xrs-tun").as_deref(), Some("eth0"));
    }

    #[test]
    fn ignores_tunnel_and_non_default_routes() {
        let only_tun = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask
xrs-tun\t00000000\t00000000\t0001\t0\t0\t0\t00000000
eth0\t0000000A\t00000000\t0001\t0\t0\t100\t00FFFFFF
";
        assert_eq!(default_uplink(only_tun, "xrs-tun"), None);
    }

    #[test]
    fn skips_routes_that_are_down() {
        let down = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask
eth0\t00000000\t0100000A\t0002\t0\t0\t100\t00000000
";
        assert_eq!(default_uplink(down, "xrs-tun"), None);
    }

    #[test]
    fn no_tunnel_means_plain_sockets() {
        assert_eq!(ProbeRoute::detect("xrs-definitely-missing"), ProbeRoute::default());
    }

    #[test]
    fn measures_a_local_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        assert!(ProbeRoute::default().tcp_latency("127.0.0.1", port).is_some());
    }

    #[test]
    fn socks5_http_probe_against_local_echo() {
        // Minimal fake SOCKS5 that accepts CONNECT then serves HTTP 204.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 3];
            stream.read_exact(&mut buf).expect("greet");
            stream.write_all(&[0x05, 0x00]).expect("greet-ok");
            let mut hdr = [0u8; 4];
            stream.read_exact(&mut hdr).expect("req-hdr");
            assert_eq!(hdr[0], 0x05);
            assert_eq!(hdr[1], 0x01);
            assert_eq!(hdr[3], 0x03);
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).expect("dlen");
            let mut rest = vec![0u8; len[0] as usize + 2];
            stream.read_exact(&mut rest).expect("domain+port");
            // reply: success, bind IPv4 0.0.0.0:0
            stream
                .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .expect("reply");
            let mut http = [0u8; 256];
            let n = stream.read(&mut http).expect("http");
            assert!(n > 0);
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .expect("resp");
        });

        let ms = http_via_socks5(addr, "example.com", 80, "/generate_204", Duration::from_secs(2));
        assert!(ms.is_some(), "expected successful probe");
        server.join().expect("server");
    }
}
