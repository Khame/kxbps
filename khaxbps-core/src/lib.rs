//! Core logic for Khaxbps: xbps interaction, models and data types.
//!
//! This crate is free of any Qt dependency so it can be tested and reused
//! without a QML/CXX-Qt bridge.

pub mod catalog;
pub mod types;

pub use catalog::{Catalog, CatalogError, SECURITY_UPDATES, is_security_update, repo_enabled};
pub use types::{
    Blocked, Category, DialogState, Inst, InstalledMap, Job, ListFilter, Pkg, PlanAction, PlanItem,
    PlanReason, Prefs, ProbeState, QueryMode, QueueAction, QueueItem, RepoId, Repos, UpdateInfo,
    ViewId,
};
