//! Browser profiles for the app (P2.8, F35): every profile seen, whether it's connected
//! now and its tabs, and renaming.

use decay_bridge::ErrorCode;
use decay_bridge::protocol::{PopupProfile, ProfileRename};
use decay_store::StoreError;
use serde::Serialize;

use crate::engine::{ConnId, Engine, EngineEvent, Output, Res};

/// A browser profile as Settings and the Library show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInfo {
    pub app_id: String,
    pub app_name: String,
    pub profile_id: String,
    /// What the user called it.
    pub name: Option<String>,
    /// 1-based by first sighting among its browser's profiles ("Profile 2").
    pub ordinal: u32,
    /// Its browser has other profiles: unnamed, it shows as "Profile N".
    pub has_siblings: bool,
    /// Its extension is connected now.
    pub connected: bool,
    /// Tabs tracked in it now (0 when not connected).
    pub tabs: u32,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// A rename's result: the stored name and what the engine has to send.
pub type Renamed = Result<(Option<String>, Vec<Output>), RenameRefused>;

/// Profiles unseen this long, with nothing archived, are forgotten.
pub const FORGET_IDLE_PROFILE_MS: i64 = 30 * 86_400_000;

/// Why Settings' "Forget" didn't happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgetRefused {
    #[error("that browser profile isn't known")]
    UnknownProfile,
    #[error("that browser profile is connected now")]
    Connected,
}

/// Why a rename didn't happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenameRefused {
    #[error("that browser profile isn't known")]
    UnknownProfile,
}

impl Engine {
    /// Every profile seen, by browser and first sighting.
    pub fn profiles(&self, now: i64) -> Res<Vec<ProfileInfo>> {
        let mut names = std::collections::HashMap::new();
        let mut out = Vec::new();
        for p in self.store.profiles()? {
            if !names.contains_key(&p.app_id) {
                let n = self.store.app(&p.app_id)?.map_or_else(|| p.app_id.clone(), |a| a.name);
                names.insert(p.app_id.clone(), n);
            }
            let app_name = names[&p.app_id].clone();
            let live = self
                .conns
                .values()
                .filter_map(|c| Self::live_adapter(c, now))
                .find(|a| a.app_id == p.app_id && a.profile == p.profile);
            let tabs = live.map_or(0, |a| {
                u32::try_from(a.tabs.values().filter(|t| t.item.is_some()).count())
                    .unwrap_or(u32::MAX)
            });
            out.push(ProfileInfo {
                app_id: p.app_id,
                app_name,
                profile_id: p.profile,
                name: p.name,
                ordinal: p.ordinal,
                has_siblings: p.has_siblings,
                connected: live.is_some(),
                tabs,
                first_seen_at: p.first_seen_at,
                last_seen_at: p.last_seen_at,
            });
        }
        Ok(out)
    }

    /// Names a profile (`None` or blank: unnamed). Returns the stored name.
    pub fn rename_profile(
        &mut self,
        app_id: &str,
        profile_id: &str,
        name: Option<&str>,
    ) -> Res<Renamed> {
        match self.store.rename_profile(app_id, profile_id, name) {
            Ok(stored) => {
                self.out.push(Output::Event(EngineEvent::ProfilesChanged));
                Ok(Ok((stored, std::mem::take(&mut self.out))))
            }
            Err(StoreError::NotFound) => Ok(Err(RenameRefused::UnknownProfile)),
            Err(e) => Err(e),
        }
    }

    /// The browser profile of connection `conn` as its popup shows it.
    pub(crate) fn popup_profile(&self, conn: ConnId) -> Res<Option<PopupProfile>> {
        let Some(a) = self.conns.get(&conn).and_then(|c| c.adapter.as_ref()) else {
            return Ok(None);
        };
        let found = self
            .store
            .profiles()?
            .into_iter()
            .find(|p| p.app_id == a.app_id && p.profile == a.profile);
        Ok(found.map(|p| PopupProfile {
            name: p.name,
            ordinal: p.ordinal,
            has_siblings: p.has_siblings,
        }))
    }

    /// `profile.rename` from a popup: names its own browser profile.
    pub(crate) fn on_profile_rename(&mut self, conn: ConnId, req: &str, r: &ProfileRename) -> Res {
        let Some((app_id, profile)) = self
            .conns
            .get(&conn)
            .and_then(|c| c.adapter.as_ref())
            .map(|a| (a.app_id.clone(), a.profile.clone()))
        else {
            self.reply_err(conn, req, ErrorCode::NotFound, "not connected");
            return Ok(());
        };
        if r.name.as_ref().is_some_and(|n| n.len() > decay_store::MAX_PROFILE_NAME * 16) {
            self.reply_err(conn, req, ErrorCode::BadMessage, "name too long");
            return Ok(());
        }
        match self.store.rename_profile(&app_id, &profile, r.name.as_deref()) {
            Ok(name) => {
                self.out.push(Output::Event(EngineEvent::ProfilesChanged));
                self.reply_ok(conn, req, &serde_json::json!({ "name": name }));
            }
            Err(StoreError::NotFound) => {
                self.reply_err(conn, req, ErrorCode::NotFound, "unknown profile");
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Hourly: connected profiles are seen now (so "last seen" stays right after a crash),
    /// and profiles idle for 30 days with nothing archived are forgotten.
    pub(crate) fn maintain_profiles(&mut self, now: i64) -> Res {
        let live: Vec<(String, String)> = self
            .conns
            .values()
            .filter_map(|c| Self::live_adapter(c, now))
            .map(|a| (a.app_id.clone(), a.profile.clone()))
            .collect();
        for (app, profile) in &live {
            self.store.touch_profile(app, profile, now)?;
        }
        let forgotten = self.store.forget_idle_profiles(now - FORGET_IDLE_PROFILE_MS)?;
        if forgotten > 0 {
            tracing::info!(forgotten, "idle browser profiles forgotten");
            self.out.push(Output::Event(EngineEvent::ProfilesChanged));
        }
        Ok(())
    }

    /// Forgets a profile that isn't connected (Settings → Browsers "Forget").
    pub fn forget_profile(
        &mut self,
        app_id: &str,
        profile_id: &str,
    ) -> Res<Result<Vec<Output>, ForgetRefused>> {
        let connected = self
            .conns
            .values()
            .filter_map(|c| c.adapter.as_ref())
            .any(|a| a.app_id == app_id && a.profile == profile_id);
        if connected {
            return Ok(Err(ForgetRefused::Connected));
        }
        if !self.store.forget_profile(app_id, profile_id)? {
            return Ok(Err(ForgetRefused::UnknownProfile));
        }
        self.out.push(Output::Event(EngineEvent::ProfilesChanged));
        Ok(Ok(std::mem::take(&mut self.out)))
    }
}
