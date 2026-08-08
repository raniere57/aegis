//! Domain normalization helpers shared by DNS hot path and list compiler.

/// Lowercase, strip trailing dot, reject empty / invalid-ish names.
pub fn normalize_domain(raw: &str) -> Option<String> {
    let mut s = raw.trim().trim_matches(|c| c == '"' || c == '\'').to_lowercase();
    if s.ends_with('.') {
        s.pop();
    }
    if s.is_empty() || s.len() > 253 {
        return None;
    }
    if s == "localhost" || s.ends_with(".localhost") {
        return None;
    }
    // Basic label checks
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
    {
        return None;
    }
    if s.split('.').any(|label| label.is_empty() || label.len() > 63) {
        return None;
    }
    Some(s)
}

pub fn is_local_bypass(domain: &str) -> bool {
    domain == "localhost"
        || domain.ends_with(".localhost")
        || domain.ends_with(".local")
        || domain == "local"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(
            normalize_domain("Ads.Example.COM."),
            Some("ads.example.com".into())
        );
        assert!(normalize_domain("").is_none());
        assert!(normalize_domain("bad..com").is_none());
    }
}
