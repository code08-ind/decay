//! Localization of the app's own text (P2.7, PRD F31): the tray menu, notifications,
//! native dialogs, and the words the engine's data is shown with (reasons, labels,
//! errors). The windows have their own catalogs (`apps/desktop/src/strings.ts`).
//!
//! Catalogs are JSON in `strings/` (English is complete; a translation may leave keys
//! out), compiled in. Messages use the windows' syntax, a small ICU subset:
//! `{name}`, `{n, plural, =0 {…} one {# tab} other {# tabs}}`,
//! `{kind, select, tab {…} other {…}}`. The locale follows the `locale` setting
//! (`system` = the OS language).

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// English, the source and the fallback.
const EN: &str = include_str!("../strings/en.json");

/// Translations: `(language tag, catalog)`.
const TRANSLATIONS: &[(&str, &str)] = &[];

fn parse(json: &str) -> HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

static CATALOGS: LazyLock<HashMap<&'static str, HashMap<String, String>>> = LazyLock::new(|| {
    let mut all: HashMap<&'static str, HashMap<String, String>> =
        TRANSLATIONS.iter().map(|(tag, json)| (*tag, parse(json))).collect();
    all.insert("en", parse(EN));
    all
});

static LOCALE: RwLock<String> = RwLock::new(String::new());

/// The OS language (`en-GB`), or English: macOS's first preferred language, Windows'
/// user locale, else `LANG`. Read once.
fn system_locale() -> String {
    static SYSTEM: LazyLock<String> = LazyLock::new(|| {
        os_locale()
            .or_else(|| std::env::var("LANG").ok().and_then(|l| tag_from(&l)))
            .unwrap_or_else(|| "en".into())
    });
    SYSTEM.clone()
}

/// `en_US.UTF-8` / `"en-IN"` → `en-US` / `en-IN`; `None` if it isn't a language tag.
fn tag_from(raw: &str) -> Option<String> {
    let tag = raw.trim().trim_matches('"').split(['.', '@']).next()?.replace('_', "-");
    let ok = !tag.is_empty()
        && tag.len() <= 35
        && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && tag != "C"
        && tag != "POSIX";
    ok.then_some(tag)
}

#[cfg(target_os = "macos")]
fn os_locale() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // `(\n    "en-IN",\n    "hi-IN"\n)`: the first entry.
    text.split('"').nth(1).and_then(tag_from)
}

#[cfg(windows)]
fn os_locale() -> Option<String> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Control Panel\\International")
        .ok()?;
    let name: String = key.get_value("LocaleName").ok()?;
    tag_from(&name)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_locale() -> Option<String> {
    None
}

/// Pseudo-locales (as in the windows): `en-XA` accents and lengthens every message,
/// `ar-XB` too, right to left.
const PSEUDO: [&str; 2] = ["en-XA", "ar-XB"];

/// The locale a tag resolves to: itself when its language has a catalog (or it's a
/// pseudo-locale), else English with the tag's region (`de-AT` → `en-AT`: English text,
/// no other language's plural rules). The windows resolve the same way.
fn resolve(tag: &str) -> String {
    if PSEUDO.contains(&tag) {
        return tag.to_owned();
    }
    let mut parts = tag.split('-');
    let lang = parts.next().unwrap_or("").to_ascii_lowercase();
    if CATALOGS.contains_key(lang.as_str()) {
        return tag.to_owned();
    }
    let region = parts.find(|p| {
        (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
            || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
    });
    region.map_or_else(|| "en".to_owned(), |r| format!("en-{}", r.to_ascii_uppercase()))
}

/// Follows the `locale` setting (`system`, a tag, or anything else → system).
pub fn set_locale(setting: Option<&str>) {
    let tag = match setting {
        Some(s) if s != "system" && !s.is_empty() => s.to_owned(),
        _ => system_locale(),
    };
    *LOCALE.write().unwrap_or_else(std::sync::PoisonError::into_inner) = resolve(&tag);
}

/// The OS language as the app reads it, for the windows' `system` (a web view may
/// report its own), so the tray and the windows agree.
#[tauri::command]
pub fn system_language() -> String {
    system_locale()
}

/// The current locale (`en` until set).
pub fn locale() -> String {
    let l = LOCALE.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    if l.is_empty() { "en".into() } else { l.clone() }
}

/// The message for `key`: full tag, then language, then English, then the key itself.
fn message(key: &str) -> String {
    let loc = locale();
    let lang = loc.split('-').next().unwrap_or("");
    for tag in [loc.as_str(), lang, "en"] {
        if let Some(m) = CATALOGS.get(tag).and_then(|c| c.get(key)) {
            return m.clone();
        }
    }
    key.to_owned()
}

/// A parameter: text or a number (numbers pick plural branches).
#[derive(Debug, Clone)]
pub enum Arg {
    Text(String),
    Num(f64),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Self::Text(s.to_owned())
    }
}
impl From<String> for Arg {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}
impl From<&String> for Arg {
    fn from(s: &String) -> Self {
        Self::Text(s.clone())
    }
}
macro_rules! num_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            #[allow(clippy::cast_precision_loss, clippy::cast_lossless, reason = "counts")]
            fn from(n: $t) -> Self { Self::Num(n as f64) }
        }
    )*};
}
num_arg!(u8, u16, u32, u64, usize, i32, i64, f64);

/// `t("key")`, or `t("key", &[("n", 3.into())])`.
pub fn t(key: &str) -> String {
    localized(format(&message(key), &[]))
}

pub fn tf(key: &str, args: &[(&str, Arg)]) -> String {
    localized(format(&message(key), args))
}

/// The pseudo-locale form when one is on: accented, ~30 % longer, bracketed.
fn localized(text: String) -> String {
    if PSEUDO.contains(&locale().as_str()) { pseudo(&text) } else { text }
}

fn pseudo(text: &str) -> String {
    const FROM: &str = "abcdefghijklmnoprstuwyzACDEINOSU";
    const TO: [char; 32] = [
        'å', 'б', 'ç', 'ð', 'é', 'ƒ', 'ĝ', 'ĥ', 'î', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ö', 'þ', 'ŕ', 'š',
        'ţ', 'û', 'ŵ', 'ý', 'ž', 'Å', 'Ç', 'Ð', 'É', 'Î', 'Ñ', 'Ö', 'Ŝ', 'Û',
    ];
    let accented: String = text.chars().map(|c| FROM.find(c).map_or(c, |i| TO[i])).collect();
    let n = text.chars().count();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let pad = ((n as f64 * 0.3).round() as usize).max(1);
    format!("[{accented} {}]", "~".repeat(pad))
}

/// A number as text, grouped as the locale writes it (as `Intl.NumberFormat` does for
/// the main language families): `1,234` in English, `1.234` in German, `1 234` in
/// French; whole numbers without decimals.
fn num_text(n: f64) -> String {
    let loc = locale();
    let lang = loc.split('-').next().unwrap_or("en");
    let (group, decimal, min_group) = match lang {
        "pt" if loc.starts_with("pt-PT") => ("\u{a0}", ",", 2),
        "de" | "it" | "nl" | "id" | "tr" | "da" | "el" | "ro" | "hr" | "sl" | "sr" | "pt" => {
            (".", ",", 1)
        }
        "es" => (".", ",", 2),
        "fr" => ("\u{202f}", ",", 1),
        "pl" => ("\u{a0}", ",", 2),
        "ru" | "uk" | "be" | "cs" | "sk" | "sv" | "nb" | "no" | "fi" | "bg" | "hu" => {
            ("\u{a0}", ",", 1)
        }
        _ => (",", ".", 1),
    };
    #[allow(clippy::cast_possible_truncation, reason = "checked whole")]
    let raw = if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", (n * 1000.0).round() / 1000.0)
    };
    let (sign, rest) = raw.strip_prefix('-').map_or(("", raw.as_str()), |r| ("-", r));
    let (int, frac) = rest.split_once('.').map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    if int.len() > 3 + 3 * (min_group - 1) {
        for (i, ch) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                out.push_str(group);
            }
            out.push(ch);
        }
    } else {
        out.push_str(int);
    }
    if let Some(f) = frac {
        out.push_str(decimal);
        out.push_str(f);
    }
    format!("{sign}{out}")
}

/// CLDR plural category for the common language families (others like English);
/// the same answers `Intl.PluralRules` gives the windows.
fn plural_category(n: f64, locale: &str) -> &'static str {
    let lang = locale.split('-').next().unwrap_or("en");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "counts")]
    let i = n.abs().trunc() as u64;
    let whole = n.fract() == 0.0; // CLDR's v = 0
    let (last, last2) = (i % 10, i % 100);
    let few = (2..=4).contains(&last) && !(12..=14).contains(&last2);
    match lang {
        "ja" | "zh" | "ko" | "th" | "vi" | "id" | "ms" | "tr" => "other",
        // pt-PT: one for 1 only; pt (Brazil), fr: 0 and 1 (and 1.5); many for millions.
        "pt" if locale.starts_with("pt-PT") => {
            if i == 1 && whole {
                "one"
            } else if whole && i != 0 && i.is_multiple_of(1_000_000) {
                "many"
            } else {
                "other"
            }
        }
        "fr" | "pt" => {
            if i <= 1 {
                "one"
            } else if whole && i.is_multiple_of(1_000_000) {
                "many"
            } else {
                "other"
            }
        }
        _ if !whole => match lang {
            "cs" | "sk" | "pl" | "ru" | "uk" | "be" => "many",
            _ => "other",
        },
        "cs" | "sk" => match i {
            1 => "one",
            2..=4 => "few",
            _ => "other",
        },
        "he" | "iw" => match i {
            1 => "one",
            2 => "two",
            _ => "other",
        },
        "pl" => {
            if i == 1 {
                "one"
            } else if few {
                "few"
            } else {
                "many"
            }
        }
        "ru" | "uk" | "be" => {
            if last == 1 && last2 != 11 {
                "one"
            } else if few {
                "few"
            } else {
                "many"
            }
        }
        "ar" => match (i, last2) {
            (0, _) => "zero",
            (1, _) => "one",
            (2, _) => "two",
            (_, 3..=10) => "few",
            (_, 11..=99) => "many",
            _ => "other",
        },
        _ => {
            if i == 1 {
                "one"
            } else {
                "other"
            }
        }
    }
}

/// Formats a message; a malformed one comes back as written (never a failure).
pub fn format(message: &str, args: &[(&str, Arg)]) -> String {
    let chars: Vec<char> = message.chars().collect();
    let mut pos = 0;
    match render(&chars, &mut pos, args, None, false) {
        Some(out) if pos == chars.len() => out,
        _ => message.to_owned(),
    }
}

fn arg<'a>(args: &'a [(&str, Arg)], name: &str) -> Option<&'a Arg> {
    args.iter().find(|(n, _)| *n == name).map(|(_, a)| a)
}

/// Renders until the end, or the `}` closing a branch (`in_branch`).
fn render(
    c: &[char],
    pos: &mut usize,
    args: &[(&str, Arg)],
    hash: Option<&str>,
    in_branch: bool,
) -> Option<String> {
    let mut out = String::new();
    while *pos < c.len() {
        match c[*pos] {
            '\'' if c.get(*pos + 1) == Some(&'\'') => {
                out.push('\'');
                *pos += 2;
            }
            '}' if in_branch => return Some(out),
            '}' => return None,
            '#' if hash.is_some() => {
                out.push_str(hash.unwrap_or_default());
                *pos += 1;
            }
            '{' => {
                *pos += 1;
                out.push_str(&render_arg(c, pos, args, hash)?);
            }
            ch => {
                out.push(ch);
                *pos += 1;
            }
        }
    }
    if in_branch { None } else { Some(out) }
}

/// `{name}` or `{name, plural|select, …}` after its `{`.
fn render_arg(
    c: &[char],
    pos: &mut usize,
    args: &[(&str, Arg)],
    hash: Option<&str>,
) -> Option<String> {
    let start = *pos;
    while *pos < c.len() && !matches!(c[*pos], ',' | '}') {
        *pos += 1;
    }
    let name: String = c[start..*pos].iter().collect::<String>().trim().to_owned();
    if name.is_empty() || !name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
        return None;
    }
    if c.get(*pos) == Some(&'}') {
        *pos += 1;
        return Some(match arg(args, &name) {
            Some(Arg::Text(s)) => s.clone(),
            Some(Arg::Num(n)) => num_text(*n),
            None => String::new(),
        });
    }
    *pos += 1; // ','
    let kind_start = *pos;
    while *pos < c.len() && c[*pos] != ',' {
        *pos += 1;
    }
    let kind: String = c[kind_start..*pos].iter().collect::<String>().trim().to_owned();
    if c.get(*pos) != Some(&',') || !matches!(kind.as_str(), "plural" | "select") {
        return None;
    }
    *pos += 1;
    let value = arg(args, &name);
    let num = match value {
        Some(Arg::Num(n)) => Some(*n),
        Some(Arg::Text(s)) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
        None => None,
    };
    let text_value = match value {
        Some(Arg::Text(s)) => s.clone(),
        Some(Arg::Num(n)) => num_text(*n),
        None => String::new(),
    };
    let mut chosen: Option<String> = None;
    let mut other: Option<String> = None;
    loop {
        while c.get(*pos).is_some_and(|ch| ch.is_whitespace()) {
            *pos += 1;
        }
        if c.get(*pos) == Some(&'}') {
            *pos += 1;
            break;
        }
        let key_start = *pos;
        while *pos < c.len() && !c[*pos].is_whitespace() && c[*pos] != '{' {
            *pos += 1;
        }
        let key: String = c[key_start..*pos].iter().collect();
        while c.get(*pos).is_some_and(|ch| ch.is_whitespace()) {
            *pos += 1;
        }
        if c.get(*pos) != Some(&'{') || key.is_empty() {
            return None;
        }
        *pos += 1;
        // `#`: the number, formatted; a missing or non-numeric value as given.
        let hash_text = num.map_or_else(|| text_value.clone(), num_text);
        let branch_hash = if kind == "plural" { Some(hash_text.as_str()) } else { hash };
        let text = render(c, pos, args, branch_hash, true)?;
        if c.get(*pos) != Some(&'}') {
            return None;
        }
        *pos += 1;
        let matches = if kind == "plural" {
            // No number: only `other` (as in the windows).
            num.is_some_and(|n| {
                if let Some(exact) = key.strip_prefix('=') {
                    exact.parse::<f64>().ok() == Some(n)
                } else {
                    key == plural_category(n, &locale())
                }
            })
        } else {
            key == text_value
        };
        if key == "other" {
            other = Some(text.clone());
        }
        // An exact `=n` beats a category: keep the first exact, else first match.
        if matches && (chosen.is_none() || key.starts_with('=')) {
            chosen = Some(text);
        }
    }
    chosen.or(other)
}

#[cfg(test)]
pub(crate) fn english() -> HashMap<String, String> {
    parse(EN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(locale: &str, f: impl FnOnce()) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_locale(Some(locale));
        f();
        set_locale(Some("en"));
    }

    #[test]
    fn formats_parameters_plurals_and_selects() {
        with("en", || {
            assert_eq!(format("Hello {name}", &[("name", "Ada".into())]), "Hello Ada");
            let m = "{n, plural, =0 {Nothing} one {# tab} other {# tabs}} open";
            assert_eq!(format(m, &[("n", 0u32.into())]), "Nothing open");
            assert_eq!(format(m, &[("n", 1u32.into())]), "1 tab open");
            assert_eq!(format(m, &[("n", 12u32.into())]), "12 tabs open");
            let s = "{k, select, tab {a tab} other {an item}}";
            assert_eq!(format(s, &[("k", "tab".into())]), "a tab");
            assert_eq!(format(s, &[("k", "x".into())]), "an item");
            assert_eq!(format("It''s {x}", &[("x", "ok".into())]), "It's ok");
            assert_eq!(format("Can't", &[]), "Can't");
            let nested = "{k, select, a {{n, plural, one {# x} other {# xs}} of {who}} other {-}}";
            assert_eq!(
                format(nested, &[("k", "a".into()), ("n", 2u32.into()), ("who", "us".into())]),
                "2 xs of us"
            );
        });
    }

    #[test]
    fn a_broken_message_comes_back_as_written() {
        for bad in ["{", "a}", "{n, plural, one {x}", "{n, nope, other {x}}", "{a b}"] {
            assert_eq!(format(bad, &[("n", 1u32.into())]), bad);
        }
    }

    #[test]
    fn reads_language_tags() {
        assert_eq!(tag_from("en_US.UTF-8").as_deref(), Some("en-US"));
        assert_eq!(tag_from("\"pt-BR\"").as_deref(), Some("pt-BR"));
        assert_eq!(tag_from("C"), None);
        assert_eq!(tag_from(""), None);
        assert_ne!(system_locale(), "");
    }

    #[test]
    fn plural_rules_follow_the_language() {
        assert_eq!(plural_category(1.0, "en"), "one");
        assert_eq!(plural_category(0.0, "fr"), "one");
        assert_eq!(plural_category(3.0, "ru"), "few");
        assert_eq!(plural_category(11.0, "ru"), "many");
        assert_eq!(plural_category(1.0, "ja"), "other");
        assert_eq!(plural_category(22.0, "pl"), "few");
        assert_eq!(plural_category(2.0, "ar"), "two");
        // As `Intl.PluralRules`: whole numbers in Czech are never `many`.
        assert_eq!(plural_category(0.0, "cs"), "other");
        assert_eq!(plural_category(5.0, "cs"), "other");
        assert_eq!(plural_category(1.5, "cs"), "many");
        assert_eq!(plural_category(0.0, "pt-PT"), "other");
        assert_eq!(plural_category(0.0, "pt-BR"), "one");
        assert_eq!(plural_category(1_000_000.0, "fr"), "many");
        assert_eq!(plural_category(1.5, "fr"), "one");
        assert_eq!(plural_category(2.0, "he"), "two");
        assert_eq!(plural_category(21.0, "pl"), "many");
        assert_eq!(plural_category(101.0, "ru"), "one");
    }

    #[test]
    fn numbers_are_grouped_for_the_locale() {
        with("en", || {
            assert_eq!(num_text(1234.0), "1,234");
            assert_eq!(num_text(-1_234_567.0), "-1,234,567");
            assert_eq!(num_text(999.0), "999");
            assert_eq!(num_text(1.5), "1.5");
            let m = "{n, plural, one {# tab} other {# tabs}}";
            assert_eq!(format(m, &[("n", 1234u32.into())]), "1,234 tabs");
        });
    }

    #[test]
    fn a_missing_number_takes_the_other_branch() {
        with("en", || {
            let m = "{n, plural, =0 {none} one {# tab} other {# tabs}}";
            assert_eq!(format(m, &[]), " tabs");
            assert_eq!(format(m, &[("n", "lots".into())]), "lots tabs");
        });
    }

    #[test]
    fn a_language_without_a_catalog_is_english_with_its_region() {
        assert_eq!(resolve("en-GB"), "en-GB");
        assert_eq!(resolve("fr-FR"), "en-FR");
        assert_eq!(resolve("ar"), "en");
        assert_eq!(resolve("zh-Hant-TW"), "en-TW");
        assert_eq!(resolve("es-419"), "en-419");
        assert_eq!(resolve("ar-XB"), "ar-XB");
        // English plural rules then, not French ones ("0 tabs", not "0 tab").
        with("fr-FR", || {
            assert_eq!(locale(), "en-FR");
            let m = "{n, plural, one {# tab} other {# tabs}}";
            assert_eq!(format(m, &[("n", 0u32.into())]), "0 tabs");
        });
    }

    #[test]
    fn pseudo_locales_mark_every_message() {
        with("en-XA", || {
            assert_eq!(t("tray.quit"), "[Qûîţ Ðéçåý ~~~]");
        });
        with("ar-XB", || assert!(t("tray.quit").starts_with("[Qûîţ")));
    }

    #[test]
    fn falls_back_to_english_then_the_key() {
        with("de-AT", || {
            assert_eq!(t("tray.quit"), "Quit Decay");
            assert_eq!(t("no.such.key"), "no.such.key");
        });
    }

    #[test]
    fn every_english_message_is_well_formed() {
        for (key, m) in english() {
            let chars: Vec<char> = m.chars().collect();
            let mut pos = 0;
            let ok = render(&chars, &mut pos, &[], None, false).is_some() && pos == chars.len();
            assert!(ok, "{key}: {m}");
        }
    }

    /// Every `t("…")` / `tf("…"` key in the app's sources is in the English catalog.
    #[test]
    fn every_key_used_exists() {
        let en = english();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for pat in ["t(\"", "tf(\""] {
                for (i, _) in src.match_indices(pat) {
                    let before = src[..i].chars().last();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue; // another function ending in t / tf
                    }
                    let rest = &src[i + pat.len()..];
                    let key = &rest[..rest.find('"').unwrap_or(0)];
                    if key.contains('.') && !en.contains_key(key) && key != "no.such.key" {
                        missing.push(format!("{}: {key}", path.display()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
    }
}
