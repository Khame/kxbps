//! The pieces of a transaction plan that are not xbps' answer.
//!
//! xbps resolves the transaction: `xbps-install -n` and `xbps-remove -n` print
//! the steps, their versions and their sizes, and [`crate::plan`] turns that
//! into [`PlanItem`]s. What xbps does not print is here — the totals of a plan,
//! the installed map a finished plan leaves behind, the held-update note and
//! the dependency check a blocked removal needs before its dry run.
//!
//! Everything is pure: the catalog, the installed map and the plan go in,
//! values come out. Nothing here touches Qt, the filesystem or the network, and
//! no user-facing prose is produced — [`crate::format::action_key`] and
//! [`crate::format::reason_key`] turn plan entries into stable translation keys.

use crate::catalog::Catalog;
use crate::types::{
    Blocked, Inst, InstalledMap, PlanAction, PlanItem, PlanReason, Repos, UpdateInfo,
};
use crate::xbps::UpdateEntry;
use std::collections::BTreeSet;

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

/// The installed-map version of [`held_updates_in`], kept for the store paths
/// that still read the seed catalog.
pub fn held_updates(catalog: &Catalog, installed: &InstalledMap, repos: &Repos) -> Vec<String> {
    let mut names: Vec<String> = catalog
        .updates(installed, repos)
        .into_iter()
        .filter(|update| installed.get(&update.name).is_some_and(|inst| inst.held))
        .map(|update| update.name)
        .collect();
    names.sort();
    names
}

/// Packages that stop `names` from being removed, without planning anything.
///
/// This is the pre-check of a removal: it answers from the catalog and the
/// installed set before any process runs, so the Blocked dialog can open
/// without waiting. It is not the authority — `xbps-remove -n` decides a
/// blocked transaction with its exit code and stderr, and when the two
/// disagree the dry run wins. The dry run also sees the whole dependency
/// graph, while this only knows what the catalog lists.
pub fn check_blocked(
    catalog: &Catalog,
    installed: &InstalledMap,
    names: &[String],
) -> Vec<Blocked> {
    let removing: BTreeSet<String> = names
        .iter()
        .filter(|name| installed.contains_key(*name))
        .cloned()
        .collect();
    let mut blocked = Vec::new();
    for name in ordered_names(names, &removing) {
        let needed_by: Vec<String> = catalog
            .revdeps(&name)
            .into_iter()
            .filter(|rev| installed.contains_key(&rev.name) && !removing.contains(&rev.name))
            .map(|rev| rev.name.clone())
            .collect();
        if !needed_by.is_empty() {
            blocked.push(Blocked { name, needed_by });
        }
    }
    blocked
}

/// Packages in the removal set, requested names first, then the rest by name.
fn ordered_names(requested: &[String], set: &BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(set.len());
    for name in requested {
        if set.contains(name) && !out.contains(name) {
            out.push(name.clone());
        }
    }
    for name in set {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
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

/// The same fingerprint over the seed catalog's update list.
pub fn update_signature(catalog: &Catalog, installed: &InstalledMap, repos: &Repos) -> u64 {
    signature_of(&catalog.updates(installed, repos))
}

trait UpdateTarget {
    fn signature_parts(&self) -> (&str, &str);
}

impl UpdateTarget for UpdateInfo {
    fn signature_parts(&self) -> (&str, &str) {
        (&self.name, &self.to)
    }
}

impl UpdateTarget for UpdateEntry {
    fn signature_parts(&self) -> (&str, &str) {
        (&self.name, &self.version)
    }
}

fn signature_of<T: UpdateTarget>(updates: &[T]) -> u64 {
    let mut text = String::new();
    for (index, update) in updates.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        let (name, to) = update.signature_parts();
        text.push_str(name);
        text.push('@');
        text.push_str(to);
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
    use crate::catalog::Catalog;
    use crate::types::{Category, Pkg, RepoId};

    fn pkg(name: &str, depends: &[&str]) -> Pkg {
        Pkg {
            name: name.to_string(),
            version: "1.0_1".to_string(),
            repo: RepoId::Current,
            category: Category::Base,
            size: 1024,
            download: 512,
            short: String::new(),
            homepage: String::new(),
            license: String::new(),
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
            files: Vec::new(),
        }
    }

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
    fn check_blocked_lists_the_installed_dependents() {
        // `base-system` needs `acpid`; `bash` needs `glibc`, which is not in the
        // way when only `acpid` is removed.
        let catalog = Catalog(vec![
            pkg("acpid", &[]),
            pkg("base-system", &["acpid"]),
            pkg("glibc", &[]),
            pkg("bash", &["glibc"]),
        ]);
        let installed = installed(&[
            ("acpid", inst("2.0.34_4", true, false)),
            ("base-system", inst("0.114_2", true, false)),
            ("glibc", inst("2.41_1", true, false)),
            ("bash", inst("5.2_1", true, false)),
        ]);
        let blocked = check_blocked(&catalog, &installed, &names(&["acpid"]));
        assert_eq!(
            blocked,
            vec![Blocked {
                name: "acpid".to_string(),
                needed_by: vec!["base-system".to_string()],
            }]
        );
    }

    #[test]
    fn check_blocked_ignores_packages_that_are_not_installed() {
        let catalog = Catalog(vec![pkg("acpid", &[]), pkg("base-system", &["acpid"])]);
        let installed = installed(&[("base-system", inst("0.114_2", true, false))]);
        assert_eq!(
            check_blocked(&catalog, &installed, &names(&["acpid", "ghost"])),
            Vec::new()
        );
    }

    #[test]
    fn check_blocked_keeps_the_requested_order() {
        let catalog = Catalog(vec![
            pkg("libfoo", &[]),
            pkg("app", &["libfoo"]),
            pkg("libbar", &[]),
            pkg("tool", &["libbar"]),
        ]);
        let installed = installed(&[
            ("libfoo", inst("1.0_1", false, false)),
            ("app", inst("1.0_1", true, false)),
            ("libbar", inst("1.0_1", false, false)),
            ("tool", inst("1.0_1", true, false)),
        ]);
        let blocked = check_blocked(&catalog, &installed, &names(&["libbar", "libfoo"]));
        assert_eq!(
            blocked
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["libbar", "libfoo"]
        );
    }

    #[test]
    fn check_blocked_is_empty_when_the_dependents_go_too() {
        let catalog = Catalog(vec![pkg("acpid", &[]), pkg("base-system", &["acpid"])]);
        let installed = installed(&[
            ("acpid", inst("2.0.34_4", true, false)),
            ("base-system", inst("0.114_2", true, false)),
        ]);
        assert_eq!(
            check_blocked(&catalog, &installed, &names(&["acpid", "base-system"])),
            Vec::new()
        );
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
