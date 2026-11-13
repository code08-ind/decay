//! Item domains: eTLD+1, lowercase, private suffixes included (each `github.io` site is its
//! own domain), like the extension's `domainOf` (tldts with `allowPrivateDomains`).

/// The registrable domain of an http(s) URL; the bare host for IP addresses, `localhost`
/// and other hosts without a public suffix; `None` for other URLs.
pub fn domain_of(url: &str) -> Option<String> {
    let scheme = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(scheme.starts_with("http://") || scheme.starts_with("https://")) {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    let host = match parsed.host()? {
        url::Host::Domain(d) => d.trim_end_matches('.').to_ascii_lowercase(),
        url::Host::Ipv4(ip) => return Some(ip.to_string()),
        url::Host::Ipv6(ip) => return Some(ip.to_string()),
    };
    if host.is_empty() {
        return None;
    }
    let domain = psl::domain_str(&host).unwrap_or(&host);
    Some(domain.to_owned())
}

#[cfg(test)]
mod tests {
    use super::domain_of;

    #[test]
    fn registrable_domains_like_the_extension() {
        let d = |u: &str| domain_of(u);
        assert_eq!(d("https://docs.github.com/en").as_deref(), Some("github.com"));
        assert_eq!(d("https://news.bbc.co.uk/x").as_deref(), Some("bbc.co.uk"));
        assert_eq!(d("HTTPS://WWW.Example.COM/").as_deref(), Some("example.com"));
        // Private suffixes count: each hosted site is its own domain.
        assert_eq!(d("https://alice.github.io/blog").as_deref(), Some("alice.github.io"));
        assert_eq!(d("http://localhost:3000/").as_deref(), Some("localhost"));
        assert_eq!(d("http://127.0.0.1:8080/").as_deref(), Some("127.0.0.1"));
        assert_eq!(d("http://[::1]/").as_deref(), Some("::1"));
        assert_eq!(d("chrome://settings"), None);
        assert_eq!(d("about:blank"), None);
        assert_eq!(d("https://"), None);
    }
}
