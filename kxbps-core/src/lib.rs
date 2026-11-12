//! Core logic for Khaxbps: xbps interaction, models and data types.
//!
//! This crate is free of any Qt dependency so it can be tested and reused
//! without a QML/CXX-Qt bridge.

pub mod catalog;
pub mod engine;
pub mod format;
pub mod mirrors;
pub mod probe;
pub mod store;
pub mod types;
pub mod xbps;

pub use catalog::{Catalog, CatalogError, SECURITY_UPDATES, is_security_update, repo_enabled};
pub use engine::{
    EngineError, Plan, PlanTotals, apply_installed, check_blocked, held_updates, plan_install,
    plan_queue, plan_remove, plan_system, plan_totals, repos_from_installed, update_signature,
};
pub use format::{ByteUnit, action_key, byte_decimals, format_bytes, reason_key};
pub use mirrors::{DEFAULT_MIRROR, MIRRORS, MirrorInfo, Region};
pub use probe::{PROBE_TIMEOUT, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};
pub use store::{SEED_AT, Store, StoreError, data_dir, fresh_install};
pub use types::{
    Blocked, Category, DialogState, Inst, InstalledMap, Job, ListFilter, Pkg, PlanAction, PlanItem,
    PlanReason, Prefs, ProbeState, QueryMode, QueueAction, QueueItem, RepoId, Repos, UpdateInfo,
    ViewId,
};
pub use xbps::{
    InstallState, InstalledPackage, RepoEntry, SearchHit, UpdateEntry, XbpsAction, XbpsError,
    parse_installed, parse_property, parse_repos, parse_search, parse_updates, split_pkgver,
};
