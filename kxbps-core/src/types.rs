//! Domain types shared by every Khaxbps layer.
//!
//! These mirror `qxbps/src/lib/xbps/types.ts` one-to-one, including the JSON
//! spelling of every string union, so the same data can be exchanged with the
//! QML side and compared against the web app's fixtures. Nothing here touches
//! Qt, the filesystem or the network.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A package repository known to xbps.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum RepoId {
    Current,
    Nonfree,
    Multilib,
    Debug,
}

/// Coarse package section shown in the package list.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Base,
    Devel,
    Desktop,
    Net,
    Media,
    Graphics,
    Office,
    Games,
    Fonts,
    Admin,
}

/// One package as advertised by a repository index.
///
/// `size` is the installed size in bytes and `download` the compressed
/// download size in bytes; both come from the web app's `kb()`/`mb()` helpers,
/// which already return byte counts.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub repo: RepoId,
    pub category: Category,
    pub size: u64,
    pub download: u64,
    pub short: String,
    pub homepage: String,
    pub license: String,
    pub depends: Vec<String>,
    pub files: Vec<String>,
}

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

/// Progress of a running transaction.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// Percent complete, 0-100 (the web app rounds before storing it).
    pub progress: u8,
    pub label: String,
    pub done: bool,
    pub cancelled: bool,
    pub lines: Vec<String>,
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
    pub dismissed: String,
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

/// A pending update: installed version versus the repository version.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub name: String,
    pub from: String,
    pub to: String,
    /// Package is in the curated security set (see
    /// [`crate::catalog::SECURITY_UPDATES`]).
    pub security: bool,
    pub download: u64,
}
