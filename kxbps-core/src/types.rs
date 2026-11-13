//! Domain types shared by every KXbps layer.
//!
//! These mirror `qxbps/src/lib/xbps/types.ts` one-to-one, including the JSON
//! spelling of every string union, so the same data can be exchanged with the
//! QML side and compared against the web app's fixtures. Nothing here touches
//! Qt, the filesystem or the network.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One entry of the installed-package database (`xbps-query -l`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Inst {
    pub version: String,
    /// Installed on purpose rather than pulled in as a dependency.
    pub explicit: bool,
    /// Pinned by the user, so updates must not touch it.
    pub held: bool,
    /// Install timestamp, Unix epoch milliseconds (the web app stores `Date.UTC(...)`).
    pub at: i64,
}

/// Installed packages keyed by package name.
pub type InstalledMap = HashMap<String, Inst>;

/// Which optional repositories are enabled (`/etc/xbps.d` config files).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Repos {
    pub nonfree: bool,
    pub multilib: bool,
    pub debug: bool,
}

/// Operation the user queued for a package.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum QueueAction {
    Install,
    Remove,
    Update,
}

/// A queued operation, before it is resolved into a [`PlanItem`].
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub name: String,
    pub action: QueueAction,
}

/// Operation a planned transaction will perform on one package.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanAction {
    Install,
    Remove,
    Update,
    Reinstall,
}

/// Why a package ended up in the transaction plan.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanReason {
    /// The user asked for it.
    Requested,
    /// Pulled in to satisfy a dependency.
    Dependency,
    /// Removed because nothing depends on it any more.
    Reverse,
    /// Removed because it is orphaned.
    Orphan,
}

/// One resolved step of a transaction plan.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub name: String,
    pub action: PlanAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub reason: PlanReason,
    pub download: u64,
    pub size: u64,
}

/// A package that cannot be removed because other packages need it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub name: String,
    pub needed_by: Vec<String>,
}

/// Top-level view of the main window.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ViewId {
    Packages,
    Query,
    Mirrors,
}

/// Package-list filter chips.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ListFilter {
    All,
    Installed,
    Updates,
    Explicit,
    Orphans,
    Held,
    Queued,
}

/// The `xbps-query` modes offered by the query view.
///
/// Variant names spell the `xbps-query` flag exactly as the web app's
/// `QueryMode` union does, so `serde` keeps the wire format identical.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueryMode {
    /// `-Rs`: search the repository pool.
    #[serde(rename = "Rs")]
    Rs,
    /// `-s`: search installed packages.
    #[serde(rename = "s")]
    S,
    /// `-l`: list installed packages.
    #[serde(rename = "l")]
    L,
    /// `-R`: show a repository package.
    #[serde(rename = "R")]
    R,
    /// `xbps-query <pkg>`: show an installed package.
    #[serde(rename = "show")]
    Show,
    /// `-x`: dependencies.
    #[serde(rename = "x")]
    X,
    /// `-X`: reverse dependencies.
    #[serde(rename = "X")]
    RevX,
    /// `-f`: files owned by a package.
    #[serde(rename = "f")]
    F,
    /// `-o`: owner of a file.
    #[serde(rename = "o")]
    O,
    /// `-m`: manually installed packages.
    #[serde(rename = "m")]
    M,
    /// `-O`: orphaned packages.
    #[serde(rename = "O")]
    Orphans,
    /// `-H`: held packages.
    #[serde(rename = "H")]
    H,
    /// `-L`: configured repositories.
    #[serde(rename = "L")]
    RepoL,
    /// `-p <prop>`: one package property.
    #[serde(rename = "p")]
    P,
}

impl QueryMode {
    /// Flag as typed on the command line; `show` has none because it is the
    /// bare `xbps-query <pkg>` form.
    pub fn flag(self) -> &'static str {
        match self {
            QueryMode::Rs => "-Rs",
            QueryMode::S => "-s",
            QueryMode::L => "-l",
            QueryMode::R => "-R",
            QueryMode::Show => "",
            QueryMode::X => "-x",
            QueryMode::RevX => "-X",
            QueryMode::F => "-f",
            QueryMode::O => "-o",
            QueryMode::M => "-m",
            QueryMode::Orphans => "-O",
            QueryMode::H => "-H",
            QueryMode::RepoL => "-L",
            QueryMode::P => "-p",
        }
    }

    /// Whether the mode takes a pattern argument at all. `-l`, `-m`, `-O`, `-H`
    /// and `-L` are argument-less listings, everything else needs one.
    pub fn needs_pattern(self) -> bool {
        !matches!(
            self,
            QueryMode::L | QueryMode::M | QueryMode::Orphans | QueryMode::H | QueryMode::RepoL
        )
    }

    /// Modes whose argument names a single package rather than matching text.
    pub fn names_single_package(self) -> bool {
        matches!(
            self,
            QueryMode::R
                | QueryMode::Show
                | QueryMode::X
                | QueryMode::RevX
                | QueryMode::F
                | QueryMode::P
        )
    }
}

/// Currently open modal dialog.
///
/// The web app discriminates the union by `kind`; the tag keeps that shape on
/// the wire so the QML side can switch on `dialog.kind`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DialogState {
    /// Confirm a transaction plan before running it.
    Plan {
        title: String,
        items: Vec<PlanItem>,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Removal refused because other packages depend on the selection.
    Blocked {
        blocked: Vec<Blocked>,
        names: Vec<String>,
    },
    About,
    Reset,
    Job,
}

/// User preferences persisted across sessions.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub notify_on_start: bool,
    pub desktop: bool,
    /// Dismissed notification key, empty when nothing was dismissed.
    #[serde(default)]
    pub dismissed: String,
    /// Whether the app starts with the session, as an autostart entry in the
    /// user's `~/.config/autostart`. Defaults to `true`, so a `prefs.json`
    /// written before this key existed starts with the notifier on. The Qt
    /// layer writes or removes the entry from this value; nothing here touches
    /// the filesystem.
    #[serde(default = "default_start_with_session")]
    pub start_with_session: bool,
    /// Version of this block. `4` means the update announcement defaults to on,
    /// an older file was migrated when it was loaded, and a delivered marker
    /// written by an earlier notifier was cleared once.
    #[serde(default)]
    pub schema: u32,
}

/// The default of [`Prefs::start_with_session`]: on.
fn default_start_with_session() -> bool {
    true
}

/// Result of the last mirror latency probe.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ProbeState {
    Idle,
    Running,
    Ok { ms: u64 },
    Fail { error: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefs_block_without_the_new_key_starts_with_autostart_on() {
        // `prefs.json` from the version before autostart has no
        // `startWithSession`; it must read as "on", and the two older optional
        // fields must not be able to fail the whole block either.
        let prefs: Prefs =
            serde_json::from_str(r#"{"notifyOnStart":false,"desktop":true,"dismissed":"7"}"#)
                .expect("the old block still reads");
        assert!(prefs.start_with_session);
        assert!(!prefs.notify_on_start);
        assert!(prefs.desktop);

        let stored: Prefs = serde_json::from_str(r#"{"notifyOnStart":true,"desktop":false}"#)
            .expect("the minimal block reads");
        assert!(stored.start_with_session);
        assert_eq!(stored.dismissed, "");
    }

    #[test]
    fn an_autostart_turned_off_survives_the_round_trip() {
        let prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
            start_with_session: false,
            schema: 4,
        };
        let json = serde_json::to_string(&prefs).expect("serialise");
        assert!(json.contains("\"startWithSession\":false"));
        assert_eq!(serde_json::from_str::<Prefs>(&json).expect("decode"), prefs);
    }
}
