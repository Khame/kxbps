//! Core logic for KXbps: xbps interaction, models and data types.
//!
//! This crate is free of any Qt dependency so it can be tested and reused
//! without a QML/CXX-Qt bridge.

pub mod catalog;
pub mod engine;
pub mod format;
pub mod mirrors;
pub mod plan;
pub mod probe;
pub mod request;
pub mod store;
pub mod types;
pub mod xbps;

pub use catalog::{Catalog, CatalogError, SECURITY_UPDATES, is_security_update, repo_enabled};
pub use engine::{
    Plan, PlanTotals, apply_installed, check_blocked, effective_updates, held_updates,
    held_updates_in, plan_totals, repos_from_installed, update_signature, update_signature_of,
};
pub use format::{ByteUnit, action_key, byte_decimals, format_bytes, reason_key};
pub use mirrors::{DEFAULT_MIRROR, MIRRORS, MirrorInfo, Region};
pub use plan::{
    Breaker, DryRunOutcome, EXIT_NOT_INSTALLED, EXIT_UNRESOLVED, is_already_installed,
    is_not_installed, items_from, missing_package, parse_breakers, read_dry_run,
};
pub use probe::{PROBE_TIMEOUT, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};
pub use request::{DryRun, PlanRequest, install_names, not_installed, remove_names};
pub use store::{SEED_AT, Store, StoreError, data_dir, fresh_install, installed_map};
pub use types::{
    Blocked, Category, DialogState, Inst, InstalledMap, Job, ListFilter, Pkg, PlanAction, PlanItem,
    PlanReason, Prefs, ProbeState, QueryMode, QueueAction, QueueItem, RepoId, Repos, UpdateInfo,
    ViewId,
};
pub use xbps::{
    InstallState, InstalledPackage, RepoEntry, SearchHit, UpdateEntry, XbpsAction, XbpsError,
    parse_depends, parse_installed, parse_pkgver_list, parse_properties, parse_search,
    parse_updates, split_pkgver,
};
