//! Transaction planning: dependency resolution, install order and removal blocking.
//!
//! This is the planning half of the web app's `src/lib/xbps/engine.ts`
//! (`planInstall`, `planUpdates`, `planSystem`, `planRemove`, `queuePlan`,
//! `applyInstalled`, `reposFromInstalled`, `updateSignature`). Every function is
//! pure: the catalog and the installed map go in, a plan comes out. Nothing here
//! touches Qt, the filesystem or the network, and no user-facing prose is
//! produced — [`crate::format::action_key`] and [`crate::format::reason_key`]
//! turn plan entries into stable translation keys.

use crate::catalog::{Catalog, repo_enabled};
use crate::types::{
    Blocked, Inst, InstalledMap, Pkg, PlanAction, PlanItem, PlanReason, QueueAction, QueueItem,
    RepoId, Repos,
};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Everything that can go wrong while planning a transaction.
///
/// The variants are structured rather than pre-formatted so the Qt/QML layer can
/// translate them; the `Display` text is for logs and tests only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// The package is not in the catalog at all.
    #[error("package `{0}` is not in the catalog")]
    UnknownPackage(String),
    /// The package comes from a repository the user has not enabled.
    #[error("repository {repo:?} is disabled for package `{package}`")]
    RepoDisabled { package: String, repo: RepoId },
    /// A direct dependency comes from a repository the user has not enabled.
    #[error("package `{package}` needs `{dependency}` from disabled repository {repo:?}")]
    DependencyRepoDisabled {
        package: String,
        dependency: String,
        repo: RepoId,
    },
    /// An update was requested for packages that are not installed.
    #[error("not installed: {0:?}")]
    NotInstalled(Vec<String>),
}

/// Ordered steps of a plan, plus the packages that blocked a removal.
///
/// A blocked removal carries no steps: like the web app, the caller shows the
/// blocking list and either gives up or replans with `cascade: true`.
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

/// Builds an install/update plan for a queue.
///
/// Mirrors the web app's `planInstall` for [`QueueAction::Install`] entries and
/// `planUpdates` for [`QueueAction::Update`] entries, in that order (the order
/// `queuePlan` uses). [`QueueAction::Remove`] entries are ignored here — removal
/// is [`plan_remove`]'s job; [`plan_queue`] combines both.
///
/// Every package pulled in to satisfy a dependency is planned before the package
/// that needs it, and missing catalog entries are skipped exactly like the web
/// app's `BY_NAME` lookups.
pub fn plan_install(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    queue: &[QueueItem],
) -> Result<Vec<PlanItem>, EngineError> {
    let mut selection = Selection::default();

    for item in queue
        .iter()
        .filter(|item| item.action == QueueAction::Install)
    {
        let pkg = validate_installable(catalog, installed, repos, &item.name)?;
        selection.push_like(catalog, installed, pkg, PlanReason::Requested, true);
    }

    // `planUpdates` reports every name that is not installed at once, before it
    // plans anything.
    let missing: Vec<String> = queue
        .iter()
        .filter(|item| item.action == QueueAction::Update && !installed.contains_key(&item.name))
        .map(|item| item.name.clone())
        .collect();
    if !missing.is_empty() {
        return Err(EngineError::NotInstalled(missing));
    }

    for item in queue
        .iter()
        .filter(|item| item.action == QueueAction::Update)
    {
        let inst = installed.get(&item.name).expect("checked above");
        // Held packages are skipped before anything else, like `planUpdates`.
        if inst.held {
            continue;
        }
        let pkg = validate_installable(catalog, installed, repos, &item.name)?;
        // An up-to-date package is not an update at all.
        if inst.version == pkg.version {
            continue;
        }
        selection.push_like(catalog, installed, pkg, PlanReason::Requested, true);
    }

    Ok(selection.finish(catalog))
}

/// Plans a system upgrade: every available update, respecting holds and
/// disabled repositories.
///
/// Mirrors the web app's `planSystem`, which is `planInstall` over
/// `updates(installed, repos)` — so [`Catalog::updates`] does the hold, version
/// and repository filtering, and this only turns the result into a queue.
pub fn plan_system(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
) -> Result<Vec<PlanItem>, EngineError> {
    let queue: Vec<QueueItem> = catalog
        .updates(installed, repos)
        .into_iter()
        .map(|update| QueueItem {
            name: update.name,
            action: QueueAction::Update,
        })
        .collect();
    plan_install(catalog, installed, repos, &queue)
}

/// Held packages that have an update waiting, sorted by name.
///
/// This is the list behind the web app's "skipped held packages" note on the
/// system plan (`planSystem`'s `held`); the core returns names, the QML layer
/// renders the sentence.
pub fn held_updates(catalog: &Catalog, installed: &InstalledMap, repos: &Repos) -> Vec<String> {
    let mut names: Vec<String> = catalog
        .packages()
        .iter()
        .filter(|pkg| repo_enabled(pkg.repo, repos))
        .filter(|pkg| {
            installed
                .get(&pkg.name)
                .is_some_and(|inst| inst.held && inst.version != pkg.version)
        })
        .map(|pkg| pkg.name.clone())
        .collect();
    names.sort();
    names
}

/// Plans a removal, optionally cascading into the packages that need the
/// removed ones.
///
/// Mirrors the web app's `planRemove(names, installed, cascade, unused: true)`:
/// every web-app call site passes `unused: true`, so the orphan sweep (removing
/// non-explicit dependencies that only the removed packages used) always runs.
///
/// A non-empty [`Plan::blocked`] means nothing can be removed yet; the caller
/// either shows the blocking list or replans with `cascade: true`.
pub fn plan_remove(
    catalog: &Catalog,
    installed: &InstalledMap,
    names: &[String],
    cascade: bool,
) -> Plan {
    let mut removing: BTreeSet<String> = names
        .iter()
        .filter(|name| installed.contains_key(*name))
        .cloned()
        .collect();

    if cascade {
        loop {
            let mut grew = false;
            let snapshot: Vec<String> = removing.iter().cloned().collect();
            for name in snapshot {
                for rev in catalog.revdeps(&name) {
                    if installed.contains_key(&rev.name) && !removing.contains(&rev.name) {
                        removing.insert(rev.name.clone());
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
    }

    let blocked = blocked_for(catalog, installed, &removing, names);
    if !blocked.is_empty() {
        return Plan {
            items: Vec::new(),
            blocked,
        };
    }

    // Orphan sweep (`unused: true`): pull in non-explicit packages that only the
    // packages being removed depend on.
    loop {
        let mut grew = false;
        let mut candidates: Vec<&String> = installed.keys().collect();
        candidates.sort();
        for name in candidates {
            if installed.get(name).is_some_and(|inst| inst.explicit) || removing.contains(name) {
                continue;
            }
            let needed_by_others = catalog
                .revdeps(name)
                .iter()
                .any(|rev| installed.contains_key(&rev.name) && !removing.contains(&rev.name));
            let used_by_removed = catalog
                .revdeps(name)
                .iter()
                .any(|rev| removing.contains(&rev.name));
            if !needed_by_others && used_by_removed {
                removing.insert(name.clone());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let mut items = Vec::with_capacity(removing.len());
    for name in order_remove_like(catalog, &removing) {
        let inst = installed.get(&name);
        let reason = if names.contains(&name) {
            PlanReason::Requested
        } else if inst.is_some_and(|inst| inst.explicit) {
            PlanReason::Reverse
        } else {
            PlanReason::Orphan
        };
        items.push(PlanItem {
            name: name.clone(),
            action: PlanAction::Remove,
            from: inst.map(|inst| inst.version.clone()),
            to: None,
            reason,
            download: 0,
            size: catalog.by_name(&name).map_or(0, |pkg| pkg.size),
        });
    }

    Plan {
        items,
        blocked: Vec::new(),
    }
}

/// Packages that stop `names` from being removed, without planning anything.
///
/// The standalone form of `planRemove(...).blocked`: only installed packages
/// that something still needs are reported.
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
    blocked_for(catalog, installed, &removing, names)
}

/// Plans a whole queue: installs and updates first, then removals.
///
/// Mirrors the web app's `queuePlan`: the removal is planned with
/// `cascade: false`, a blocked removal discards every step, and duplicate names
/// keep their first occurrence.
pub fn plan_queue(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    queue: &[QueueItem],
) -> Result<Plan, EngineError> {
    let mut items = plan_install(catalog, installed, repos, queue)?;

    let removes: Vec<String> = queue
        .iter()
        .filter(|item| item.action == QueueAction::Remove)
        .map(|item| item.name.clone())
        .collect();
    let removal = plan_remove(catalog, installed, &removes, false);
    if !removal.blocked.is_empty() {
        return Ok(Plan {
            items: Vec::new(),
            blocked: removal.blocked,
        });
    }

    let mut seen: HashSet<String> = items.iter().map(|item| item.name.clone()).collect();
    for item in removal.items {
        if seen.insert(item.name.clone()) {
            items.push(item);
        }
    }

    Ok(Plan {
        items,
        blocked: Vec::new(),
    })
}

/// Applies a finished plan to the installed database.
///
/// Mirrors the web app's `applyInstalled(installed, items, now)`: removed
/// packages disappear, a package requested by the user becomes explicit, an
/// explicit flag and a hold survive, and `at` is `now` only for fresh installs.
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

/// Stable fingerprint of the pending update list.
///
/// Mirrors the web app's `updateSignature(updates(installed, repos))`, which
/// joins `name@to` pairs; the joined text is hashed with FNV-1a so the result is
/// a `u64` that is identical across runs and platforms (a `DefaultHasher` is
/// randomly seeded and would change on every start).
pub fn update_signature(catalog: &Catalog, installed: &InstalledMap, repos: &Repos) -> u64 {
    let updates = catalog.updates(installed, repos);
    let mut text = String::new();
    for (index, update) in updates.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&update.name);
        text.push('@');
        text.push_str(&update.to);
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

/// The packages selected for an install-like plan, in first-visit order.
#[derive(Debug, Default)]
struct Selection {
    seen: HashSet<String>,
    visit_order: Vec<String>,
    items: HashMap<String, PlanItem>,
}

impl Selection {
    /// Adds `pkg` and, depth first, everything it depends on.
    ///
    /// This is the web app's `pushInstallLike`: dependencies are visited first
    /// (with reason [`PlanReason::Dependency`]), a package already visited is
    /// skipped, and an installed package is only planned when it is outdated
    /// (`allow_update`, not held) or was requested explicitly (reinstall).
    ///
    /// Recursion is bounded by the catalog size and the `seen` guard stops a
    /// dependency cycle from looping.
    fn push_like(
        &mut self,
        catalog: &Catalog,
        installed: &InstalledMap,
        pkg: &Pkg,
        reason: PlanReason,
        allow_update: bool,
    ) {
        if !self.seen.insert(pkg.name.clone()) {
            return;
        }
        for dep in &pkg.depends {
            // A dependency missing from the catalog is skipped, like `BY_NAME.get`.
            if let Some(dep_pkg) = catalog.by_name(dep) {
                self.push_like(catalog, installed, dep_pkg, PlanReason::Dependency, true);
            }
        }

        let existing = installed.get(&pkg.name);
        let action = match existing {
            None => Some(PlanAction::Install),
            Some(inst) if inst.version != pkg.version && allow_update && !inst.held => {
                Some(PlanAction::Update)
            }
            Some(inst) if reason == PlanReason::Requested && inst.version == pkg.version => {
                Some(PlanAction::Reinstall)
            }
            Some(_) => None,
        };
        let Some(action) = action else {
            return;
        };

        self.visit_order.push(pkg.name.clone());
        self.items.insert(
            pkg.name.clone(),
            PlanItem {
                name: pkg.name.clone(),
                action,
                from: existing.map(|inst| inst.version.clone()),
                to: Some(pkg.version.clone()),
                reason,
                download: pkg.download,
                size: pkg.size,
            },
        );
    }

    /// Orders the selection so dependencies come first.
    fn finish(self, catalog: &Catalog) -> Vec<PlanItem> {
        let rank: HashMap<String, usize> = self
            .visit_order
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), index))
            .collect();
        let order = order_install_like(catalog, &self.items, &rank);
        let mut items = self.items;
        order
            .into_iter()
            .filter_map(|name| items.remove(&name))
            .collect()
    }
}

/// Resolves a queue name to a catalog package and checks the repository rules
/// `planInstall` applies before planning anything.
fn validate_installable<'a>(
    catalog: &'a Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    name: &str,
) -> Result<&'a Pkg, EngineError> {
    let pkg = catalog
        .by_name(name)
        .ok_or_else(|| EngineError::UnknownPackage(name.to_string()))?;
    if !repo_enabled(pkg.repo, repos) && !installed.contains_key(name) {
        return Err(EngineError::RepoDisabled {
            package: pkg.name.clone(),
            repo: pkg.repo,
        });
    }
    // Only direct dependencies are checked, like the web app.
    for dep in &pkg.depends {
        if let Some(dep_pkg) = catalog.by_name(dep)
            && !repo_enabled(dep_pkg.repo, repos)
            && !installed.contains_key(dep)
        {
            return Err(EngineError::DependencyRepoDisabled {
                package: pkg.name.clone(),
                dependency: dep.clone(),
                repo: dep_pkg.repo,
            });
        }
    }
    Ok(pkg)
}

/// Kahn's algorithm over `depends`, restricted to the planned packages.
///
/// Ready nodes are emitted in first-visit order, which keeps the web app's
/// depth-first order for packages that do not constrain each other. A dependency
/// cycle leaves no ready node; instead of hanging, the smallest remaining name
/// is emitted so the plan stays deterministic (and a test pins that).
fn order_install_like(
    catalog: &Catalog,
    items: &HashMap<String, PlanItem>,
    rank: &HashMap<String, usize>,
) -> Vec<String> {
    let names: Vec<&str> = items.keys().map(String::as_str).collect();
    let mut indegree: HashMap<&str, usize> = names.iter().map(|name| (*name, 0)).collect();
    // dependency -> packages that need it
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for name in &names {
        let Some(pkg) = catalog.by_name(name) else {
            continue;
        };
        for dep in &pkg.depends {
            if dep.as_str() != *name && indegree.contains_key(dep.as_str()) {
                *indegree.get_mut(*name).expect("planned package") += 1;
                dependents.entry(dep.as_str()).or_default().push(*name);
            }
        }
    }

    let mut emitted: HashSet<&str> = HashSet::with_capacity(names.len());
    let mut order: Vec<String> = Vec::with_capacity(names.len());
    while order.len() < names.len() {
        let mut ready: Vec<&str> = Vec::new();
        for (name, degree) in &indegree {
            if *degree == 0 && !emitted.contains(*name) {
                ready.push(*name);
            }
        }
        ready.sort_by_key(|name| (rank.get(*name).copied().unwrap_or(usize::MAX), *name));
        let next = ready.first().copied().unwrap_or_else(|| {
            // Dependency cycle: break it by name.
            indegree
                .keys()
                .copied()
                .filter(|name| !emitted.contains(*name))
                .min()
                .expect("an unemitted node exists")
        });
        emitted.insert(next);
        order.push(next.to_string());
        if let Some(children) = dependents.get(next) {
            for child in children {
                if let Some(degree) = indegree.get_mut(*child) {
                    *degree = (*degree).saturating_sub(1);
                }
            }
        }
    }
    order
}

/// Kahn's algorithm with reversed edges: a package is emitted only once nothing
/// still planned depends on it, so dependents are removed before dependencies.
///
/// Ready nodes are emitted in name order; a cycle is broken by name instead of
/// the web app's silent `break`, so every requested removal reaches the plan.
fn order_remove_like(catalog: &Catalog, removing: &BTreeSet<String>) -> Vec<String> {
    let mut indegree: HashMap<&str, usize> =
        removing.iter().map(|name| (name.as_str(), 0)).collect();
    // package -> its planned dependencies
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for name in removing {
        let Some(pkg) = catalog.by_name(name) else {
            continue;
        };
        for dep in &pkg.depends {
            if dep.as_str() != name && indegree.contains_key(dep.as_str()) {
                *indegree.get_mut(dep.as_str()).expect("planned package") += 1;
                dependents
                    .entry(name.as_str())
                    .or_default()
                    .push(dep.as_str());
            }
        }
    }

    let mut emitted: HashSet<&str> = HashSet::with_capacity(indegree.len());
    let mut order: Vec<String> = Vec::with_capacity(indegree.len());
    while order.len() < indegree.len() {
        let mut ready: Vec<&str> = Vec::new();
        for (name, degree) in &indegree {
            if *degree == 0 && !emitted.contains(*name) {
                ready.push(*name);
            }
        }
        ready.sort_unstable();
        let next = ready.first().copied().unwrap_or_else(|| {
            indegree
                .keys()
                .copied()
                .filter(|name| !emitted.contains(*name))
                .min()
                .expect("an unemitted node exists")
        });
        emitted.insert(next);
        order.push(next.to_string());
        if let Some(children) = dependents.get(next) {
            for child in children {
                if let Some(degree) = indegree.get_mut(*child) {
                    *degree = (*degree).saturating_sub(1);
                }
            }
        }
    }
    order
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

/// `planRemove`'s blocking pass: an installed package that something else
/// installed (and not being removed) still needs stops the whole removal.
fn blocked_for(
    catalog: &Catalog,
    installed: &InstalledMap,
    removing: &BTreeSet<String>,
    requested: &[String],
) -> Vec<Blocked> {
    let mut blocked = Vec::new();
    for name in ordered_names(requested, removing) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Category;

    const CATALOG_JSON: &str = include_str!("../../khaxbps-qt/resources/catalog.json");

    fn catalog() -> Catalog {
        Catalog::load_from_str(CATALOG_JSON).expect("catalog JSON parses")
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

    fn queue(entries: &[(&str, QueueAction)]) -> Vec<QueueItem> {
        entries
            .iter()
            .map(|(name, action)| QueueItem {
                name: (*name).to_string(),
                action: *action,
            })
            .collect()
    }

    fn names(items: &[PlanItem]) -> Vec<&str> {
        items.iter().map(|item| item.name.as_str()).collect()
    }

    fn reasons(items: &[PlanItem]) -> Vec<PlanReason> {
        items.iter().map(|item| item.reason).collect()
    }

    fn pkg(name: &str, version: &str, repo: RepoId, depends: &[&str]) -> Pkg {
        Pkg {
            name: name.to_string(),
            version: version.to_string(),
            repo,
            category: Category::Base,
            size: 100,
            download: 10,
            short: String::new(),
            homepage: String::new(),
            license: String::new(),
            depends: depends.iter().map(|dep| (*dep).to_string()).collect(),
            files: Vec::new(),
        }
    }

    /// Every planned dependency must appear before the package that needs it.
    fn assert_dependencies_first(catalog: &Catalog, plan: &[PlanItem]) {
        let positions: HashMap<&str, usize> = plan
            .iter()
            .enumerate()
            .map(|(index, item)| (item.name.as_str(), index))
            .collect();
        for item in plan {
            let pkg = catalog.by_name(&item.name).expect("planned package exists");
            for dep in &pkg.depends {
                if let Some(&dep_position) = positions.get(dep.as_str()) {
                    assert!(
                        dep_position < positions[item.name.as_str()],
                        "{} planned before its dependency {dep}",
                        item.name
                    );
                }
            }
        }
    }

    #[test]
    fn install_plan_puts_dependencies_first() {
        let catalog = catalog();
        let plan = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("vim", QueueAction::Install)]),
        )
        .expect("vim plans");

        assert_eq!(names(&plan), vec!["glibc", "ncurses", "vim"]);
        assert_eq!(
            reasons(&plan),
            vec![
                PlanReason::Dependency,
                PlanReason::Dependency,
                PlanReason::Requested
            ]
        );
        assert!(plan.iter().all(|item| item.action == PlanAction::Install));
        assert_eq!(plan[0].from, None);
        assert_eq!(plan[2].to.as_deref(), Some("9.1.1700_1"));

        let glibc = catalog.by_name("glibc").expect("glibc exists");
        assert_eq!(plan[0].download, glibc.download);
        assert_eq!(plan[0].size, glibc.size);
    }

    #[test]
    fn install_plan_orders_deep_dependency_chains() {
        let catalog = catalog();
        let plan = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("git", QueueAction::Install)]),
        )
        .expect("git plans");

        let mut planned = names(&plan);
        planned.sort_unstable();
        assert_eq!(
            planned,
            vec!["ca-certificates", "curl", "git", "glibc", "openssl", "zlib"]
        );
        assert_eq!(plan.last().map(|item| item.name.as_str()), Some("git"));
        assert_dependencies_first(&catalog, &plan);
    }

    #[test]
    fn install_plan_updates_outdated_dependency() {
        let catalog = catalog();
        let installed = installed(&[("glibc", inst("2.40_1", true, false))]);
        let plan = plan_install(
            &catalog,
            &installed,
            &Repos::default(),
            &queue(&[("vim", QueueAction::Install)]),
        )
        .expect("vim plans");

        assert_eq!(names(&plan), vec!["glibc", "ncurses", "vim"]);
        assert_eq!(plan[0].action, PlanAction::Update);
        assert_eq!(plan[0].reason, PlanReason::Dependency);
        assert_eq!(plan[0].from.as_deref(), Some("2.40_1"));
        assert_eq!(plan[0].to.as_deref(), Some("2.41_1"));
    }

    #[test]
    fn install_plan_reinstalls_requested_installed_package() {
        let catalog = catalog();
        // Dependencies installed too, otherwise they would be planned as well.
        let installed = installed(&[
            ("vim", inst("9.1.1700_1", true, false)),
            ("glibc", inst("2.41_1", false, false)),
            ("ncurses", inst("6.5_1", false, false)),
        ]);
        let plan = plan_install(
            &catalog,
            &installed,
            &Repos::default(),
            &queue(&[("vim", QueueAction::Install)]),
        )
        .expect("vim plans");

        assert_eq!(names(&plan), vec!["vim"]);
        assert_eq!(plan[0].action, PlanAction::Reinstall);
        assert_eq!(plan[0].reason, PlanReason::Requested);
        assert_eq!(plan[0].from.as_deref(), Some("9.1.1700_1"));
        assert_eq!(plan[0].to.as_deref(), Some("9.1.1700_1"));
    }

    #[test]
    fn install_plan_leaves_held_dependencies_alone() {
        let catalog = catalog();
        let installed = installed(&[("glibc", inst("2.40_1", true, true))]);
        let plan = plan_install(
            &catalog,
            &installed,
            &Repos::default(),
            &queue(&[("vim", QueueAction::Install)]),
        )
        .expect("vim plans");

        // glibc is held, so it is neither updated nor reinstalled.
        assert_eq!(names(&plan), vec!["ncurses", "vim"]);
    }

    #[test]
    fn install_plan_rejects_unknown_package() {
        let catalog = catalog();
        let error = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("no-such-package", QueueAction::Install)]),
        )
        .expect_err("unknown package");
        assert_eq!(
            error,
            EngineError::UnknownPackage("no-such-package".to_string())
        );
    }

    #[test]
    fn install_plan_rejects_disabled_repository() {
        let catalog = catalog();
        let error = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("steam", QueueAction::Install)]),
        )
        .expect_err("nonfree is disabled");
        assert_eq!(
            error,
            EngineError::RepoDisabled {
                package: "steam".to_string(),
                repo: RepoId::Nonfree,
            }
        );

        let repos = Repos {
            nonfree: true,
            ..Repos::default()
        };
        let plan = plan_install(
            &catalog,
            &InstalledMap::new(),
            &repos,
            &queue(&[("steam", QueueAction::Install)]),
        )
        .expect("steam plans with nonfree enabled");
        assert_eq!(plan.last().map(|item| item.name.as_str()), Some("steam"));
        assert_dependencies_first(&catalog, &plan);
    }

    #[test]
    fn install_plan_rejects_dependency_from_disabled_repository() {
        let catalog = Catalog(vec![
            pkg("app", "1_1", RepoId::Current, &["lib"]),
            pkg("lib", "1_1", RepoId::Nonfree, &[]),
        ]);
        let error = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("app", QueueAction::Install)]),
        )
        .expect_err("dependency comes from nonfree");
        assert_eq!(
            error,
            EngineError::DependencyRepoDisabled {
                package: "app".to_string(),
                dependency: "lib".to_string(),
                repo: RepoId::Nonfree,
            }
        );
    }

    #[test]
    fn update_queue_requires_installed_packages() {
        let catalog = catalog();
        let error = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("vim", QueueAction::Update)]),
        )
        .expect_err("vim is not installed");
        assert_eq!(error, EngineError::NotInstalled(vec!["vim".to_string()]));
    }

    #[test]
    fn install_plan_skips_unknown_dependencies() {
        let catalog = Catalog(vec![pkg("app", "1_1", RepoId::Current, &["ghost"])]);
        let plan = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("app", QueueAction::Install)]),
        )
        .expect("a missing catalog entry is skipped");
        assert_eq!(names(&plan), vec!["app"]);
    }

    #[test]
    fn install_plan_breaks_dependency_cycles_deterministically() {
        let catalog = Catalog(vec![
            pkg("a", "1_1", RepoId::Current, &["b"]),
            pkg("b", "1_1", RepoId::Current, &["a"]),
        ]);
        let plan = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("a", QueueAction::Install)]),
        )
        .expect("a cycle still plans");
        assert_eq!(names(&plan), vec!["a", "b"]);

        // Same input, same order: the cycle break is not hash-order dependent.
        let again = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("a", QueueAction::Install)]),
        )
        .expect("a cycle still plans");
        assert_eq!(plan, again);
    }

    #[test]
    fn system_plan_skips_held_packages() {
        let catalog = catalog();
        let mut installed = installed(&[("linux", inst("6.11.0_1", true, true))]);
        assert!(
            plan_system(&catalog, &installed, &Repos::default())
                .expect("plan")
                .is_empty()
        );
        assert_eq!(
            held_updates(&catalog, &installed, &Repos::default()),
            vec!["linux"]
        );

        installed.get_mut("linux").expect("linux").held = false;
        let plan = plan_system(&catalog, &installed, &Repos::default()).expect("plan");
        assert_eq!(names(&plan), vec!["linux"]);
        assert_eq!(plan[0].action, PlanAction::Update);
        assert_eq!(plan[0].from.as_deref(), Some("6.11.0_1"));
        assert_eq!(plan[0].to.as_deref(), Some("6.12.61_1"));
        assert!(held_updates(&catalog, &installed, &Repos::default()).is_empty());
    }

    #[test]
    fn system_plan_skips_disabled_repositories() {
        let catalog = catalog();
        let glibc = catalog.by_name("glibc").expect("glibc exists");
        let mesa = catalog.by_name("mesa").expect("mesa exists");
        let installed = installed(&[
            ("steam", inst("1.0.0.1_1", true, false)),
            ("glibc", inst(&glibc.version, true, false)),
            ("mesa", inst(&mesa.version, true, false)),
        ]);

        assert!(
            plan_system(&catalog, &installed, &Repos::default())
                .expect("plan")
                .is_empty()
        );

        let repos = Repos {
            nonfree: true,
            ..Repos::default()
        };
        let plan = plan_system(&catalog, &installed, &repos).expect("plan");
        assert_eq!(names(&plan), vec!["steam"]);
        assert_eq!(plan[0].action, PlanAction::Update);
    }

    #[test]
    fn remove_reports_blocked_dependents() {
        let catalog = catalog();
        let installed = installed(&[
            ("glibc", inst("2.41_1", true, false)),
            ("ncurses", inst("6.5_1", false, false)),
            ("vim", inst("9.1.1700_1", true, false)),
        ]);
        let plan = plan_remove(&catalog, &installed, &["glibc".to_string()], false);

        assert!(plan.items.is_empty());
        assert!(plan.is_blocked());
        assert_eq!(plan.blocked.len(), 1);
        assert_eq!(plan.blocked[0].name, "glibc");
        let mut needed_by = plan.blocked[0].needed_by.clone();
        needed_by.sort();
        assert_eq!(needed_by, vec!["ncurses", "vim"]);

        // The standalone check agrees with the planner.
        assert_eq!(
            check_blocked(&catalog, &installed, &["glibc".to_string()]),
            plan.blocked
        );
    }

    #[test]
    fn cascade_removes_dependents_first() {
        let catalog = catalog();
        let installed = installed(&[
            ("glibc", inst("2.41_1", true, false)),
            ("ncurses", inst("6.5_1", false, false)),
            ("vim", inst("9.1.1700_1", true, false)),
        ]);
        let plan = plan_remove(&catalog, &installed, &["glibc".to_string()], true);

        assert!(!plan.is_blocked());
        assert_eq!(names(&plan.items), vec!["vim", "ncurses", "glibc"]);
        assert_eq!(
            reasons(&plan.items),
            vec![
                PlanReason::Reverse,
                PlanReason::Orphan,
                PlanReason::Requested
            ]
        );
        assert!(
            plan.items
                .iter()
                .all(|item| item.action == PlanAction::Remove)
        );
        assert!(plan.items.iter().all(|item| item.download == 0));
        assert_eq!(plan.items[0].from.as_deref(), Some("9.1.1700_1"));

        let vim = catalog.by_name("vim").expect("vim exists");
        assert_eq!(plan.items[0].size, vim.size);
    }

    #[test]
    fn remove_ignores_uninstalled_names() {
        let catalog = catalog();
        let plan = plan_remove(&catalog, &InstalledMap::new(), &["vim".to_string()], false);
        assert!(plan.items.is_empty());
        assert!(!plan.is_blocked());
    }

    #[test]
    fn apply_installed_tracks_explicit_dependency_and_holds() {
        let installed = installed(&[
            ("vim", inst("9.0.0_1", true, false)),
            ("tzdata", inst("2024a_1", true, false)),
            ("nano", inst("7.0_1", false, true)),
        ]);
        let now = 1_800_000_000_000;
        let plan = vec![
            PlanItem {
                name: "vim".to_string(),
                action: PlanAction::Update,
                from: Some("9.0.0_1".to_string()),
                to: Some("9.1.1700_1".to_string()),
                reason: PlanReason::Requested,
                download: 1,
                size: 2,
            },
            PlanItem {
                name: "glibc".to_string(),
                action: PlanAction::Install,
                from: None,
                to: Some("2.41_1".to_string()),
                reason: PlanReason::Dependency,
                download: 3,
                size: 4,
            },
            PlanItem {
                name: "tzdata".to_string(),
                action: PlanAction::Remove,
                from: Some("2024a_1".to_string()),
                to: None,
                reason: PlanReason::Requested,
                download: 0,
                size: 5,
            },
        ];

        let next = apply_installed(&installed, &plan, now);

        let vim = next.get("vim").expect("vim stays installed");
        assert_eq!(vim.version, "9.1.1700_1");
        assert!(vim.explicit);
        assert_eq!(vim.at, 1_790_000_000_000, "an update keeps its timestamp");

        let glibc = next.get("glibc").expect("glibc is installed");
        assert_eq!(glibc.version, "2.41_1");
        assert!(!glibc.explicit, "a dependency is not explicit");
        assert!(!glibc.held);
        assert_eq!(glibc.at, now);

        assert!(!next.contains_key("tzdata"), "removal drops the package");
        assert_eq!(
            next.get("nano"),
            installed.get("nano"),
            "untouched packages keep their entry"
        );
    }

    #[test]
    fn apply_installed_keeps_hold_and_explicit_on_update() {
        let installed = installed(&[("vim", inst("9.0.0_1", true, true))]);
        let plan = vec![PlanItem {
            name: "vim".to_string(),
            action: PlanAction::Update,
            from: Some("9.0.0_1".to_string()),
            to: Some("9.1.1700_1".to_string()),
            reason: PlanReason::Dependency,
            download: 1,
            size: 2,
        }];
        let next = apply_installed(&installed, &plan, 42);
        let vim = next.get("vim").expect("vim");
        assert!(vim.explicit, "explicit survives a dependency update");
        assert!(vim.held, "held survives");
        assert_eq!(vim.at, 1_790_000_000_000);
    }

    #[test]
    fn repos_from_installed_detects_repo_packages() {
        let none = repos_from_installed(&InstalledMap::new());
        assert_eq!(none, Repos::default());

        let installed = installed(&[
            ("void-repo-nonfree", inst("1_1", true, false)),
            ("void-repo-debug", inst("1_1", true, false)),
        ]);
        assert_eq!(
            repos_from_installed(&installed),
            Repos {
                nonfree: true,
                multilib: false,
                debug: true,
            }
        );
    }

    #[test]
    fn update_signature_tracks_the_update_list() {
        let catalog = catalog();
        let repos = Repos::default();
        let empty = update_signature(&catalog, &InstalledMap::new(), &repos);

        let bash = catalog.by_name("bash").expect("bash exists");
        let vim_outdated = installed(&[
            ("vim", inst("9.0.0_1", true, false)),
            ("bash", inst(&bash.version, true, false)),
        ]);
        let with_vim = update_signature(&catalog, &vim_outdated, &repos);
        assert_ne!(with_vim, empty, "an update appears");

        // An up-to-date package does not change the pending update list.
        let vim_only = installed(&[("vim", inst("9.0.0_1", true, false))]);
        assert_eq!(update_signature(&catalog, &vim_only, &repos), with_vim);

        // Same content, different insertion order: still the same fingerprint.
        let reordered = installed(&[
            ("bash", inst(&bash.version, true, false)),
            ("vim", inst("9.0.0_1", true, false)),
        ]);
        assert_eq!(update_signature(&catalog, &reordered, &repos), with_vim);

        let mut more = vim_outdated.clone();
        more.insert("linux".to_string(), inst("6.11.0_1", true, false));
        let with_linux = update_signature(&catalog, &more, &repos);
        assert_ne!(with_linux, with_vim, "another update appears");
        assert_ne!(with_linux, empty);
    }

    #[test]
    fn plan_totals_sum_download_and_disk_use() {
        let catalog = catalog();
        let install = plan_install(
            &catalog,
            &InstalledMap::new(),
            &Repos::default(),
            &queue(&[("vim", QueueAction::Install)]),
        )
        .expect("vim plans");
        let totals = plan_totals(&install);
        assert_eq!(
            totals.download,
            install.iter().map(|item| item.download).sum::<u64>()
        );
        assert_eq!(
            totals.added,
            install.iter().map(|item| item.size).sum::<u64>()
        );
        assert_eq!(totals.removed, 0);

        let installed = installed(&[
            ("glibc", inst("2.41_1", true, false)),
            ("ncurses", inst("6.5_1", false, false)),
            ("vim", inst("9.1.1700_1", true, false)),
        ]);
        let removal = plan_remove(&catalog, &installed, &["glibc".to_string()], true);
        let totals = plan_totals(&removal.items);
        assert_eq!(totals.download, 0);
        assert_eq!(totals.added, 0);
        assert_eq!(
            totals.removed,
            removal.items.iter().map(|item| item.size).sum::<u64>()
        );
    }

    #[test]
    fn queue_plan_combines_installs_updates_and_removals() {
        let catalog = catalog();
        let installed = installed(&[("tzdata", inst("2024a_1", true, false))]);
        let plan = plan_queue(
            &catalog,
            &installed,
            &Repos::default(),
            &queue(&[
                ("vim", QueueAction::Install),
                ("tzdata", QueueAction::Remove),
            ]),
        )
        .expect("queue plans");

        assert!(!plan.is_blocked());
        assert_eq!(
            names(&plan.items),
            vec!["glibc", "ncurses", "vim", "tzdata"]
        );
        assert_eq!(plan.items[3].action, PlanAction::Remove);
        assert_eq!(plan.items[3].reason, PlanReason::Requested);
    }

    #[test]
    fn queue_plan_reports_blocked_removals() {
        let catalog = catalog();
        let installed = installed(&[
            ("glibc", inst("2.41_1", true, false)),
            ("vim", inst("9.1.1700_1", true, false)),
        ]);
        let plan = plan_queue(
            &catalog,
            &installed,
            &Repos::default(),
            &queue(&[("glibc", QueueAction::Remove)]),
        )
        .expect("planning does not fail");

        assert!(plan.is_blocked());
        assert!(plan.items.is_empty());
        assert_eq!(plan.blocked[0].name, "glibc");
        assert_eq!(plan.blocked[0].needed_by, vec!["vim"]);
    }
}
