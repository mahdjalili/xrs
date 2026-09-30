//! Just enough of RFC 3986 / WHATWG URL splitting for share links
//! (`vless://`, `trojan://`) and subscription URLs, without the IDNA and
//! Unicode normalization tables the `url` crate compiles in.

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use std::net::Ipv6Addr;

/// WHATWG path percent-encode set.
const PATH: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'<').add(b'>').add(b'?').add(b'`').add(b'{').add(b'}');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri<'a> {
    pub scheme: String,
    /// Still percent-encoded.
    pub username: &'a str,
    /// Percent-decoded; IPv6 literals are canonical and unbracketed.
    pub host: String,
    pub port: Option<u16>,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub fragment: Option<&'a str>,
}

impl<'a> Uri<'a> {
    pub fn parse(input: &'a str) -> Option<Self> {
        let s = input.trim_matches(|c: char| c <= ' ');
        let (scheme, rest) = s.split_once("://")?;
        let mut chars = scheme.chars();
        if !chars.next()?.is_ascii_alphabetic()
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let scheme = scheme.to_ascii_lowercase();
        let special = matches!(scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp");
        let rest = if special { rest.trim_start_matches(['/', '\\']) } else { rest };

        let (rest, fragment) = split_opt(rest, '#');
        let (rest, query) = split_opt(rest, '?');
        let end = rest.find(|c| c == '/' || (special && c == '\\')).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(end);

        let (userinfo, host_port) = authority.rsplit_once('@').unwrap_or(("", authority));
        let username = userinfo.split(':').next().unwrap_or_default();

        let (host, port) = if let Some(v6) = host_port.strip_prefix('[') {
            let (addr, after) = v6.split_once(']')?;
            let port = match after {
                "" => "",
                p => p.strip_prefix(':')?,
            };
            (addr.parse::<Ipv6Addr>().ok()?.to_string(), port)
        } else {
            let (raw, port) = host_port.split_once(':').unwrap_or((host_port, ""));
            // Opaque (non-special) hosts are validated before decoding, domains after.
            if !special && raw.chars().any(is_forbidden_host_char) {
                return None;
            }
            let host = percent_decode_str(raw).decode_utf8().ok()?.into_owned();
            if special && host.chars().any(|c| c == '%' || is_forbidden_host_char(c)) {
                return None;
            }
            (if special { host.to_ascii_lowercase() } else { host }, port)
        };
        if host.is_empty() {
            return None;
        }
        // Like `url`, a backslash may end the port even for non-special schemes.
        let port = port.split('\\').next().unwrap_or_default();
        let port = match port {
            "" => None,
            p if p.bytes().all(|b| b.is_ascii_digit()) => Some(p.parse::<u16>().ok()?),
            _ => return None,
        };

        Some(Self { scheme, username, host, port, path, query, fragment })
    }

    /// Host as `Url::host_str` prints it (IPv6 in brackets).
    pub fn host_str(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
    }

    /// The path as `Url::path` serialises it for http(s): never empty,
    /// backslashes as slashes, dot segments resolved, unsafe bytes escaped.
    pub fn http_path(&self) -> String {
        let mut out: Vec<&str> = Vec::new();
        let segments: Vec<&str> = self.path.split(['/', '\\']).skip(1).collect();
        for (i, seg) in segments.iter().enumerate() {
            let last = i + 1 == segments.len();
            match seg.to_ascii_lowercase().as_str() {
                "." | "%2e" => {
                    if last {
                        out.push("");
                    }
                }
                ".." | ".%2e" | "%2e." | "%2e%2e" => {
                    out.pop();
                    if last {
                        out.push("");
                    }
                }
                _ => out.push(seg),
            }
        }
        let joined = format!("/{}", out.join("/"));
        utf8_percent_encode(&joined, PATH).to_string()
    }

    /// `application/x-www-form-urlencoded` pairs, like `Url::query_pairs`.
    pub fn query_pairs(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.query
            .unwrap_or_default()
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (form_decode(k), form_decode(v))
            })
    }
}

fn split_opt(s: &str, sep: char) -> (&str, Option<&str>) {
    match s.split_once(sep) {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

fn form_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    percent_decode_str(&s).decode_utf8_lossy().into_owned()
}

fn is_forbidden_host_char(c: char) -> bool {
    matches!(c, '\0' | '\t' | '\n' | '\r' | ' ' | '#' | '/' | ':' | '<' | '>' | '?' | '@' | '[' | '\\' | ']' | '^' | '|')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_share_links() {
        let u = Uri::parse("vless://id%40x:pw@Host.Example:443/p?type=ws&path=%2Fa+b#Name%201").expect("parse");
        assert_eq!(u.scheme, "vless");
        assert_eq!(u.username, "id%40x");
        assert_eq!(u.host, "Host.Example");
        assert_eq!(u.port, Some(443));
        assert_eq!(u.path, "/p");
        assert_eq!(u.fragment, Some("Name%201"));
        let q: Vec<_> = u.query_pairs().collect();
        assert_eq!(q[1], ("path".to_string(), "/a b".to_string()));
    }

    #[test]
    fn ipv6_is_canonicalised() {
        let u = Uri::parse("trojan://pw@[2001:DB8:0::1]:8443").expect("parse");
        assert_eq!((u.host.as_str(), u.port), ("2001:db8::1", Some(8443)));
        assert!(Uri::parse("trojan://pw@[zz::1]:8443").is_none());
    }

    #[test]
    fn rejects_bad_ports_and_hosts() {
        assert!(Uri::parse("vless://u@h:70000").is_none());
        assert!(Uri::parse("vless://u@h:44x").is_none());
        assert!(Uri::parse("vless://u@a b:443").is_none());
        assert!(Uri::parse("https://:443/x").is_none());
        assert!(Uri::parse("no-scheme").is_none());
        assert_eq!(Uri::parse("vless://u@h:").map(|u| u.port), Some(None));
    }

    #[test]
    fn special_schemes_lowercase_hosts() {
        let u = Uri::parse("HTTPS://user:tok@Sub.Example.COM:8443?token=1").expect("parse");
        assert_eq!((u.scheme.as_str(), u.host.as_str(), u.path), ("https", "sub.example.com", ""));
        assert_eq!(u.query, Some("token=1"));
    }
}
