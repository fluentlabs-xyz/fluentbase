//! Shared `"host:port"` → [`Ingress`] parser for operator-supplied peer
//! addresses (bootstrappers JSON `socket` field and `--dpos.dialable`).
//!
//! A string that parses as a [`SocketAddr`] (IPv4 `1.2.3.4:9000`, bracketed
//! IPv6 `[::1]:9000`) yields [`Ingress::Socket`]; anything else is treated as
//! a DNS `host:port` and validated against commonware's RFC-1035/1123
//! [`Hostname`] type, yielding [`Ingress::Dns`]. DNS-form ingress is only
//! *dialed* when the network-wide `ALLOW_DNS` policy is `true`
//! (see [`crate::constants::ALLOW_DNS`]); this parser is policy-agnostic.

use std::net::SocketAddr;

use commonware_p2p::Ingress;
use commonware_utils::Hostname;

/// Parse a `"host:port"` string into an [`Ingress`].
///
/// - Exact [`SocketAddr`] literal (IPv4, or bracketed IPv6) → [`Ingress::Socket`].
/// - Otherwise split on the LAST `':'` into host + port; the host is validated
///   as a DNS hostname (`Hostname::new`, RFC-1035/1123, ≤253 chars) → [`Ingress::Dns`].
///
/// Errors name the offending string. A bare hostname without a port, an empty
/// host, an invalid port, or a hostname that fails RFC validation (e.g. an
/// unbracketed IPv6 literal, an over-length label, an underscore) all reject.
pub fn parse_ingress(s: &str) -> eyre::Result<Ingress> {
    // Fast path: exact socket literal (covers IPv4 and bracketed IPv6).
    if let Ok(addr) = s.parse::<SocketAddr>() {
        return Ok(Ingress::Socket(addr));
    }
    // DNS form: split host from port on the last ':'.
    let (host, port_str) = s
        .rsplit_once(':')
        .ok_or_else(|| eyre::eyre!("peer address {s:?} is missing a ':port' suffix"))?;
    if host.is_empty() {
        return Err(eyre::eyre!("peer address {s:?} has an empty host"));
    }
    let port: u16 = port_str
        .parse()
        .map_err(|_| eyre::eyre!("peer address {s:?} has an invalid port {port_str:?}"))?;
    // A host whose every dot-separated label is all-digits (e.g. `300.1.2.3`) is
    // never a legitimate DNS name for a peer address — it's a malformed IP that
    // just missed the `SocketAddr` fast path (out-of-range octet, too few/many
    // groups). Fail fast rather than silently dialing it as a hostname.
    if host.split('.').all(|label| !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit())) {
        return Err(eyre::eyre!(
            "peer address {s:?} looks like a malformed IP address (all-numeric host {host:?}); \
             expected a valid IP literal or a DNS hostname"
        ));
    }
    let host = Hostname::new(host)
        .map_err(|e| eyre::eyre!("peer address {s:?} has an invalid DNS hostname: {e}"))?;
    Ok(Ingress::Dns { host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ipv4_socket() {
        match parse_ingress("10.0.0.1:9000").unwrap() {
            Ingress::Socket(a) => assert_eq!(a, "10.0.0.1:9000".parse::<SocketAddr>().unwrap()),
            other => panic!("expected Socket, got {other:?}"),
        }
    }

    #[test]
    fn parses_bracketed_ipv6_socket() {
        match parse_ingress("[::1]:9000").unwrap() {
            Ingress::Socket(a) => assert_eq!(a, "[::1]:9000".parse::<SocketAddr>().unwrap()),
            other => panic!("expected Socket, got {other:?}"),
        }
    }

    #[test]
    fn parses_dns_hostname() {
        match parse_ingress("validator-3:9000").unwrap() {
            Ingress::Dns { host, port } => {
                assert_eq!(host.as_str(), "validator-3");
                assert_eq!(port, 9000);
            }
            other => panic!("expected Dns, got {other:?}"),
        }
    }

    #[test]
    fn parses_dotted_dns_hostname() {
        match parse_ingress("seed.example.com:9000").unwrap() {
            Ingress::Dns { host, port } => {
                assert_eq!(host.as_str(), "seed.example.com");
                assert_eq!(port, 9000);
            }
            other => panic!("expected Dns, got {other:?}"),
        }
    }

    #[test]
    fn rejects_hostname_with_invalid_port() {
        assert!(parse_ingress("validator-3:notaport").is_err());
        assert!(parse_ingress("validator-3:99999").is_err());
    }

    #[test]
    fn rejects_missing_port() {
        assert!(parse_ingress("validator-3").is_err());
    }

    #[test]
    fn rejects_empty_host() {
        assert!(parse_ingress(":9000").is_err());
    }

    #[test]
    fn rejects_overlong_host() {
        // >253 chars fails commonware's Hostname RFC-1035 length cap.
        let host = "a".repeat(300);
        assert!(parse_ingress(&format!("{host}:9000")).is_err());
    }

    #[test]
    fn rejects_malformed_ip_as_all_numeric_host() {
        // Out-of-range octet fails SocketAddr, must NOT become a DNS hostname.
        assert!(parse_ingress("300.1.2.3:9000").is_err());
        // Too few groups, all-numeric.
        assert!(parse_ingress("10.0.0:9000").is_err());
        // A bare numeric label.
        assert!(parse_ingress("12345:9000").is_err());
    }

    #[test]
    fn rejects_unbracketed_ipv6() {
        // A bare IPv6 literal is neither a SocketAddr nor a valid hostname.
        assert!(parse_ingress("::1:9000").is_err());
    }
}
