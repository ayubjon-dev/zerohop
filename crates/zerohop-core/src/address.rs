//! Decides which network addresses Zerohop may talk to (protocol section 6.1).
//!
//! An address is local only when it is on-link on an active interface that is
//! neither a VPN nor cellular. The same check runs for outgoing and incoming
//! connections.

use std::io;
use std::net::IpAddr;

/// One subnet attached to an eligible interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subnet {
    /// An address on the interface.
    pub addr: IpAddr,
    /// The interface's netmask.
    pub mask: IpAddr,
}

/// The set of networks considered local right now.
#[derive(Clone, Debug, Default)]
pub struct LocalNetworks {
    subnets: Vec<Subnet>,
    has_ipv6_interface: bool,
    allow_loopback: bool,
}

const EXCLUDED_INTERFACE_PREFIXES: &[&str] = &[
    "utun", "tun", "tap", "wg", "ppp", "ipsec", "pdp_ip", "rmnet", "ccmni", "lo",
];

fn is_eligible_interface(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !EXCLUDED_INTERFACE_PREFIXES
        .iter()
        .any(|p| lower.starts_with(p))
}

fn same_subnet(a: IpAddr, b: IpAddr, mask: IpAddr) -> bool {
    match (a, b, mask) {
        (IpAddr::V4(a), IpAddr::V4(b), IpAddr::V4(m)) => {
            let m = u32::from(m);
            u32::from(a) & m == u32::from(b) & m
        }
        (IpAddr::V6(a), IpAddr::V6(b), IpAddr::V6(m)) => {
            let m = u128::from(m);
            u128::from(a) & m == u128::from(b) & m
        }
        _ => false,
    }
}

impl LocalNetworks {
    /// Reads the current interfaces from the operating system.
    ///
    /// # Errors
    /// Returns the operating system's error if interfaces cannot be listed.
    pub fn detect() -> io::Result<Self> {
        let subnets = if_addrs::get_if_addrs()?
            .into_iter()
            .filter(|i| !i.is_loopback() && is_eligible_interface(&i.name))
            .map(|i| match i.addr {
                if_addrs::IfAddr::V4(v4) => Subnet {
                    addr: IpAddr::V4(v4.ip),
                    mask: IpAddr::V4(v4.netmask),
                },
                if_addrs::IfAddr::V6(v6) => Subnet {
                    addr: IpAddr::V6(v6.ip),
                    mask: IpAddr::V6(v6.netmask),
                },
            })
            .collect();
        Ok(Self::from_subnets(subnets))
    }

    /// This device's addresses on the local networks.
    #[must_use]
    pub fn addresses(&self) -> Vec<IpAddr> {
        self.subnets.iter().map(|s| s.addr).collect()
    }

    /// Builds the set from a known list of subnets.
    #[must_use]
    pub fn from_subnets(subnets: Vec<Subnet>) -> Self {
        let has_ipv6_interface = subnets.iter().any(|s| s.addr.is_ipv6());
        Self {
            subnets,
            has_ipv6_interface,
            allow_loopback: false,
        }
    }

    /// Also treats loopback as local. Exists only for tests and benchmarks.
    #[cfg(feature = "dev-loopback")]
    #[must_use]
    pub fn with_loopback(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// Whether Zerohop may exchange traffic with this address.
    #[must_use]
    pub fn is_local(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        if ip.is_loopback() {
            return self.allow_loopback;
        }
        if ip.is_unspecified() || ip.is_multicast() {
            return false;
        }
        if let IpAddr::V6(v6) = ip
            && v6.is_unicast_link_local()
        {
            return self.has_ipv6_interface;
        }
        self.subnets.iter().any(|s| same_subnet(ip, s.addr, s.mask))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn home() -> LocalNetworks {
        LocalNetworks::from_subnets(vec![
            Subnet {
                addr: "192.168.1.20".parse().unwrap(),
                mask: "255.255.255.0".parse().unwrap(),
            },
            Subnet {
                addr: "2001:db8:1:2::5".parse().unwrap(),
                mask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
            },
        ])
    }

    #[test]
    fn accepts_an_address_on_the_same_subnet() {
        assert!(home().is_local("192.168.1.77".parse().unwrap()));
    }

    #[test]
    fn rejects_a_private_address_on_another_subnet() {
        assert!(!home().is_local("192.168.2.77".parse().unwrap()));
        assert!(!home().is_local("10.0.0.5".parse().unwrap()));
    }

    #[test]
    fn rejects_public_and_carrier_grade_addresses() {
        assert!(!home().is_local("8.8.8.8".parse().unwrap()));
        assert!(!home().is_local("100.64.0.9".parse().unwrap()));
    }

    #[test]
    fn rejects_a_public_address_hidden_in_ipv4_mapped_form() {
        assert!(!home().is_local("::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn accepts_a_local_address_in_ipv4_mapped_form() {
        assert!(home().is_local("::ffff:192.168.1.3".parse().unwrap()));
    }

    #[test]
    fn accepts_on_link_global_ipv6() {
        assert!(home().is_local("2001:db8:1:2::99".parse().unwrap()));
        assert!(!home().is_local("2001:db8:9:9::99".parse().unwrap()));
    }

    #[test]
    fn accepts_ipv6_link_local_when_an_interface_has_ipv6() {
        assert!(home().is_local("fe80::1".parse().unwrap()));
    }

    #[test]
    fn rejects_ipv6_link_local_without_an_ipv6_interface() {
        let v4_only = LocalNetworks::from_subnets(vec![Subnet {
            addr: "10.0.0.2".parse().unwrap(),
            mask: "255.255.255.0".parse().unwrap(),
        }]);
        assert!(!v4_only.is_local("fe80::1".parse().unwrap()));
    }

    #[test]
    fn rejects_loopback_by_default() {
        assert!(!home().is_local("127.0.0.1".parse().unwrap()));
        assert!(!home().is_local("::1".parse().unwrap()));
        assert!(!home().is_local("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn rejects_unspecified_and_multicast() {
        assert!(!home().is_local("0.0.0.0".parse().unwrap()));
        assert!(!home().is_local("224.0.0.251".parse().unwrap()));
        assert!(!home().is_local("ff02::fb".parse().unwrap()));
    }

    #[test]
    fn excludes_vpn_cellular_and_loopback_interfaces() {
        for name in ["utun3", "wg0", "pdp_ip0", "rmnet_data1", "lo0", "tun0"] {
            assert!(!is_eligible_interface(name), "{name}");
        }
        for name in ["en0", "wlan0", "Ethernet", "Wi-Fi"] {
            assert!(is_eligible_interface(name), "{name}");
        }
    }

    proptest! {
        #[test]
        fn no_address_outside_the_subnet_is_local(a in any::<u32>()) {
            let ip = Ipv4Addr::from(a);
            let in_net = a & 0xFFFF_FF00 == u32::from(Ipv4Addr::new(192, 168, 1, 0));
            if !in_net {
                prop_assert!(!home().is_local(IpAddr::V4(ip)));
                prop_assert!(!home().is_local(IpAddr::V6(ip.to_ipv6_mapped())));
            }
        }

        #[test]
        fn random_ipv6_outside_known_ranges_is_not_local(a in any::<u128>()) {
            let ip = Ipv6Addr::from(a);
            let net = u128::from("2001:db8:1:2::".parse::<Ipv6Addr>().unwrap());
            let is_mapped = ip.to_ipv4_mapped().is_some();
            let link_local = ip.is_unicast_link_local();
            if a >> 64 != net >> 64 && !is_mapped && !link_local {
                prop_assert!(!home().is_local(IpAddr::V6(ip)));
            }
        }
    }
}
