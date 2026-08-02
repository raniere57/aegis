use aegis_core::normalize::normalize_domain;

/// Parse hosts / domain-list / simple Adblock domain rules into normalized unique domains.
pub fn normalize_list_text(text: &str) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        // Strip inline comments
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }

        // Adblock: ||ads.example.com^  or ||ads.example.com^$important
        let domain = if let Some(rest) = line.strip_prefix("||") {
            let host = rest.split('^').next().unwrap_or(rest);
            let host = host.split('$').next().unwrap_or(host).trim();
            // Skip path-style or wildcard-heavy rules
            if host.contains('/') || host.contains('*') || host.contains('^') {
                None
            } else {
                Some(host)
            }
        } else if let Some(rest) = line.strip_prefix("0.0.0.0") {
            rest.trim().split_whitespace().next()
        } else if let Some(rest) = line.strip_prefix("127.0.0.1") {
            rest.trim().split_whitespace().next()
        } else if let Some(rest) = line.strip_prefix("::") {
            let rest = rest.trim();
            let rest = rest.strip_prefix('1').unwrap_or(rest).trim();
            rest.split_whitespace().next()
        } else if line.contains(' ') || line.contains('\t') {
            line.split_whitespace().nth(1)
        } else {
            Some(line.trim_end_matches('^'))
        };

        let Some(domain) = domain else { continue };
        if domain.parse::<std::net::IpAddr>().is_ok() {
            continue;
        }
        if let Some(n) = normalize_domain(domain) {
            if n != "localhost" && n != "broadcasthost" && n != "local" {
                set.insert(n);
            }
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts() {
        let text = r#"
# comment
0.0.0.0 ads.example.com
127.0.0.1 tracker.net
example.org
||blocked.example.net^
"#;
        let d = normalize_list_text(text);
        assert!(d.iter().any(|x| x == "ads.example.com"));
        assert!(d.iter().any(|x| x == "tracker.net"));
        assert!(d.iter().any(|x| x == "example.org"));
        assert!(d.iter().any(|x| x == "blocked.example.net"));
    }
}
