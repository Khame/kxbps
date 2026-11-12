//! The pieces of a transaction plan that are not xbps' answer.
//!
//! xbps resolves the transaction: `xbps-install -n` and `xbps-remove -n` print
//! the steps, their versions and their sizes, and [`crate::plan`] turns that
//! into [`PlanItem`]s. What xbps does not print is here — the totals of a plan,
//! the installed map a finished plan leaves behind and the held-update note.
//!
//! Everything is pure: the installed map and the plan go in, values come out.
//! Nothing here touches Qt, the filesystem or the network, and no user-facing
//! prose is produced — [`crate::format::action_key`] and
//! [`crate::format::reason_key`] turn plan entries into stable translation keys.

use crate::types::{Blocked, Inst, InstalledMap, PlanAction, PlanItem, PlanReason, Repos};
use crate::xbps::UpdateEntry;

/// Ordered steps of a plan, plus the packages that blocked a removal.
///
/// A blocked removal carries no steps: the caller shows the blocking list and
/// either gives up or replans with the whole dependent list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub blocked: Vec<Blocked>,
}

impl Plan {
    /// Whether the removal was refused because other packages need it.
    pub fn is_blocked(&self) -> bool {
        !self.blocked.is_empty()
    }
}

/// Download and disk-size totals of a plan, in bytes.
///
/// `added` covers installs, updates and reinstalls; `removed` covers removals.
/// The two are kept apart because the net disk delta (`added - removed`) and its
/// sign belong to the Qt/QML layer, not to the core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanTotals {
    pub download: u64,
    pub added: u64,
    pub removed: u64,
}

/// Sums the byte counts of a plan, as they appear on each [`PlanItem`].
pub fn plan_totals(items: &[PlanItem]) -> PlanTotals {
    let mut totals = PlanTotals::default();
    for item in items {
        totals.download += item.download;
        if item.action == PlanAction::Remove {
            totals.removed += item.size;
        } else {
            totals.added += item.size;
        }
    }
    totals
}

/// The updates the application shows: what xbps reported, minus held packages.
///
/// xbps' own dry run (`xbps-install -un`) already respects a hold in its
/// configuration, but the hold the user toggles in this window is only in the
/// store until a transaction writes it, so it is filtered here. Entries keep
/// the order xbps printed and only `xbps-query -H` can make one disappear.
pub fn effective_updates(pending: &[UpdateEntry], installed: &InstalledMap) -> Vec<UpdateEntry> {
    pending
        .iter()
        .filter(|entry| !installed.get(&entry.name).is_some_and(|inst| inst.held))
        .cloned()
        .collect()
}

/// Held packages that the reported updates want to touch, sorted by name.
///
/// The core returns names and the QML layer renders the sentence.
pub fn held_updates_in(pending: &[UpdateEntry], installed: &InstalledMap) -> Vec<String> {
    let mut names: Vec<String> = pending
        .iter()
        .filter(|entry| installed.get(&entry.name).is_some_and(|inst| inst.held))
        .map(|entry| entry.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Applies a finished plan to the installed database.
///
/// Removed packages disappear, a package requested by the user becomes
/// explicit, an explicit flag and a hold survive, and `at` is `now` only for
/// fresh installs.
pub fn apply_installed(installed: &InstalledMap, plan: &[PlanItem], now: i64) -> InstalledMap {
    let mut next = installed.clone();
    for item in plan {
        if item.action == PlanAction::Remove {
            next.remove(&item.name);
            continue;
        }
        let prev = installed.get(&item.name);
        let version = item
            .to
            .clone()
            .or_else(|| prev.map(|inst| inst.version.clone()))
            .unwrap_or_default();
        let explicit = if item.reason == PlanReason::Requested {
            true
        } else {
            prev.is_some_and(|inst| inst.explicit)
        };
        let held = prev.is_some_and(|inst| inst.held);
        let at = if item.action == PlanAction::Install {
            now
        } else {
            prev.map_or(now, |inst| inst.at)
        };
        next.insert(
            item.name.clone(),
            Inst {
                version,
                explicit,
                held,
                at,
            },
        );
    }
    next
}

/// Enabled repositories inferred from the installed `void-repo-*` packages.
pub fn repos_from_installed(installed: &InstalledMap) -> Repos {
    Repos {
        nonfree: installed.contains_key("void-repo-nonfree"),
        multilib: installed.contains_key("void-repo-multilib"),
        debug: installed.contains_key("void-repo-debug"),
    }
}

/// Stable fingerprint of a pending update list.
///
/// The joined `name@to` text is hashed with FNV-1a so the result is a `u64`
/// identical across runs and platforms (a `DefaultHasher` is randomly seeded and
/// would change on every start).
pub fn update_signature_of(pending: &[UpdateEntry]) -> u64 {
    signature_of(pending)
}

fn signature_of(updates: &[UpdateEntry]) -> u64 {
    let mut text = String::new();
    for (index, update) in updates.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&update.name);
        text.push('@');
        text.push_str(&update.version);
    }
    fnv1a64(text.as_bytes())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
#[cfg(test)]
mod tests {
    use super::*;
    fn inst(version: &str, explicit: bool, held: bool) -> Inst {
        Inst {
            version: version.to_string(),
            explicit,
            held,
            at: 1_790_000_000_000,
        }
    }

    fn installed(entries: &[(&str, Inst)]) -> InstalledMap {
        entries
            .iter()
            .map(|(name, inst)| ((*name).to_string(), inst.clone()))
            .collect()
    }

    fn update(name: &str, version: &str) -> UpdateEntry {
        UpdateEntry {
            name: name.to_string(),
            version: version.to_string(),
            pkgver: format!("{name}-{version}"),
            action: crate::xbps::XbpsAction::Update,
            arch: "x86_64".to_string(),
            repository: "https://repo.example/current".to_string(),
            installed_size: 1,
            download_size: 1,
        }
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn apply_installed_tracks_explicit_dependency_and_holds() {
        let before = installed(&[
            ("keep", inst("1.0_1", true, true)),
            ("dep", inst("1.0_1", false, false)),
            ("old", inst("1.0_1", false, false)),
        ]);
        let items = vec![
            PlanItem {
                name: "dep".into(),
                action: PlanAction::Update,
                from: Some("1.0_1".into()),
                to: Some("2.0_1".into()),
                reason: PlanReason::Dependency,
                download: 0,
                size: 0,
            },
            PlanItem {
                name: "fresh".into(),
                action: PlanAction::Install,
                from: None,
                to: Some("3.0_1".into()),
                reason: PlanReason::Requested,
                download: 0,
                size: 0,
            },
            PlanItem {
                name: "old".into(),
                action: PlanAction::Remove,
                from: Some("1.0_1".into()),
                to: None,
                reason: PlanReason::Requested,
                download: 0,
                size: 0,
            },
        ];
        let after = apply_installed(&before, &items, 42);

        assert!(!after.contains_key("old"), "a removal leaves the map");
        assert_eq!(after["dep"].version, "2.0_1");
        assert!(!after["dep"].explicit, "a dependency stays implicit");
        assert_eq!(
            after["dep"].at, before["dep"].at,
            "an update keeps its date"
        );
        assert_eq!(after["fresh"].version, "3.0_1");
        assert!(after["fresh"].explicit, "a requested install is explicit");
        assert_eq!(after["fresh"].at, 42, "a fresh install is stamped now");
        assert!(after["keep"].held, "a hold survives a plan that skips it");
    }

    #[test]
    fn apply_installed_keeps_hold_and_explicit_on_update() {
        let before = installed(&[("vim", inst("1.0_1", true, true))]);
        let items = vec![PlanItem {
            name: "vim".into(),
            action: PlanAction::Update,
            from: Some("1.0_1".into()),
            to: Some("2.0_1".into()),
            reason: PlanReason::Dependency,
            download: 0,
            size: 0,
        }];
        let after = apply_installed(&before, &items, 42);
        assert_eq!(after["vim"].version, "2.0_1");
        assert!(after["vim"].explicit, "explicit survives");
        assert!(after["vim"].held, "the hold survives");
        assert_eq!(after["vim"].at, before["vim"].at);
    }

    #[test]
    fn repos_from_installed_detects_repo_packages() {
        let installed = installed(&[("void-repo-nonfree", inst("1.0_1", true, false))]);
        let repos = repos_from_installed(&installed);
        assert!(repos.nonfree);
        assert!(!repos.multilib);
        assert!(!repos.debug);
    }

    #[test]
    fn update_signature_tracks_the_update_list() {
        let pending = vec![update("vim", "9.2_1"), update("git", "2.0_1")];
        let changed = vec![update("vim", "9.3_1"), update("git", "2.0_1")];
        assert_ne!(update_signature_of(&pending), update_signature_of(&changed));
        assert_eq!(
            update_signature_of(&pending),
            update_signature_of(&[update("vim", "9.2_1"), update("git", "2.0_1")])
        );
        assert_eq!(update_signature_of(&[]), update_signature_of(&[]));
    }

    #[test]
    fn plan_totals_sum_download_and_disk_use() {
        let items = vec![
            PlanItem {
                name: "vim".into(),
                action: PlanAction::Install,
                from: None,
                to: Some("9.2_1".into()),
                reason: PlanReason::Requested,
                download: 1_468_006,
                size: 4_194_304,
            },
            PlanItem {
                name: "tzdata".into(),
                action: PlanAction::Remove,
                from: Some("2024a_1".into()),
                to: None,
                reason: PlanReason::Requested,
                download: 0,
                size: 2_097_152,
            },
        ];
        let totals = plan_totals(&items);
        assert_eq!(totals.download, 1_468_006);
        assert_eq!(totals.added, 4_194_304);
        assert_eq!(totals.removed, 2_097_152);
    }

    #[test]
    fn effective_updates_drops_the_packages_the_user_held() {
        let pending = vec![update("vim", "9.2_1"), update("git", "2.0_1")];
        let installed = installed(&[
            ("vim", inst("9.1_1", true, true)),
            ("git", inst("1.9_1", true, false)),
        ]);
        let effective = effective_updates(&pending, &installed);
        assert_eq!(
            effective
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["git"]
        );
    }

    #[test]
    fn held_updates_in_names_exactly_the_held_pending_updates() {
        let pending = vec![
            update("vim", "9.2_1"),
            update("git", "2.0_1"),
            update("vim", "9.2_1"),
        ];
        let installed = installed(&[
            ("vim", inst("9.1_1", true, true)),
            ("git", inst("1.9_1", true, false)),
        ]);
        assert_eq!(held_updates_in(&pending, &installed), names(&["vim"]));
    }
}
