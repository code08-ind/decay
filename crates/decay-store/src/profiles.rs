//! Browser profiles (P2.8, F35): each extension instance's random profile id, the name the
//! user gave it, and when it was first seen. Items reach their profile through their
//! source (`sources.profile`).
//!
//! How a profile is shown ([`ProfileLabel`]): its name; else, when its browser has several
//! profiles, "Profile N" by order of first sighting; else nothing (just the browser).

use std::collections::HashMap;

use rusqlite::params;

use crate::{Result, Store, StoreError};

/// The longest profile name (characters).
pub const MAX_PROFILE_NAME: usize = 40;

/// A browser profile Decay has seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub app_id: String,
    /// The extension's random profile id.
    pub profile: String,
    /// What the user called it.
    pub name: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    /// 1-based, by first sighting, among its browser's profiles.
    pub ordinal: u32,
    /// Its browser has other profiles too.
    pub has_siblings: bool,
}

impl Profile {
    pub fn label(&self) -> ProfileLabel {
        match &self.name {
            Some(name) => ProfileLabel::Named(name.clone()),
            None if self.has_siblings => ProfileLabel::Numbered(self.ordinal),
            None => ProfileLabel::Only,
        }
    }
}

/// How a profile is shown next to its browser's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileLabel {
    /// "Chrome · Work".
    Named(String),
    /// "Chrome · Profile 2" (unnamed, and the browser has several).
    Numbered(u32),
    /// Just "Chrome" (its browser's only profile).
    Only,
}

/// A name as stored: trimmed, inner whitespace runs (and control characters) as one
/// space, at most [`MAX_PROFILE_NAME`] characters. `None`: empty (unnamed).
pub fn clean_profile_name(name: &str) -> Option<String> {
    // Invisible and direction-changing characters (zero-width, bidi overrides, BOM)
    // could hide a name or flip the line it's shown in: dropped.
    let visible: String = name.chars().filter(|c| !is_format(*c)).collect();
    let words: Vec<&str> = visible
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect();
    let joined = words.join(" ");
    let cut: String = joined.chars().take(MAX_PROFILE_NAME).collect();
    let cut = cut.trim_end().to_owned();
    (!cut.is_empty()).then_some(cut)
}

/// Unicode format characters a name mustn't carry (the common ones of category Cf).
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

impl Store {
    /// Records that a profile is connected now (first sighting, or seen again).
    pub fn touch_profile(&self, app_id: &str, profile: &str, now: i64) -> Result<()> {
        if profile.is_empty() {
            return Ok(());
        }
        self.conn
            .prepare_cached(
                "INSERT INTO profiles (app_id, profile, first_seen_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT (app_id, profile) DO UPDATE SET
                   last_seen_at = max(profiles.last_seen_at, excluded.last_seen_at)",
            )?
            .execute(params![app_id, profile, now])?;
        Ok(())
    }

    /// Every profile, by browser and then first sighting.
    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut rows = self
            .conn
            .prepare_cached(
                "SELECT app_id, profile, name, first_seen_at, last_seen_at,
                   row_number() OVER (PARTITION BY app_id ORDER BY first_seen_at, profile),
                   count(*) OVER (PARTITION BY app_id)
                 FROM profiles ORDER BY app_id, first_seen_at, profile",
            )?
            .query_map([], |r| {
                Ok(Profile {
                    app_id: r.get(0)?,
                    profile: r.get(1)?,
                    name: r.get(2)?,
                    first_seen_at: r.get(3)?,
                    last_seen_at: r.get(4)?,
                    ordinal: r.get(5)?,
                    has_siblings: r.get::<_, i64>(6)? > 1,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.shrink_to_fit();
        Ok(rows)
    }

    /// Every profile by `(app_id, profile)`.
    pub fn profile_map(&self) -> Result<HashMap<(String, String), Profile>> {
        Ok(self
            .profiles()?
            .into_iter()
            .map(|p| ((p.app_id.clone(), p.profile.clone()), p))
            .collect())
    }

    /// Names a profile (`None` or blank: unnamed again). Returns the name as stored.
    /// Fails with `NotFound` for a profile never seen.
    pub fn rename_profile(
        &self,
        app_id: &str,
        profile: &str,
        name: Option<&str>,
    ) -> Result<Option<String>> {
        let name = name.and_then(clean_profile_name);
        let n = self
            .conn
            .prepare_cached("UPDATE profiles SET name = ?3 WHERE app_id = ?1 AND profile = ?2")?
            .execute(params![app_id, profile, name])?;
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    const CHROME: &str = "com.google.Chrome";
    const ARC: &str = "company.thebrowser.Browser";

    fn apps(store: &Store) {
        for id in [CHROME, ARC] {
            store
                .upsert_app(&crate::App {
                    id: id.into(),
                    name: id.into(),
                    platform: crate::Platform::MacOs,
                    is_browser: true,
                    close_allowed: false,
                    restore_kind: crate::RestoreKind::Url,
                    icon_png: None,
                })
                .unwrap();
        }
    }

    #[test]
    fn profiles_are_numbered_by_first_sighting_per_browser() {
        let (store, _dir) = store();
        apps(&store);
        store.touch_profile(CHROME, "b", NOW + 10).unwrap();
        store.touch_profile(CHROME, "a", NOW + 20).unwrap();
        store.touch_profile(ARC, "c", NOW).unwrap();
        store.touch_profile(CHROME, "b", NOW + 99).unwrap(); // seen again: order kept
        store.touch_profile(CHROME, "", NOW).unwrap(); // ignored
        let all = store.profiles().unwrap();
        let got: Vec<_> = all.iter().map(|p| (p.profile.as_str(), p.label())).collect();
        assert_eq!(
            got,
            [
                ("b", ProfileLabel::Numbered(1)),
                ("a", ProfileLabel::Numbered(2)),
                ("c", ProfileLabel::Only)
            ]
        );
        let b = &all[0];
        assert_eq!((b.first_seen_at, b.last_seen_at), (NOW + 10, NOW + 99));
    }

    #[test]
    fn names_are_cleaned_and_can_be_cleared() {
        let (store, _dir) = store();
        apps(&store);
        store.touch_profile(CHROME, "a", NOW).unwrap();
        let name = store.rename_profile(CHROME, "a", Some("  Work \n\t stuff ")).unwrap();
        assert_eq!(name.as_deref(), Some("Work stuff"));
        let p = &store.profiles().unwrap()[0];
        assert_eq!(p.label(), ProfileLabel::Named("Work stuff".into()));
        assert_eq!(store.rename_profile(CHROME, "a", Some("   ")).unwrap(), None);
        assert_eq!(store.profiles().unwrap()[0].label(), ProfileLabel::Only);
        assert!(matches!(
            store.rename_profile(CHROME, "nope", Some("x")),
            Err(StoreError::NotFound)
        ));
        let long = "é".repeat(60);
        let stored = store.rename_profile(CHROME, "a", Some(&long)).unwrap().unwrap();
        assert_eq!(stored.chars().count(), MAX_PROFILE_NAME);
    }

    #[test]
    fn clean_names() {
        assert_eq!(clean_profile_name(""), None);
        assert_eq!(clean_profile_name(" a\u{0}b "), Some("a b".into()));
        assert_eq!(clean_profile_name("Wo\u{200B}rk\u{202E}"), Some("Work".into()));
        assert_eq!(clean_profile_name("\u{202E}\u{FEFF}"), None);
        assert_eq!(clean_profile_name(&format!("{} x", "a".repeat(39))), Some("a".repeat(39)));
    }
}
