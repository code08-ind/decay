//! Pro gates (P4.2, PRD §7.4): custom-timing rules beyond three, and the retention when
//! Pro ends (never deleting what was archived under Pro on the day it ends).

mod support;

use decay_core::{ItemKind, ItemState, RuleAction, RuleScope, Thresholds};
use decay_engine::{RuleInput, RuleRefused};
use decay_store::Item;
#[allow(clippy::wildcard_imports)]
use support::*;

const DAY: i64 = 86_400_000;

fn custom(domain: &str) -> RuleInput {
    RuleInput {
        scope: RuleScope::Domain,
        pattern: domain.into(),
        action: RuleAction::CustomThreshold,
        threshold_hours: Some(Thresholds { stale: 1.0, hidden: 2.0, archive: 3.0 }),
        enabled: None,
    }
}

#[test]
fn a_fourth_custom_rule_needs_pro() {
    let mut h = Harness::new();
    for d in ["a.com", "b.com", "c.com"] {
        h.now += 1;
        h.engine.create_rule(&custom(d), h.now).unwrap();
    }
    h.now += 1;
    assert!(matches!(h.engine.create_rule(&custom("d.com"), h.now), Err(RuleRefused::ProRequired)));
    // Other kinds of rules are unlimited; a turned-off custom rule doesn't count.
    let protect =
        RuleInput { action: RuleAction::Protect, threshold_hours: None, ..custom("e.com") };
    h.engine.create_rule(&protect, h.now).unwrap();
    h.now += 1;
    let off = RuleInput { enabled: Some(false), ..custom("f.com") };
    let (off, ..) = h.engine.create_rule(&off, h.now).unwrap();
    // ... but turning it on does.
    let on = RuleInput { enabled: Some(true), ..custom("f.com") };
    assert!(matches!(h.engine.update_rule(&off.id, &on, h.now), Err(RuleRefused::ProRequired)));

    // With Pro: unlimited. Pro ending keeps them all (only three apply).
    h.engine.set_license(true, None, h.now).unwrap();
    h.engine.update_rule(&off.id, &on, h.now).unwrap();
    h.now += 1;
    h.engine.create_rule(&custom("g.com"), h.now).unwrap();
    h.engine.set_license(false, Some(h.now), h.now).unwrap();
    let custom_rules = h
        .engine
        .rules()
        .unwrap()
        .into_iter()
        .filter(|r| r.action == RuleAction::CustomThreshold && r.enabled)
        .count();
    assert_eq!(custom_rules, 5, "kept, not deleted");
    // Without Pro, the three that apply can still be edited; the others can't be changed
    // while on (turning them off works).
    let id_of = |h: &Harness, d: &str| {
        h.engine.rules().unwrap().into_iter().find(|r| r.pattern == d).unwrap().id
    };
    let slower = Some(Thresholds { stale: 2.0, hidden: 3.0, archive: 4.0 });
    let edit_a = RuleInput { threshold_hours: slower, ..custom("a.com") };
    let a = id_of(&h, "a.com");
    h.engine.update_rule(&a, &edit_a, h.now).unwrap();
    let edit_g = RuleInput { threshold_hours: slower, ..custom("g.com") };
    let g = id_of(&h, "g.com");
    assert!(matches!(h.engine.update_rule(&g, &edit_g, h.now), Err(RuleRefused::ProRequired)));
    let disable = RuleInput { enabled: Some(false), ..custom("g.com") };
    h.engine.update_rule(&g, &disable, h.now).unwrap();
}

fn archived_at(h: &Harness, id: &str, at: i64) {
    let mut item = Item::new_live(id.into(), ItemKind::Tab, "com.google.Chrome", at);
    item.url = Some(format!("https://{id}.example/"));
    item.title = id.into();
    item.state = ItemState::Archived;
    item.archived_at = Some(at);
    item.state_changed_at = at;
    h.store().insert_item(&item).unwrap();
}

#[test]
fn when_pro_ends_older_archives_keep_their_retention() {
    let mut h = Harness::new();
    h.store().set_setting("retention_days", &365, h.now).unwrap();
    h.engine.set_license(true, None, h.now).unwrap();
    let pro_ended = h.now - 100 * DAY;
    // Archived under Pro (before it ended), and since.
    archived_at(&h, "under-pro", pro_ended - 10 * DAY);
    archived_at(&h, "since-old", pro_ended + DAY);
    archived_at(&h, "since-new", h.now - 10 * DAY);
    archived_at(&h, "ancient", h.now - 400 * DAY);
    h.engine.set_license(false, Some(pro_ended), h.now).unwrap();
    h.now += 2 * 3_600_000;
    h.poll();
    let left = |id: &str| h.store().item(id).unwrap().is_some();
    assert!(left("under-pro"), "archived under Pro: keeps 365 days");
    assert!(!left("since-old"), "archived since Pro ended, 99 days ago: 90-day retention");
    assert!(left("since-new"));
    assert!(!left("ancient"), "past the set retention");
}

#[test]
fn with_pro_the_set_retention_applies_to_everything() {
    let mut h = Harness::new();
    h.store().set_setting("retention_days", &365, h.now).unwrap();
    archived_at(&h, "old", h.now - 200 * DAY);
    h.engine.set_license(true, Some(h.now - 300 * DAY), h.now).unwrap();
    h.now += 2 * 3_600_000;
    h.poll();
    assert!(h.store().item("old").unwrap().is_some());
}
