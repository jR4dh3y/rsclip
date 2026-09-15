use url::Url;

/// Extracted link metadata.
#[derive(Clone, Debug)]
pub struct LinkInfo {
    /// Full normalized URL string.
    pub url: String,
    /// Domain name stripped of leading `www.` (e.g. `github.com`).
    pub domain: String,
    /// Icon identifier for recognized platforms or generic fallback.
    pub icon: String,
}

/// Detect whether the provided text consists solely of a single valid HTTP/HTTPS URL.
pub fn detect_single_url(text: &str) -> Option<LinkInfo> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.split_whitespace().count() != 1 {
        return None;
    }

    if !trimmed.contains("://") {
        return None;
    }

    let url = Url::parse(trimmed).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    let domain = url
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if !is_valid_web_host(&domain) {
        return None;
    }

    Some(LinkInfo {
        url: url.to_string(),
        icon: icon_for_domain(&domain).to_string(),
        domain,
    })
}

fn is_valid_web_host(host: &str) -> bool {
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// Return the icon name for a known domain, or `"globe"` for general web links.
pub fn icon_for_domain(domain: &str) -> &'static str {
    match domain {
        "github.com" => "github",
        "gitlab.com" => "gitlab",
        "linkedin.com" => "linkedin",
        "youtube.com" | "youtu.be" => "youtube",
        "x.com" | "twitter.com" => "x-twitter",
        "reddit.com" => "reddit",
        "instagram.com" => "instagram",
        "facebook.com" => "facebook",
        "discord.com" | "discord.gg" => "discord",
        "stackoverflow.com" => "stackoverflow",
        "docs.rs" | "crates.io" => "rust",
        "npmjs.com" => "npm",
        "pypi.org" => "python",
        "archlinux.org" | "wiki.archlinux.org" => "arch",
        _ => "globe",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_url() {
        let link = detect_single_url("https://github.com/owner/repo").unwrap();
        assert_eq!(link.domain, "github.com");
        assert_eq!(link.icon, "github");
    }

    #[test]
    fn detects_http_url() {
        let link = detect_single_url("http://example.com/path").unwrap();
        assert_eq!(link.url, "http://example.com/path");
        assert_eq!(link.domain, "example.com");
    }

    #[test]
    fn ignores_bare_domain() {
        assert!(detect_single_url("example.com").is_none());
        assert!(detect_single_url("ghcr.io").is_none());
    }

    #[test]
    fn ignores_non_http_schemes() {
        assert!(detect_single_url("ftp://example.com/file").is_none());
    }

    #[test]
    fn ignores_invalid_token_hostnames() {
        assert!(
            detect_single_url("https://fbsa_1f3a8e0389a8c0cbc656fca80307e478.fbs-admin-token-2026")
                .is_none()
        );
    }

    #[test]
    fn ignores_multi_word_text() {
        assert!(detect_single_url("open https://github.com now").is_none());
    }
}
