//! JS title patterns on `regress`, exactly as the extension runs them:
//! `new RegExp(pattern, 'i')` without the `u` flag, over UTF-16 code units.
//!
//! JS's case-insensitive matching compares `Canonicalize(ch)` of pattern and input
//! (ECMA-262 §22.2.2.7.3: a code unit's single-unit `toUpperCase`, never mapping non-ASCII
//! onto ASCII). Instead of trusting another engine's case folding, the title is mapped
//! through that exact function ([`crate::js_case`], generated from Node), and the pattern is
//! rewritten to match canonical text case-sensitively: literals become their canonical
//! unit, and every character class becomes the set of its members' canonical units (a
//! class matches `ch` iff some member has the same canonical form, so this is exact for
//! ranges and negated classes too).
//!
//! The rewrite also closes the syntax gaps between JS (non-`u`, Annex B) and `regress`:
//! characters outside the BMP are two code units, `\u`/`\x`/`\c` without valid digits are
//! literals, legacy octal escapes, `\b`/`^`/`$` can't be quantified, and reversed ranges are
//! errors. Checked against the extension by `tests/policy_parity.rs`.

#![allow(clippy::many_single_char_names, reason = "parser indices and code units: p, i, j, u")]

use std::fmt::Write as _;

use crate::js_case::{CANONICAL, WHITE_SPACE};

/// JS `Canonicalize` for a non-`u`, case-insensitive pattern.
pub(crate) fn canonicalize(unit: u16) -> u16 {
    CANONICAL.binary_search_by_key(&unit, |&(from, _)| from).map_or(unit, |i| CANONICAL[i].1)
}

/// The text as the rewritten pattern sees it: canonical UTF-16 code units.
pub(crate) fn canonical_units(text: &str) -> Vec<u16> {
    text.encode_utf16().map(canonicalize).collect()
}

/// Titles longer than this never match a count above it, so larger counts are clamped
/// (`regress` and JS disagree on huge ones; the meaning over a title can't change).
const MAX_COUNT: u64 = 257;

const WORD: [(u16, u16); 4] = [(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)];
const DIGIT: [(u16, u16); 1] = [(0x30, 0x39)];

const fn is(unit: u16, c: u8) -> bool {
    unit == c as u16
}

fn complement(ranges: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    let mut out = Vec::new();
    let mut next = 0u32;
    for (a, b) in sorted {
        if u32::from(a) > next {
            out.push((next as u16, a - 1));
        }
        next = next.max(u32::from(b) + 1);
    }
    if next <= 0xFFFF {
        out.push((next as u16, 0xFFFF));
    }
    out
}

/// `\d \D \s \S \w \W` as code-unit ranges.
fn class_escape(letter: u16) -> Option<Vec<(u16, u16)>> {
    let base: &[(u16, u16)] = match letter | 0x20 {
        0x64 => &DIGIT,      // d
        0x73 => WHITE_SPACE, // s
        0x77 => &WORD,       // w
        _ => return None,
    };
    Some(if letter & 0x20 == 0 { complement(base) } else { base.to_vec() })
}

const fn hex(unit: u16) -> Option<u16> {
    match unit {
        0x30..=0x39 => Some(unit - 0x30),
        0x41..=0x46 => Some(unit - 0x41 + 10),
        0x61..=0x66 => Some(unit - 0x61 + 10),
        _ => None,
    }
}

fn hex_value(p: &[u16], from: usize, digits: usize) -> Option<u16> {
    p.get(from..from + digits)?.iter().try_fold(0u16, |acc, &u| Some(acc * 16 + hex(u)?))
}

const fn octal(unit: Option<&u16>) -> Option<u16> {
    match unit {
        Some(&u) if u >= 0x30 && u <= 0x37 => Some(u - 0x30),
        _ => None,
    }
}

/// Annex B `LegacyOctalEscapeSequence` starting at `p[j]` (an octal digit): value, length.
fn legacy_octal(p: &[u16], j: usize) -> (u16, usize) {
    let d0 = octal(p.get(j)).unwrap_or(0);
    let Some(d1) = octal(p.get(j + 1)) else { return (d0, 1) };
    if d0 <= 3
        && let Some(d2) = octal(p.get(j + 2))
    {
        return (d0 * 64 + d1 * 8 + d2, 3);
    }
    (d0 * 8 + d1, 2)
}

/// The code unit of the character escape after a backslash at `p[j]`, and how many units
/// after the backslash it uses (0: the backslash alone is a literal `\`, as in `\c`
/// without a control letter).
fn char_escape(p: &[u16], j: usize, in_class: bool) -> (u16, usize) {
    let c = p[j];
    let ascii = u8::try_from(c).unwrap_or(0);
    match ascii {
        b'f' => (0x0C, 1),
        b'n' => (0x0A, 1),
        b'r' => (0x0D, 1),
        b't' => (0x09, 1),
        b'v' => (0x0B, 1),
        b'c' => match p.get(j + 1).and_then(|&u| u8::try_from(u).ok()) {
            Some(l)
                if l.is_ascii_alphabetic() || (in_class && (l.is_ascii_digit() || l == b'_')) =>
            {
                (u16::from(l % 32), 2)
            }
            _ => (u16::from(b'\\'), 0),
        },
        b'x' => hex_value(p, j + 1, 2).map_or((c, 1), |v| (v, 3)),
        b'u' => hex_value(p, j + 1, 4).map_or((c, 1), |v| (v, 5)),
        b'0'..=b'7' => legacy_octal(p, j),
        _ => (c, 1), // identity escape
    }
}

/// `{n}`, `{n,}` or `{n,m}` at `p[i]`: its length, `n`, and the maximum (`None`: open).
fn braced_quantifier(p: &[u16], i: usize) -> Option<(usize, u64, Option<u64>)> {
    let digits =
        |from: usize| p[from..].iter().take_while(|&&u| (0x30..=0x39).contains(&u)).count();
    let number = |from: usize, len: usize| {
        p[from..from + len]
            .iter()
            .fold(0u64, |acc, &u| acc.saturating_mul(10).saturating_add(u64::from(u - 0x30)))
    };
    if !is(*p.get(i)?, b'{') {
        return None;
    }
    let n_len = digits(i + 1);
    if n_len == 0 {
        return None;
    }
    let n = number(i + 1, n_len);
    let mut j = i + 1 + n_len;
    let max = if p.get(j).is_some_and(|&u| is(u, b',')) {
        let m_len = digits(j + 1);
        let m = (m_len > 0).then(|| number(j + 1, m_len));
        j += 1 + m_len;
        m
    } else {
        Some(n)
    };
    is(*p.get(j)?, b'}').then_some((j + 1 - i, n, max))
}

fn quantifier_at(p: &[u16], i: usize) -> bool {
    p.get(i).is_some_and(|&u| is(u, b'*') || is(u, b'+') || is(u, b'?'))
        || braced_quantifier(p, i).is_some()
}

/// Whether the pattern has a named group, which turns `\k` into a group reference.
fn has_named_groups(p: &[u16]) -> bool {
    p.windows(4).enumerate().any(|(i, w)| {
        is(w[0], b'(')
            && is(w[1], b'?')
            && is(w[2], b'<')
            && !is(w[3], b'=')
            && !is(w[3], b'!')
            && (i == 0 || !is(p[i - 1], b'\\'))
    })
}

fn push_escaped(out: &mut String, unit: u16) {
    let _ = write!(out, "\\u{unit:04X}");
}

/// A literal outside a class, canonicalised. `regress` joins an escaped high + low
/// surrogate pair into one character even in UCS-2 mode, but JS (non-`u`) sees two code
/// units, so surrogates are wrapped in their own groups (a following quantifier applies to
/// the low unit, as in JS).
fn push_literal(out: &mut String, unit: u16) {
    let unit = canonicalize(unit);
    if u8::try_from(unit).is_ok_and(|b| b.is_ascii_alphanumeric()) {
        out.push(char::from(unit as u8));
    } else if (0xD800..0xE000).contains(&unit) {
        out.push_str("(?:");
        push_escaped(out, unit);
        out.push(')');
    } else {
        push_escaped(out, unit);
    }
}

/// A class member: ASCII letters and digits as themselves, anything else escaped.
fn push_class_unit(out: &mut String, unit: u16) {
    match u8::try_from(unit) {
        Ok(b) if b.is_ascii_alphanumeric() => out.push(char::from(b)),
        _ => push_escaped(out, unit),
    }
}

enum ClassAtom {
    Unit(u16),
    Set(Vec<(u16, u16)>),
}

fn class_atom(p: &[u16], j: usize) -> Option<(ClassAtom, usize)> {
    if !is(p[j], b'\\') {
        return Some((ClassAtom::Unit(p[j]), j + 1));
    }
    let next = *p.get(j + 1)?;
    if is(next, b'b') {
        return Some((ClassAtom::Unit(0x08), j + 2));
    }
    if let Some(set) = class_escape(next) {
        return Some((ClassAtom::Set(set), j + 2));
    }
    let (unit, len) = char_escape(p, j + 1, true);
    Some((ClassAtom::Unit(unit), j + 1 + len))
}

/// Parses the class whose content starts at `p[start]` (after `[`) and emits it as the set
/// of its members' canonical units. Returns the index after `]`.
fn class(p: &[u16], start: usize, out: &mut String) -> Option<usize> {
    let mut j = start;
    let negate = p.get(j).is_some_and(|&u| is(u, b'^'));
    if negate {
        j += 1;
    }
    let mut ranges: Vec<(u16, u16)> = Vec::new();
    let add = |atom: ClassAtom, ranges: &mut Vec<(u16, u16)>| match atom {
        ClassAtom::Unit(u) => ranges.push((u, u)),
        ClassAtom::Set(set) => ranges.extend(set),
    };
    loop {
        let c = *p.get(j)?; // unterminated class: a SyntaxError
        if is(c, b']') {
            j += 1;
            break;
        }
        let (a, after) = class_atom(p, j)?;
        j = after;
        let dash = p.get(j).is_some_and(|&u| is(u, b'-'));
        if dash && p.get(j + 1).is_some_and(|&u| !is(u, b']')) {
            let (b, after) = class_atom(p, j + 1)?;
            j = after;
            match (a, b) {
                (ClassAtom::Unit(x), ClassAtom::Unit(y)) => {
                    if x > y {
                        return None; // range out of order
                    }
                    ranges.push((x, y));
                }
                // Annex B: a class escape as an endpoint makes the `-` literal.
                (a, b) => {
                    add(a, &mut ranges);
                    ranges.push((0x2D, 0x2D));
                    add(b, &mut ranges);
                }
            }
        } else {
            add(a, &mut ranges);
        }
    }

    let mut bits = vec![0u64; 1024];
    for (a, b) in ranges {
        for u in a..=b {
            let c = canonicalize(u);
            bits[usize::from(c >> 6)] |= 1 << (c & 63);
        }
    }
    let has = |u: u32| bits[(u >> 6) as usize] & (1 << (u & 63)) != 0;
    // Runs of members, split at the surrogate blocks. `regress` mis-parses a class where a
    // high-surrogate escape is followed by anything but `]` (it tries to pair it), so the
    // one run of high surrogates goes last; no run then starts with a low surrogate right
    // after a high one.
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut u = 0u32;
    while u <= 0xFFFF {
        if has(u) {
            let lo = u;
            while u < 0xFFFF && has(u + 1) && ![0xD800, 0xDC00, 0xE000].contains(&(u + 1)) {
                u += 1;
            }
            runs.push((lo, u));
        }
        u += 1;
    }
    runs.sort_by_key(|&(lo, _)| (0xD800..0xDC00).contains(&lo));
    out.push_str(if negate { "[^" } else { "[" });
    for (lo, hi) in runs {
        push_class_unit(out, lo as u16);
        if hi != lo {
            out.push('-');
            push_class_unit(out, hi as u16);
        }
    }
    out.push(']');
    Some(j)
}

/// Rewrites a JS (non-`u`, `i`) pattern into a case-sensitive `regress` pattern over
/// canonical code units; `None` when JS would throw a `SyntaxError` that `regress` might
/// not (the caller also compiles the result, which catches the rest).
pub(crate) fn translate(pattern: &str) -> Option<String> {
    let p: Vec<u16> = pattern.encode_utf16().collect();
    let named_groups = has_named_groups(&p);
    let mut out = String::with_capacity(pattern.len() * 2);
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        let ascii = u8::try_from(c).unwrap_or(0);
        match ascii {
            b'\\' => {
                let next = *p.get(i + 1)?; // `\` at the end
                let n = u8::try_from(next).unwrap_or(0);
                match n {
                    b'b' | b'B' => {
                        out.push('\\');
                        out.push(char::from(n));
                        i += 2;
                        if quantifier_at(&p, i) {
                            return None; // nothing to repeat
                        }
                    }
                    b'd' | b'D' | b's' | b'S' | b'w' | b'W' => {
                        // Canonical text keeps word / digit / space-ness (canonicalising
                        // never maps non-ASCII onto ASCII), so these work unchanged.
                        out.push('\\');
                        out.push(char::from(n));
                        i += 2;
                    }
                    // A back-reference (or legacy escape): always valid without `u`, and
                    // refused by the problem check, so only a placeholder is needed.
                    b'1'..=b'9' => {
                        out.push_str("\\u0000");
                        i += 2;
                    }
                    b'k' if named_groups => {
                        out.push_str("\\k");
                        i += 2;
                    }
                    _ => {
                        let (unit, len) = char_escape(&p, i + 1, false);
                        push_literal(&mut out, unit);
                        i += 1 + len;
                    }
                }
            }
            b'[' => i = class(&p, i + 1, &mut out)?,
            b'^' | b'$' => {
                out.push(char::from(ascii));
                i += 1;
                if quantifier_at(&p, i) {
                    return None;
                }
            }
            b'(' => {
                out.push('(');
                i += 1;
                if p.get(i).is_some_and(|&u| is(u, b'?')) {
                    // Group syntax (`?:`, `?=`, `?<=`, `?<name>`) is copied, not canonicalised.
                    let end = if !p.get(i + 1).is_some_and(|&u| is(u, b'<')) {
                        i + 2
                    } else if p.get(i + 2).is_some_and(|&u| is(u, b'=') || is(u, b'!')) {
                        i + 3
                    } else {
                        p[i..].iter().position(|&u| is(u, b'>')).map_or(p.len(), |k| i + k + 1)
                    };
                    let end = end.min(p.len());
                    out.push_str(&String::from_utf16_lossy(&p[i..end]));
                    i = end;
                }
            }
            b')' | b'|' | b'.' | b'*' | b'+' | b'?' => {
                out.push(char::from(ascii));
                i += 1;
            }
            b'{' => {
                if let Some((len, n, max)) = braced_quantifier(&p, i) {
                    if max.is_some_and(|m| m < n) {
                        return None; // numbers out of order
                    }
                    let n = n.min(MAX_COUNT);
                    match max {
                        None => {
                            let _ = write!(out, "{{{n},}}");
                        }
                        Some(m) => {
                            let _ = write!(out, "{{{n},{}}}", m.min(MAX_COUNT));
                        }
                    }
                    i += len;
                } else {
                    push_literal(&mut out, c); // Annex B: a literal `{`
                    i += 1;
                }
            }
            _ => {
                push_literal(&mut out, c);
                i += 1;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalises_like_js() {
        assert_eq!(canonicalize(u16::from(b'a')), u16::from(b'A'));
        assert_eq!(canonicalize(0x017F), 0x017F); // ſ → S would map non-ASCII onto ASCII
        assert_eq!(canonicalize(0x0131), 0x0131); // ı likewise
        assert_eq!(canonicalize(0x00DF), 0x00DF); // ß uppercases to two units
        assert_eq!(canonicalize(0x03C9), 0x03A9); // ω → Ω
        assert_eq!(canonicalize(0x2126), 0x2126); // the ohm sign stays itself
    }

    #[test]
    fn rewrites_classes_as_canonical_sets() {
        assert_eq!(translate("[a-c]").as_deref(), Some(r"[A-C]"));
        assert_eq!(translate("[^a]").as_deref(), Some(r"[^A]"));
        assert_eq!(translate("[z-a]"), None);
        assert_eq!(translate("[\\x-a]"), None); // `\x` without digits is `x`
        assert_eq!(translate("[a"), None);
        assert_eq!(translate("\\b+"), None);
        assert_eq!(translate("^*"), None);
        assert_eq!(translate("x{2,1}"), None);
        assert_eq!(translate("a{99999}").as_deref(), Some("A{257,257}"));
    }

    #[test]
    fn puts_high_surrogates_last_in_a_class() {
        // `regress` mis-parses anything after a high-surrogate escape inside a class.
        let p = translate("[a-😀]").unwrap();
        assert!(p.ends_with(r"\uD800-\uD83D]"), "{p}");
        let re = regress::Regex::with_flags(&p, "").unwrap();
        assert!(re.find_from_ucs2(&[0x0100], 0).next().is_some());
        assert!(re.find_from_ucs2(&[0x0038], 0).next().is_none());
    }
}
