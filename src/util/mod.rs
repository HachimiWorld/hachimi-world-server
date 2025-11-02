use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use anyhow::bail;

pub mod gracefully_shutdown;
pub mod redlock;

pub trait IsBlank {
    fn is_blank(&self) -> bool; 
}

impl IsBlank for str {
    fn is_blank(&self) -> bool {
        self.is_empty() || self.chars().all(char::is_whitespace)
    }
}

impl IsBlank for String {
    fn is_blank(&self) -> bool {
        self.as_str().is_blank()
    }
}

pub fn convert_ip_to_anonymous_uid(ip: &str) -> anyhow::Result<i64> {
    if let Ok(ipv4) = Ipv4Addr::from_str(ip) {
        Ok(ipv4.to_bits() as i64)
    } else if let Ok(ipv6) = Ipv6Addr::from_str(ip) {
        // Take the first 64 bits
        let octets = ipv6.octets();
        let mut result: i64 = 0;
        for i in 0..8 {
            result = (result << 8) | (octets[i] as i64);
        }
        Ok(result)
    } else {
        bail!("Invalid IP address format: {ip}")
    }
}