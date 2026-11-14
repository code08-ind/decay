//! `decay://` links (API Design §6, F26). They arrive from the OS (another app, a
//! notification, the purchase email) or, on Windows, as an argument to a second launch.
//!
//! A link only ever opens UI: restoring, undoing and activating a license are confirmed
//! there ("Deep links never perform a destructive action without UI confirmation").
//! Anything malformed is ignored.

use url::Url;

/// Settings tabs a link may open (Settings sidebar, UI Design Guidelines).
pub const SETTINGS_TABS: [&str; 8] = [
    "general",
    "aging",
    "protection",
    "privacy",
    "shortcuts",
    "browsers",
    "notifications",
    "license",
];

const MAX_QUERY: usize = 500;
const MAX_LICENSE_KEY: usize = 512;

/// A parsed `decay://` link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// `decay://search?q=…`: the panel, prefilled.
    Search { query: String },
    /// `decay://restore/{itemId}`.
    Restore { item_id: String },
    /// `decay://sweep/{sweepId}`.
    Sweep { sweep_id: String },
    /// `decay://undo/{sweepId}`.
    Undo { sweep_id: String },
    /// `decay://settings/{tab}` (no tab: the first one).
    Settings { tab: Option<String> },
    /// `decay://license/activate?key=…`.
    LicenseActivate { key: String },
    /// `decay://shadow-report`.
    ShadowReport,
}

/// A window of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Panel,
    Library,
    Settings,
    /// First run (F21).
    Onboarding,
}

impl Window {
    /// The Tauri window label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Library => "library",
            Self::Settings => "settings",
            Self::Onboarding => "onboarding",
        }
    }
}

/// Item and sweep ids are UUIDs; accept the id alphabet only.
fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn query_param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned())
}

/// Parses a link; `None` for anything that isn't a well-formed `decay://` link.
pub fn parse(link: &str) -> Option<DeepLink> {
    let url = Url::parse(link.trim()).ok()?;
    if url.scheme() != "decay" {
        return None;
    }
    // `decay://search?q=x`: "search" is the host; the rest is the path.
    let host = url.host_str()?.to_ascii_lowercase();
    let parts: Vec<&str> =
        url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let one_id = || match parts.as_slice() {
        [id] if valid_id(id) => Some((*id).to_owned()),
        _ => None,
    };
    match host.as_str() {
        "search" if parts.is_empty() => {
            let query: String =
                query_param(&url, "q").unwrap_or_default().chars().take(MAX_QUERY).collect();
            Some(DeepLink::Search { query })
        }
        "restore" => one_id().map(|item_id| DeepLink::Restore { item_id }),
        "sweep" => one_id().map(|sweep_id| DeepLink::Sweep { sweep_id }),
        "undo" => one_id().map(|sweep_id| DeepLink::Undo { sweep_id }),
        "settings" => match parts.as_slice() {
            [] => Some(DeepLink::Settings { tab: None }),
            [tab] => {
                let tab = tab.to_ascii_lowercase();
                SETTINGS_TABS
                    .contains(&tab.as_str())
                    .then_some(DeepLink::Settings { tab: Some(tab) })
            }
            _ => None,
        },
        "license" if parts.as_slice() == ["activate"] => {
            let key = query_param(&url, "key")?;
            let key = key.trim();
            (!key.is_empty() && key.len() <= MAX_LICENSE_KEY && !key.chars().any(char::is_control))
                .then(|| DeepLink::LicenseActivate { key: key.to_owned() })
        }
        "shadow-report" if parts.is_empty() => Some(DeepLink::ShadowReport),
        _ => None,
    }
}

impl DeepLink {
    /// Where the link opens: a window and the route inside it (the UI confirms anything
    /// that changes data).
    pub fn target(&self) -> (Window, String) {
        match self {
            Self::Search { .. } => (Window::Panel, String::new()),
            Self::Restore { item_id } => {
                (Window::Library, format!("/item/{item_id}?confirm=restore"))
            }
            Self::Sweep { sweep_id } => (Window::Library, format!("/sweeps/{sweep_id}")),
            Self::Undo { sweep_id } => {
                (Window::Library, format!("/sweeps/{sweep_id}?confirm=undo"))
            }
            Self::Settings { tab } => {
                (Window::Settings, format!("/{}", tab.as_deref().unwrap_or(SETTINGS_TABS[0])))
            }
            // The key travels in the event payload, not in the route (routes may be logged).
            Self::LicenseActivate { .. } => (Window::Settings, "/license?activate".to_owned()),
            Self::ShadowReport => (Window::Library, "/shadow-report".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b7f3a52-6c1e-4f7d-9a0e-2d4c8b1e9f00";

    #[test]
    fn parses_every_documented_link() {
        assert_eq!(
            parse("decay://search?q=pricing%20page"),
            Some(DeepLink::Search { query: "pricing page".into() })
        );
        assert_eq!(parse("decay://search"), Some(DeepLink::Search { query: String::new() }));
        assert_eq!(
            parse(&format!("decay://restore/{ID}")),
            Some(DeepLink::Restore { item_id: ID.into() })
        );
        assert_eq!(
            parse(&format!("decay://sweep/{ID}")),
            Some(DeepLink::Sweep { sweep_id: ID.into() })
        );
        assert_eq!(
            parse(&format!("decay://undo/{ID}/")),
            Some(DeepLink::Undo { sweep_id: ID.into() })
        );
        assert_eq!(
            parse("decay://settings/Shortcuts"),
            Some(DeepLink::Settings { tab: Some("shortcuts".into()) })
        );
        assert_eq!(parse("decay://settings"), Some(DeepLink::Settings { tab: None }));
        assert_eq!(
            parse("decay://license/activate?key=ABCD-1234"),
            Some(DeepLink::LicenseActivate { key: "ABCD-1234".into() })
        );
        assert_eq!(parse("DECAY://shadow-report"), Some(DeepLink::ShadowReport));
    }

    #[test]
    fn rejects_anything_else() {
        for bad in [
            "https://search?q=x",
            "decay://",
            "decay://nope",
            "decay://restore",
            "decay://restore/a/b",
            "decay://restore/..%2F..",
            "decay://restore/<script>",
            "decay://settings/../../etc",
            "decay://settings/unknown",
            "decay://license/activate",
            "decay://license/activate?key=%20",
            "decay://license/activate?key=a%0Ab",
            "decay://search/extra?q=x",
            "not a url",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        let long = format!("decay://restore/{}", "a".repeat(65));
        assert_eq!(parse(&long), None);
    }

    #[test]
    fn a_long_query_is_cut() {
        let link = format!("decay://search?q={}", "é".repeat(800));
        let Some(DeepLink::Search { query }) = parse(&link) else { panic!() };
        assert_eq!(query.chars().count(), MAX_QUERY);
    }

    #[test]
    fn links_open_ui_only() {
        let target = |l: &str| parse(l).unwrap().target();
        assert_eq!(target("decay://search?q=x"), (Window::Panel, String::new()));
        assert_eq!(
            target(&format!("decay://undo/{ID}")),
            (Window::Library, format!("/sweeps/{ID}?confirm=undo"))
        );
        assert_eq!(
            target(&format!("decay://restore/{ID}")),
            (Window::Library, format!("/item/{ID}?confirm=restore"))
        );
        assert_eq!(target(&format!("decay://sweep/{ID}")).1, format!("/sweeps/{ID}"));
        assert_eq!(target("decay://settings").1, "/general");
        assert_eq!(target("decay://shadow-report"), (Window::Library, "/shadow-report".into()));
        let (window, route) = target("decay://license/activate?key=SECRET");
        assert_eq!(window, Window::Settings);
        assert!(!route.contains("SECRET"));
        assert_eq!(Window::Panel.label(), "panel");
        assert_eq!(Window::Onboarding.label(), "onboarding");
    }
}
