use std::time::Duration;
use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowOrigin, Any, CorsLayer, MaxAge};

/// A single `allow_origins` entry: either an exact origin, or a single-level
/// subdomain wildcard like `https://*.example.com`.
#[derive(Debug)]
enum OriginRule {
    Exact(HeaderValue),
    Wildcard { scheme: String, suffix: String },
}

impl OriginRule {
    fn parse(s: &str) -> Self {
        // Browsers never send a trailing slash in `Origin`
        let s = s.trim_end_matches('/');
        match s.split_once("://*") {
            Some((scheme, suffix)) => Self::Wildcard { scheme: format!("{scheme}://"), suffix: suffix.to_owned() },
            None => Self::Exact(s.parse().unwrap_or_else(|_| panic!("invalid cors origin: {s}"))),
        }
    }

    fn matches(&self, origin: &HeaderValue) -> bool {
        match self {
            Self::Exact(v) => v == origin,
            Self::Wildcard { scheme, suffix } => origin.to_str().ok()
                .and_then(|o| o.strip_prefix(scheme.as_str())?.strip_suffix(suffix.as_str()))
                .is_some_and(|sub| !sub.is_empty() && !sub.contains(['/', ':', '.'])),
        }
    }
}

pub fn cors_layer(allow_origins: &[&str]) -> CorsLayer {
    let allow_origin = if allow_origins.contains(&"*") {
        AllowOrigin::any()
    } else {
        let rules: Vec<OriginRule> = allow_origins.iter().map(|s| OriginRule::parse(s)).collect();
        AllowOrigin::predicate(move |origin, _| rules.iter().any(|r| r.matches(origin)))
    };

    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        .max_age(MaxAge::exact(Duration::from_secs(86400)))
}

#[cfg(test)]
mod test {
    use super::OriginRule;
    use axum::http::HeaderValue;

    fn matches(rule: &str, origin: &str) -> bool {
        OriginRule::parse(rule).matches(&HeaderValue::from_str(origin).unwrap())
    }

    #[test]
    fn test_exact() {
        assert!(matches("http://localhost", "http://localhost"));
        assert!(matches("https://hachimi.world/", "https://hachimi.world"));
        assert!(!matches("http://localhost", "http://localhost:3000"));
        assert!(!matches("https://hachimi.world", "http://hachimi.world"));
    }

    #[test]
    fn test_wildcard() {
        let rule = "https://*.hachimi-world-client.pages.dev";
        assert!(matches(rule, "https://8ca6645a.hachimi-world-client.pages.dev"));
        assert!(matches(&format!("{rule}/"), "https://8ca6645a.hachimi-world-client.pages.dev"));

        assert!(!matches(rule, "https://hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "https://.hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "http://8ca6645a.hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "https://a.b.hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "https://evil.com/.hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "https://evil.com:.hachimi-world-client.pages.dev"));
        assert!(!matches(rule, "https://8ca6645a.hachimi-world-client.pages.dev.evil.com"));
    }
}
