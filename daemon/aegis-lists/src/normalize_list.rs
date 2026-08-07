use std::net::IpAddr;

use aegis_core::normalize::normalize_domain;

/// Parse hosts / domain-list / simple Adblock domain rules into normalized unique domains.
pub fn normalize_list_text(text: &str) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        // Element-hiding rules (##, #@#, #?#, #$#) and exceptions (@@) are not domain blocks.
        // These must be caught BEFORE stripping '#' comments below: otherwise EasyList's
        // `nytimes.com###ad-slot` collapses to a bare `nytimes.com` and suffix-blocks the site.
        if line.starts_with("@@")
            || line.contains("##")
            || line.contains("#@#")
            || line.contains("#?#")
            || line.contains("#$#")
        {
            continue;
        }
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }

        let domain = if let Some(rest) = line.strip_prefix("||") {
            // Adblock: ||ads.example.com^  or ||ads.example.com^$important
            let host = rest.split('^').next().unwrap_or(rest);
            let host = host.split('$').next().unwrap_or(host).trim();
            // Skip path-style or wildcard-heavy rules
            if host.contains('/') || host.contains('*') || host.contains('^') {
                None
            } else {
                Some(host)
            }
        } else {
            // hosts format is "<address> <name>"; a plain domain list is just "<name>".
            // Decide by parsing the first token, never by string prefix — the old
            // `strip_prefix("::")` + `strip_prefix('1')` chain ate the domain's first
            // character, turning ":: 1password.com" into "password.com".
            let mut tokens = line.split_whitespace();
            match tokens.next() {
                Some(first) if first.parse::<IpAddr>().is_ok() => tokens.next(),
                Some(first) => Some(first.trim_end_matches('^')),
                None => None,
            }
        };

        // AdGuard-style wildcard lists write "*.example.com" for what is already suffix
        // matching here, so strip the prefix rather than dropping the entry.
        let domain = domain.map(|d| d.strip_prefix("*.").unwrap_or(d));

        let Some(domain) = domain else { continue };
        if domain.parse::<IpAddr>().is_ok() {
            continue;
        }
        if let Some(n) = normalize_domain(domain) {
            // A single label is a TLD under suffix matching, so `plus` harvested from an
            // "[Adblock Plus 2.0]" header would NXDOMAIN every name under .plus. This also
            // covers the old localhost/broadcasthost/local special cases.
            if n.contains('.') {
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

    /// EasyList cosmetic rules must never yield the bare site domain: stripping the '#'
    /// comment first turned `nytimes.com###ad` into a suffix block on the whole site.
    #[test]
    fn cosmetic_rules_never_yield_the_site_domain() {
        let text = "nytimes.com###APS_300_X_600\ngoogle.com##.ad-banner\nexample.org#@#.ad";
        assert_eq!(normalize_list_text(text), Vec::<String>::new());
    }

    /// "[Adblock Plus 2.0]" is three whitespace tokens; nth(1) used to harvest "Plus",
    /// which suffix-blocks every name under the real .plus gTLD.
    #[test]
    fn rejects_single_label_entries_that_would_block_a_tld() {
        for text in ["[Adblock Plus 2.0]", "plus", "ip6-localhost", "localhost", "broadcasthost"] {
            assert_eq!(normalize_list_text(text), Vec::<String>::new(), "input: {text}");
        }
    }

    /// The old chain stripped a literal '1' after trimming, eating the domain's first
    /// character instead of the address's.
    #[test]
    fn ipv6_hosts_lines_keep_the_whole_domain() {
        let d = normalize_list_text(":: 1password.com\n::1 1drv.ms\nfe80::1 example.net");
        assert!(d.iter().any(|x| x == "1password.com"), "got {d:?}");
        assert!(d.iter().any(|x| x == "1drv.ms"), "got {d:?}");
    }

    /// An HTML interstitial (captive portal, GitHub page URL instead of the raw one) must
    /// parse to nothing, so the empty-list guard in the updater can catch it.
    #[test]
    fn html_yields_nothing() {
        let text = "<!DOCTYPE html>\n<body>Sign in to WiFi</body>\n<p>Please log in to continue</p>";
        assert_eq!(normalize_list_text(text), Vec::<String>::new());
    }

    /// AdGuard wildcard lists are suffix rules, which is already how the trie matches.
    #[test]
    fn accepts_adguard_wildcards() {
        let d = normalize_list_text("*.wild.example\n*.ads.tracker.net");
        assert!(d.iter().any(|x| x == "wild.example"), "got {d:?}");
        assert!(d.iter().any(|x| x == "ads.tracker.net"), "got {d:?}");
    }

    #[test]
    fn skips_adblock_exceptions() {
        let d = normalize_list_text("@@||good.example.com^\n||bad.example.com^");
        assert_eq!(d, vec!["bad.example.com".to_string()]);
    }

    #[test]
    fn still_handles_trailing_comments_and_ip_tokens() {
        let d = normalize_list_text("0.0.0.0 ads.example.com # tracker\nexample.org#note");
        assert!(d.iter().any(|x| x == "ads.example.com"), "got {d:?}");
        assert!(d.iter().any(|x| x == "example.org"), "got {d:?}");
    }
}
