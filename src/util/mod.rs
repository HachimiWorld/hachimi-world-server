use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::LazyLock;
use anyhow::bail;
use url::Url;
use crate::{common, err};
use crate::web::result::{CommonError, WebError};

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

impl IsBlank for Option<String> {
    fn is_blank(&self) -> bool {
        match self {
            None => true,
            Some(x) => x.is_blank()
        }
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


static PLATFORM_HOST_MAP: LazyLock<HashMap<&'static str, Vec<&'static str>>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert("bilibili", vec!["www.bilibili.com", "b23.tv"]);
    map.insert("douyin", vec!["v.douyin.com"]);
    map.insert("youtube", vec!["www.youtube.com", "youtu.be"]);
    map.insert("niconico", vec!["www.nicovideo.jp"]);
    map
});

pub fn validate_platforms(platform: &str, url: &str) -> Result<bool, WebError<CommonError>>{
    let url = match Url::parse(&url) {
        Ok(url) => url,
        Err(_) => err!("invalid_external_link_url", "Invalid url in external link")
    };
    let host = url.host_str().ok_or_else(|| common!("invalid_url", "Invalid url in external link"))?;
    // Validate for all supported platforms
    let domains = PLATFORM_HOST_MAP.get(platform);
    match domains {
        Some(domains) => {
            if !domains.iter().any(|&domain| host.ends_with(domain)) {
                err!("invalid_external_link", "Invalid Bilibili url")
            }
            Ok(true)
        }
        None => {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::util::validate_platforms;

    #[test]
    fn test_validate_platforms() {
        assert!(validate_platforms("bilibili", "https://www.bilibili.com/video/BV114514").unwrap());
        assert!(validate_platforms("niconico", "https://www.nicovideo.jp/watch/sm114514").unwrap());
        assert!(validate_platforms("douyin", "https://v.douyin.com/114514-1145/").unwrap());
        assert!(validate_platforms("youtube", "https://youtu.be/114514").unwrap());
        assert!(validate_platforms("youtube", "https://www.youtube.com/watch?v=114514").unwrap());
        assert!(validate_platforms("bilibili", "https://www.youtube.com/watch?v=114514").is_err());
        assert_eq!(validate_platforms("instgram", "https://www.youtube.com/watch?v=114514").unwrap(), false);
    }
}