//! Core logic for KXbps: xbps interaction, models and data types.
//!
//! This crate is free of any Qt dependency so it can be tested and reused
//! without a QML/CXX-Qt bridge.

pub mod engine;
pub mod format;
pub mod mirrors;
pub mod plan;
pub mod probe;
pub mod request;
pub mod store;
pub mod types;
pub mod xbps;

pub use engine::{
    Plan, PlanTotals, apply_installed, effective_updates, held_updates_in, plan_totals,
    repos_from_installed, update_signature_of,
};
pub use format::{ByteUnit, action_key, byte_decimals, format_bytes, reason_key};
pub use mirrors::{DEFAULT_MIRROR, MIRRORS, MirrorInfo, Region};
pub use plan::{
    Breaker, DryRunOutcome, EXIT_NOT_INSTALLED, EXIT_UNRESOLVED, is_already_installed,
    is_not_installed, items_from, missing_package, parse_breakers, read_dry_run,
};
pub use probe::{PROBE_TIMEOUT, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};
pub use request::{DryRun, PlanRequest, install_names, not_installed, remove_names};
pub use store::{Store, StoreError, data_dir, installed_map};
pub use types::{
    Blocked, DialogState, Inst, InstalledMap, Job, ListFilter, PlanAction, PlanItem, PlanReason,
    Prefs, ProbeState, QueryMode, QueueAction, QueueItem, Repos, ViewId,
};
pub use xbps::{
    InstallState, InstalledPackage, RepoEntry, SearchHit, SizeEntry, UpdateEntry, XbpsAction,
    XbpsError, parse_depends, parse_human_size, parse_installed, parse_pkgver_list,
    parse_properties, parse_search, parse_size_list, parse_updates, split_pkgver,
};
