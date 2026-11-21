// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Core logic for KXbps: xbps interaction, models and data types.
//!
//! This crate is free of any Qt dependency so it can be tested and reused
//! without a QML/CXX-Qt bridge.

pub mod engine;
pub mod format;
pub mod mirror_apply;
pub mod mirror_config;
pub mod mirrors;
pub mod plan;
pub mod probe;
pub mod repository_edit;
pub mod request;
pub mod search;
pub mod store;
pub mod transaction;
pub mod types;
pub mod update_check;
pub mod xbps;

pub use engine::{
    Plan, PlanTotals, apply_installed, drop_applied_updates, effective_updates, held_updates_in,
    pending_updates, plan_totals, repos_from_installed, update_signature_of,
};
pub use format::{ByteUnit, action_key, byte_decimals, format_bytes, reason_key};
pub use mirror_apply::{
    ApplyRequest, ApplyResult, Cli, Exit, HELPER_PATH, XBPS_QUERY, apply, check_url, last_line,
    verify_argv, verify_repos,
};
pub use mirror_config::{
    BACKUP_SUFFIX, Backup, ConfigError, ParseReason, Previous, SYSTEM_CONFIG_PATH, SystemConfig,
    check_base_url, check_repository_value, parse_repository_conf, read_config, render_config,
    revert, snapshot_previous, write_config_atomic,
};
pub use mirrors::{DEFAULT_MIRROR, MIRRORS, MirrorInfo, Region};
pub use plan::{
    Breaker, DryRunOutcome, EXIT_NOT_INSTALLED, EXIT_UNRESOLVED, already_installed_names,
    is_already_installed, is_not_installed, is_only_already_installed, items_from, lock_waited_on,
    missing_package, parse_breakers, read_dry_run,
};
pub use probe::{PROBE_TIMEOUT, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};
pub use request::{
    DryRun, PlanRequest, install_names, installs_as_updates, not_installed, remove_names,
};
pub use search::rank_search_hits;
pub use store::{Store, StoreError, data_dir, installed_map};
pub use transaction::{
    MIRROR_ACTION_ID, MIRROR_APPLY_PATH, Outcome, PKEXEC, Privileged, Transaction, TransactionPlan,
    XPBS_INSTALL_PATH, XPBS_REMOVE_PATH, mirror_argv, pkgvers,
};
pub use types::{
    Blocked, DEFAULT_UPDATE_CHECK_MINUTES, DialogState, Inst, InstalledMap, ListFilter, PlanAction,
    PlanItem, PlanReason, Prefs, ProbeState, QueueAction, QueueItem, Repos, UPDATE_INTERVALS,
    ViewId, normalize_interval,
};
pub use update_check::{
    AUTO_GAP_MS, CheckOutcome, CheckStatus, UpdatesOrigin, check_status, claims_up_to_date, is_due,
    is_offline, local_replaces, offline_key,
};
pub use xbps::{
    InstallState, InstalledPackage, RepoEntry, SearchHit, SizeEntry, UpdateEntry, XbpsAction,
    XbpsError, parse_depends, parse_human_size, parse_installed, parse_pkgver_list,
    parse_properties, parse_search, parse_size_list, parse_updates, split_pkgver,
};
