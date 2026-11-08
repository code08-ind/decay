//! Rule matching (F8, F9). Port of the extension's `lib/policy/match.ts`; the semantics
//! are fixed in API Design §1 ("Rule semantics") and must stay identical in both.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, LazyLock, Mutex};

use url::Url;

use crate::js_regex;
use crate::types::{Rule, RuleScope};

/// What a rule is matched against.
#[derive(Debug, Clone, Copy)]
pub struct MatchTarget<'a> {
    pub app_id: &'a str,
    pub url: Option<&'a str>,
    pub title: &'a str,
}

/// Lowercase hostname of an http(s) URL (trailing dot removed), else `None`.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(host.strip_suffix('.').map(str::to_owned).unwrap_or(host))
}

fn is_ip(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<Ipv4Addr>().is_ok() || bare.parse::<Ipv6Addr>().is_ok()
}

/// `github.com` matches `github.com` and every subdomain, never `notgithub.com`.
/// IP addresses only match exactly (`1.1` must not match `192.168.1.1`). Same steps as the
/// extension's `domainMatches`.
pub fn domain_matches(host: Option<&str>, rule_domain: &str) -> bool {
    let Some(host) = host else { return false };
    let lower = rule_domain.to_lowercase(); // JS `toLowerCase`
    let d = lower.strip_prefix("*.").unwrap_or(&lower);
    let d = d.strip_suffix('.').unwrap_or(d);
    // Hosts are ASCII (punycode), so a Unicode rule domain (`bücher.de`) is converted the
    // way the browser converts hosts.
    let ascii;
    let d = if d.is_ascii() {
        d
    } else {
        ascii = Url::parse(&format!("http://{d}/"))
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_else(|| d.to_owned());
        &ascii
    };
    if d.is_empty() {
        return false;
    }
    if host == d {
        return true;
    }
    if is_ip(host) || is_ip(d) {
        return false;
    }
    host.len() > d.len() && host.ends_with(d) && host.as_bytes()[host.len() - d.len() - 1] == b'.'
}

/// The URL that globs see: userinfo is dropped (`https://github.com@evil.com/` is
/// evil.com), so a glob can't be fooled by it. Non-web URLs are used as-is.
pub fn glob_subject(url: &str) -> String {
    let Ok(u) = Url::parse(url) else { return url.to_owned() };
    if !matches!(u.scheme(), "http" | "https") {
        return url.to_owned();
    }
    let mut out = format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""));
    if let Some(port) = u.port() {
        let _ = write!(out, ":{port}");
    }
    out.push_str(u.path());
    // Like the browser's `search` / `hash`: an empty query or fragment is dropped.
    if let Some(q) = u.query().filter(|q| !q.is_empty()) {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = u.fragment().filter(|f| !f.is_empty()) {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Wildcard match where only `*` is special (any run of characters); everything else,
/// including `?`, is literal. Case-insensitive, whole string. Iterative with backtracking
/// to the last star only: O(pattern × text) at worst, no regex.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Longest title a title pattern is run against, in UTF-16 code units (like JS strings).
pub const MAX_TITLE_MATCH_LENGTH: usize = 256;
/// Longest title pattern accepted, in UTF-16 code units.
pub const MAX_REGEX_LENGTH: usize = 200;
/// Most backtracking paths a title pattern may have from one start position (see
/// [`backtracking_cost`]): two unbounded repeats (`.*x.*`).
pub const MAX_PATTERN_COST: u64 = 256 * 256;
const COST_CAP: u64 = 1 << 32;

/// Inline modifiers (`(?i)`, `(?i:…)`, `(?-s)`) are valid in some JS engines (Chrome 125+)
/// and not others; both sides refuse them so a rule never means different things.
fn has_inline_flags(pattern: &str) -> bool {
    let b = pattern.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == b'(' && b[i + 1] == b'?' && (b[i + 2].is_ascii_alphabetic() || b[i + 2] == b'-')
        {
            return true;
        }
        i += 1;
    }
    false
}

/// A choice point's weight (see [`backtracking_cost`]).
fn choice(n: u64) -> u64 {
    if n > 1 { n.max(4) } else { n }
}

/// `{n}`, `{n,}`, `{n,m}` at the start of `rest`: its length and how many repetition
/// counts it allows (capped at the title length).
fn braced_factor(rest: &[char]) -> Option<(usize, u64)> {
    let number = |from: usize| {
        let len = rest[from..].iter().take_while(|c| c.is_ascii_digit()).count();
        let value = rest[from..from + len].iter().fold(0u64, |acc, c| {
            acc.saturating_mul(10).saturating_add(u64::from(c.to_digit(10).unwrap_or(0)))
        });
        (len, value.min(1_000_000))
    };
    let (n_len, n) = number(1);
    if n_len == 0 {
        return None;
    }
    let mut j = 1 + n_len;
    let mut factor = 1;
    if rest.get(j) == Some(&',') {
        let (m_len, m) = number(j + 1);
        factor = if m_len == 0 { 256 } else { choice((m.saturating_sub(n) + 1).min(256)) };
        j += 1 + m_len;
    }
    (rest.get(j) == Some(&'}')).then_some((j + 1, factor))
}

/// An upper bound on the work a backtracking engine can do from one start position: a
/// sequence multiplies its items' choices, an alternation adds its branches', `{n,m}` is
/// m − n + 1 choices, and `*` / `+` / `{n,}` are up to the title length (256). Every choice
/// point counts at least 4 (`?`, `{1,2}`, a group with alternatives): chains of small
/// choices have long paths (`a?` × 17 takes ~0.5 s at 2¹⁷ paths). Repeated groups and
/// back-references are refused separately, so this bounds the work. Same computation as
/// the extension's `backtrackingCost`.
pub fn backtracking_cost(pattern: &str) -> u64 {
    let chars: Vec<char> = pattern.chars().collect();
    let mut stack: Vec<(u64, u64)> = Vec::new();
    let (mut sum, mut seq) = (0u64, 1u64);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let factor = match ch {
            '\\' => {
                i += 2;
                continue;
            }
            '[' => {
                i += 1;
                if chars.get(i) == Some(&'^') {
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            '(' => {
                stack.push((sum, seq));
                (sum, seq) = (0, 1);
                i += if chars.get(i + 1) == Some(&'?') { 3 } else { 1 };
                continue;
            }
            '|' => {
                sum = (sum + seq).min(COST_CAP);
                seq = 1;
                i += 1;
                continue;
            }
            ')' => {
                let inner = (sum + seq).min(COST_CAP);
                if let Some((outer_sum, outer_seq)) = stack.pop() {
                    sum = outer_sum;
                    seq = outer_seq.saturating_mul(choice(inner)).min(COST_CAP);
                }
                i += 1;
                continue;
            }
            '?' => Some((1, 4)),
            '*' | '+' => Some((1, 256)),
            '{' => braced_factor(&chars[i..]),
            _ => None,
        };
        match factor {
            Some((len, f)) => {
                seq = seq.saturating_mul(f).min(COST_CAP);
                i += len;
                if chars.get(i) == Some(&'?') {
                    i += 1; // lazy
                }
            }
            None => i += 1,
        }
    }
    (sum + seq).min(COST_CAP)
}

/// The compiled title pattern, or why it's refused. Same checks, in the same order, as the
/// extension's `titleRegexProblem` (the patterns run on backtracking engines in both,
/// against titles that websites control): too long, inline flags, invalid,
/// back-references, lookarounds (and `(?<`), quantified groups, and too many paths.
fn checked_regex(pattern: &str) -> Result<regress::Regex, &'static str> {
    if pattern.encode_utf16().count() > MAX_REGEX_LENGTH {
        return Err("Pattern is too long");
    }
    if has_inline_flags(pattern) {
        return Err("Inline flags are not supported");
    }
    let re = js_regex::translate(pattern)
        .and_then(|p| regress::Regex::with_flags(&p, "").ok())
        .ok_or("Invalid title pattern")?;
    let chars: Vec<char> = pattern.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' {
            if at(i + 1).is_some_and(|n| matches!(n, '1'..='9' | 'k')) {
                return Err("Back-references are not supported");
            }
            i += 2;
            continue;
        }
        if in_class {
            if ch == ']' {
                in_class = false;
            }
        } else if ch == '[' {
            in_class = true;
        } else if ch == '(' && at(i + 1) == Some('?') && matches!(at(i + 2), Some('=' | '!' | '<'))
        {
            // Like the extension, `(?<…` is refused whether it's a lookbehind or a named group.
            return Err("Lookarounds are not supported");
        } else if ch == ')' && matches!(at(i + 1), Some('*' | '+' | '?' | '{')) {
            return Err("Repeated groups are not supported");
        }
        i += 1;
    }
    if backtracking_cost(pattern) > MAX_PATTERN_COST {
        return Err("Pattern is too complex");
    }
    Ok(re)
}

/// Why a title pattern is refused, or `None` if it's fine (then it matches exactly like
/// the extension's `new RegExp(pattern, 'i')`; see [`js_regex`]).
pub fn title_regex_problem(pattern: &str) -> Option<&'static str> {
    checked_regex(pattern).err()
}

const CACHE_LIMIT: usize = 5000;

type RegexCache = Mutex<HashMap<String, Option<Arc<regress::Regex>>>>;
static REGEX_CACHE: LazyLock<RegexCache> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Compiles a safe title pattern (cached, shared across threads); `None` if invalid or
/// refused, and then it never matches.
fn compile_title_regex(pattern: &str) -> Option<Arc<regress::Regex>> {
    let lock = || REGEX_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(hit) = lock().get(pattern) {
        return hit.clone();
    }
    let re = checked_regex(pattern).ok().map(Arc::new); // compiled without holding the lock
    let mut cache = lock();
    if cache.len() >= CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(pattern.to_owned(), re.clone());
    re
}

/// Whether a title pattern matches a title (its first 256 UTF-16 code units).
pub fn title_matches(pattern: &str, title: &str) -> bool {
    compile_title_regex(pattern).is_some_and(|re| {
        let mut units = js_regex::canonical_units(title);
        units.truncate(MAX_TITLE_MATCH_LENGTH);
        re.find_from_ucs2(&units, 0).next().is_some()
    })
}

/// Whether an enabled rule matches the target.
pub fn rule_matches(rule: &Rule, target: &MatchTarget<'_>) -> bool {
    if !rule.enabled {
        return false;
    }
    match rule.scope {
        RuleScope::Global => true,
        RuleScope::App => rule.pattern == target.app_id,
        RuleScope::Domain => domain_matches(target.url.and_then(host_of).as_deref(), &rule.pattern),
        RuleScope::UrlGlob => {
            target.url.is_some_and(|u| glob_matches(&rule.pattern, &glob_subject(u)))
        }
        RuleScope::TitleRegex => title_matches(&rule.pattern, target.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RuleAction, RuleSource};
    use std::time::{Duration, Instant};

    fn rule(scope: RuleScope, pattern: &str, enabled: bool) -> Rule {
        Rule {
            id: "r".into(),
            scope,
            pattern: pattern.into(),
            action: RuleAction::Protect,
            threshold_hours: None,
            enabled,
            source: RuleSource::User,
            created_at: 0,
            hit_count: 0,
        }
    }
    fn target<'a>(url: Option<&'a str>, title: &'a str) -> MatchTarget<'a> {
        MatchTarget { app_id: "com.google.Chrome", url, title }
    }

    #[test]
    fn extracts_hosts_from_web_urls_only() {
        assert_eq!(host_of("https://Docs.GitHub.com./x").as_deref(), Some("docs.github.com"));
        assert_eq!(host_of("chrome://settings"), None);
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn matches_a_domain_and_its_subdomains_not_look_alikes() {
        assert!(domain_matches(Some("github.com"), "github.com"));
        assert!(domain_matches(Some("gist.github.com"), "github.com"));
        assert!(domain_matches(Some("gist.github.com"), "*.github.com"));
        assert!(!domain_matches(Some("notgithub.com"), "github.com"));
        assert!(!domain_matches(Some("github.com.evil.io"), "github.com"));
        assert!(!domain_matches(None, "github.com"));
    }

    #[test]
    fn matches_ip_addresses_exactly() {
        assert!(!domain_matches(Some("192.168.1.1"), "1.1"));
        assert!(!domain_matches(Some("10.0.0.1"), "0.1"));
        assert!(domain_matches(Some("10.0.0.1"), "10.0.0.1"));
    }

    #[test]
    fn matches_globs_with_only_star_as_a_wildcard() {
        assert!(glob_matches("https://*.notion.so/*", "https://acme.notion.so/page"));
        assert!(!glob_matches("https://*.notion.so/*", "https://notion.so.evil/x"));
        assert!(glob_matches("https://a.com/?q=1", "https://a.com/?q=1")); // ? is literal
        assert!(!glob_matches("https://a.com/?q=1", "https://a.com/xq=1"));
        assert!(glob_matches("https://a.com/(x)+", "https://A.com/(x)+"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("a*b*c", "axxbyyc"));
        assert!(!glob_matches("a*b*c", "axxbyy"));
    }

    #[test]
    fn globs_ignore_userinfo_tricks() {
        let r = rule(RuleScope::UrlGlob, "https://github.com*", true);
        assert!(!rule_matches(&r, &target(Some("https://github.com@evil.com/"), "")));
        assert!(rule_matches(&r, &target(Some("https://github.com/x"), "")));
    }

    #[test]
    fn glob_subject_keeps_port_query_and_fragment() {
        assert_eq!(
            glob_subject("https://user:pw@a.com:8443/p?q=1#h"),
            "https://a.com:8443/p?q=1#h"
        );
        assert_eq!(glob_subject("https://a.com/?"), "https://a.com/");
        assert_eq!(glob_subject("chrome://settings"), "chrome://settings");
    }

    #[test]
    fn globs_stay_fast_on_long_non_matching_urls() {
        let r = rule(RuleScope::UrlGlob, "https://*a*b*c*d*e*f*g*h*zzz", true);
        let url = format!("https://example.com/{}", "abcdefgh".repeat(300));
        let start = Instant::now();
        assert!(!rule_matches(&r, &target(Some(&url), "")));
        // Generous: debug builds on shared CI runners.
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn refuses_title_patterns_that_can_backtrack_catastrophically() {
        for bad in ["(a+)+$", "(a|aa)*", r"(\w+\s?)*x", "(.*)*", r"(a)\1", "(?=a)a", ".*.*.*x"] {
            assert!(title_regex_problem(bad).is_some(), "{bad}");
        }
        for ok in ["^Inbox", "jira|confluence", r"\(\d+\) unread", ".*bank.*", "[a-z]{2,5}"] {
            assert_eq!(title_regex_problem(ok), None, "{ok}");
        }
        let title = format!("{}!", "a".repeat(40));
        let start = Instant::now();
        assert!(!rule_matches(
            &rule(RuleScope::TitleRegex, "(a+)+$", true),
            &target(Some("x"), &title)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn refuses_bounded_repeats_that_multiply_into_too_many_paths() {
        for bad in [
            "a{1,30}a{1,30}a{1,30}a{1,30}a{1,30}$",
            r"\w{1,40}\w{1,40}\w{1,40}\w{1,40}\w{1,40}!",
            "a?a?a?a?a?a?a?a?a?aaaaaaaaaaaaaaaaaa",
            &"(a|a)".repeat(9),
            ".*a.*c?x",
        ] {
            assert_eq!(title_regex_problem(bad), Some("Pattern is too complex"), "{bad}");
        }
        assert_eq!(backtracking_cost(".*x.*"), 256 * 256);
        assert_eq!(backtracking_cost("a{1,30}b{2}c?"), 30 * 4);
        assert_eq!(backtracking_cost("colou?r|a{1,2}"), 4 + 4);
        assert_eq!(backtracking_cost("jira|confluence|(a|b)c+"), 2 + 4 * 256);
        assert_eq!(backtracking_cost("a+?b*?"), 256 * 256); // lazy `?` isn't a choice
        // The worst accepted patterns still finish quickly on a hostile title (~10–30 ms
        // each in release builds; generous here for debug builds on shared CI runners).
        let title = format!("{}b", "a".repeat(255));
        for ok in [
            "a{1,30}a{1,30}a{1,30}$",
            r"\w{1,40}\w{1,40}\w{1,40}$",
            ".*a.*x",
            "a?a?a?a?a?a?a?a?aaaaaaaax",
            &"(a|a)".repeat(8),
        ] {
            assert!(backtracking_cost(ok) <= MAX_PATTERN_COST, "{ok}");
            let start = Instant::now();
            title_matches(ok, &title);
            assert!(start.elapsed() < Duration::from_secs(2), "{ok}");
        }
    }

    #[test]
    fn quantified_assertions_are_invalid_as_in_js() {
        for bad in [r"\b+", r"x\B?", r"\b{2}", "^*", "$+", "x{2,1}"] {
            assert_eq!(title_regex_problem(bad), Some("Invalid title pattern"), "{bad}");
        }
        assert_eq!(title_regex_problem(r"\b{"), None); // `{` is a literal there
    }

    #[test]
    fn class_ranges_fold_like_js() {
        // (pattern, title, what `new RegExp(p, 'i')` says)
        for (p, t, want) in [
            ("[ą-ż]", "Yazı", true),
            ("[ı-ł]", "ł", true),
            ("[Ā-ɏ]", "S", false),
            ("[℀-⅏]", "k", false),
            ("[℀-⅏]", "\u{212A}", true),
            ("[℀-⅏]", "K", false),
            ("[^\\u0080-\\uffff]", "k", true),
            ("[İ-Ĳ]", "ı", true),
            ("[Ẑ-Ạ]", "ß", false),
            ("ᾆ", "ᾎ", false),
            ("[ϴ]", "θ", false),
        ] {
            assert_eq!(title_matches(p, t), want, "{p} on {t:?}");
        }
        assert_eq!(title_regex_problem("[ſ-ƀ]"), None);
    }

    #[test]
    fn unicode_rule_domains_match_their_punycode_hosts() {
        assert!(domain_matches(Some("xn--bcher-kva.de"), "bücher.de"));
        assert!(domain_matches(Some("shop.xn--bcher-kva.de"), "*.Bücher.de."));
        assert!(domain_matches(Some("kakao.com"), "\u{212A}akao.com")); // Kelvin sign, as JS
    }

    #[test]
    fn matches_each_scope_and_ignores_disabled_rules() {
        let t = target(Some("https://app.figma.com/file/1"), "Design — Figma");
        assert!(rule_matches(&rule(RuleScope::Global, "*", true), &t));
        assert!(rule_matches(&rule(RuleScope::App, "com.google.Chrome", true), &t));
        assert!(!rule_matches(&rule(RuleScope::App, "com.microsoft.Edge", true), &t));
        assert!(rule_matches(&rule(RuleScope::Domain, "figma.com", true), &t));
        assert!(rule_matches(&rule(RuleScope::UrlGlob, "https://app.figma.com/file/*", true), &t));
        assert!(rule_matches(&rule(RuleScope::TitleRegex, "figma$", true), &t));
        assert!(!rule_matches(&rule(RuleScope::TitleRegex, "(unclosed", true), &t));
        assert!(!rule_matches(&rule(RuleScope::Domain, "figma.com", false), &t));
    }

    #[test]
    fn title_patterns_follow_js_non_unicode_semantics() {
        // (pattern, title, what the extension's `new RegExp(p, 'i')` says)
        for (p, t, want) in [
            (r"\w+", "é", false),
            (r"\bcafé\b", "le café!", false),
            ("^.$", "𝒜", false),
            (r"^\d$", "٣", false),
            ("ſ", "S", false),
            ("k", "\u{212A}", false),
            (r"[^\W]", "S", true),
            (r"\w{3,30}", "Q3 plan", true),
            ("{draft}", "{draft} Q3", true),
            ("😀", "😀😀 party", true),
            ("^😀{2}", "😀😀", false),
            (r"\u{41}", &"u".repeat(41), true),
            ("résumé", "RÉSUMÉ", true),
        ] {
            assert_eq!(title_matches(p, t), want, "{p} on {t:?}");
        }
        assert_eq!(title_regex_problem("(?i)abc"), Some("Inline flags are not supported"));
        // Without `u`, `[a-😀]` is `a`…U+D83D plus U+DE00, as in JS.
        assert_eq!(title_regex_problem("[a-😀]"), None);
        assert!(!title_matches("[a-😀]", "8"));
        assert!(title_matches("[a-😀]", "é"));
        assert_eq!(title_regex_problem(&"😀".repeat(101)), Some("Pattern is too long"));
    }

    #[test]
    fn title_patterns_only_see_the_first_256_utf16_units() {
        let title = format!("{}needle", "x".repeat(300));
        assert!(!rule_matches(&rule(RuleScope::TitleRegex, "needle", true), &target(None, &title)));
        // 256 UTF-16 code units, like JS: 127 astral characters leave room for "ne" only.
        let astral = format!("{}needle", "𝒜".repeat(127));
        assert!(!title_matches("needle", &astral));
        assert!(title_matches("ne$", &astral));
    }
}
