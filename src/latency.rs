use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::fs;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// How probe sockets leave the machine.
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

    pub fn bypasses_tunnel(&self) -> bool {
        self.bind_device.is_some()
    }

    /// TCP handshake time to `host:port`. DNS resolution is excluded so the
    /// number reflects the network path rather than resolver speed.
    pub fn tcp_latency(&self, host: &str, port: u16) -> Option<u64> {
        let addr = resolve(host, port)?;
        let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP)).ok()?;
        // Only IPv4 is policy-routed into the tunnel, so v6 sockets stay unbound.
        if let (Some(dev), SocketAddr::V4(_)) = (&self.bind_device, addr)
            && let Err(e) = socket.bind_device(Some(dev.as_bytes()))
        {
            log::debug!("SO_BINDTODEVICE {dev} failed: {e}");
        }
        let target = SockAddr::from(addr);
        let start = Instant::now();
        socket.connect_timeout(&target, PROBE_TIMEOUT).ok()?;
        Some(start.elapsed().as_millis() as u64)
    }
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
        assert!(!ProbeRoute::detect("xrs-definitely-missing").bypasses_tunnel());
    }

    #[test]
    fn measures_a_local_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        assert!(ProbeRoute::default().tcp_latency("127.0.0.1", port).is_some());
    }
}
