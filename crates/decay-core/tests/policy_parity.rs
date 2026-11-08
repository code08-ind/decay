//! Cross-language parity: `decay_core::evaluate` must decide exactly like the extension's
//! `lib/policy/policy.ts` (Software Design §4.1). The fixture is generated from the real
//! TypeScript policy by `apps/extension/scripts/policy-parity.ts`.

use decay_core::matching::{MatchTarget, title_matches, title_regex_problem};
use decay_core::policy::{is_blank_page, is_excluded};
use decay_core::{
    Decision, FormInput, ItemKind, Mode, PolicyContext, PolicyTarget, Reason, Rule, SaveState,
    Signals, Thresholds, evaluate,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Fixture {
    pool: Vec<Rule>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    ctx: Ctx,
    rule_idx: Vec<usize>,
    item: Item,
    signals: Sig,
    decision: Value,
    excluded: bool,
    blank: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ctx {
    now: i64,
    mode: Mode,
    hold_until: Option<i64>,
    thresholds: Thresholds,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    app_id: String,
    url: Option<String>,
    title: String,
    pinned: bool,
    active_age_s: u64,
    grace_until: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sig {
    active: bool,
    audible: bool,
    /// `true`, `false`, `"unknown"` or absent.
    form_input: Option<Value>,
}

/// The Rust decision in the extension's JSON shape.
fn to_ts_json(d: &Decision) -> Value {
    let reason = |r: &Reason| serde_json::to_value(r).unwrap();
    match d {
        Decision::Keep { active_hours, threshold } => {
            json!({"action":"keep","activeHours":active_hours,"threshold":threshold})
        }
        Decision::Protect { .. } | Decision::Archive { .. } => {
            let action = if d.is_protected() { "protect" } else { "archive" };
            json!({"action": action, "reason": reason(&d.reason().unwrap())})
        }
        Decision::Stale { active_hours, threshold, rule_id } => {
            let mut v = json!({"action":"stale","activeHours":active_hours,"threshold":threshold});
            if let Some(id) = rule_id {
                v["ruleId"] = json!(id);
            }
            v
        }
        Decision::Hide { .. } => json!({"action":"hide"}),
    }
}

/// JSON equality with numbers compared as f64 (Rust prints `60.0`, JS prints `60`).
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-9
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(v, w))
        }
        _ => a == b,
    }
}

#[test]
fn rust_policy_matches_the_extension_on_every_fixture_case() {
    let raw = include_str!("fixtures/policy_parity.json");
    let fixture: Fixture = serde_json::from_str(raw).expect("fixture parses");
    assert!(fixture.cases.len() >= 5000, "fixture too small");

    let mut failures = Vec::new();
    for (i, c) in fixture.cases.iter().enumerate() {
        let rules: Vec<Rule> = c.rule_idx.iter().map(|&k| fixture.pool[k].clone()).collect();
        let ctx = PolicyContext {
            now: c.ctx.now,
            mode: c.ctx.mode,
            hold_until: c.ctx.hold_until,
            thresholds: c.ctx.thresholds,
            rules: &rules,
            pro: false,
        };
        let item = PolicyTarget {
            kind: ItemKind::Tab,
            app_id: &c.item.app_id,
            url: c.item.url.as_deref(),
            title: &c.item.title,
            pinned: c.item.pinned,
            active_age_s: c.item.active_age_s,
            grace_until: c.item.grace_until,
        };
        let form_input = match &c.signals.form_input {
            None => FormInput::NotApplicable,
            Some(Value::Bool(true)) => FormInput::Dirty,
            Some(Value::Bool(false)) => FormInput::Clean,
            Some(Value::String(s)) if s == "unknown" => FormInput::Unknown,
            Some(other) => panic!("case {i}: unexpected formInput {other}"),
        };
        let signals = Signals {
            in_use: c.signals.active,
            audible: c.signals.audible,
            form_input,
            save_state: SaveState::NotApplicable,
        };
        let got = to_ts_json(&evaluate(&item, &signals, &ctx));
        if !same(&got, &c.decision) {
            failures.push(format!("case {i}: rust {got} vs ts {}", c.decision));
        }
        let target = MatchTarget { app_id: item.app_id, url: item.url, title: item.title };
        if is_excluded(&target, &rules) != c.excluded || is_blank_page(item.url) != c.blank {
            failures.push(format!("case {i}: isExcluded / isBlankPage differ"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        fixture.cases.len(),
        failures.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[derive(Deserialize)]
struct TitleCase {
    pattern: String,
    problem: Option<String>,
    title: String,
    matches: bool,
}

#[test]
fn title_patterns_validate_and_match_like_the_extension() {
    let raw = include_str!("fixtures/title_parity.json");
    let cases: Vec<TitleCase> = serde_json::from_str(raw).expect("fixture parses");
    assert!(cases.len() >= 2000, "fixture too small");
    let mut failures = Vec::new();
    for c in &cases {
        let problem = title_regex_problem(&c.pattern);
        if problem != c.problem.as_deref() {
            failures
                .push(format!("problem {:?}: rust {problem:?} vs ts {:?}", c.pattern, c.problem));
            continue;
        }
        let got = title_matches(&c.pattern, &c.title);
        if got != c.matches {
            failures.push(format!(
                "match {:?} on {:?}: rust {got} vs ts {}",
                c.pattern, c.title, c.matches
            ));
        }
    }
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} of {} title cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// One cased BMP code unit (or a random class range) under three patterns, each against a
/// few titles: `m[pattern][title]` is 1 when the extension matched.
#[derive(Deserialize)]
struct FoldCase {
    p: Vec<String>,
    t: Vec<String>,
    m: Vec<Vec<u8>>,
}

#[test]
fn case_insensitive_matching_folds_exactly_like_js() {
    let raw = include_str!("fixtures/fold_parity.json");
    let cases: Vec<FoldCase> = serde_json::from_str(raw).expect("fixture parses");
    assert!(cases.len() >= 2500, "fixture too small");
    let mut failures = Vec::new();
    for c in &cases {
        for (pattern, row) in c.p.iter().zip(&c.m) {
            for (title, &want) in c.t.iter().zip(row) {
                if title_matches(pattern, title) != (want == 1) {
                    failures.push(format!("{pattern:?} on {title:?}: ts {want}"));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} fold cases differ:\n{}",
        failures.len(),
        failures.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}
