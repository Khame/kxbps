//! The package catalog: the embedded package table and the lookups built on it.
//!
//! The data itself is generated from the web app by `scripts/gen-catalog.mjs`
//! and ships as `khaxbps-qt/resources/catalog.json`, where the QML module embeds
//! it into the binary. This crate never reads the resource itself: the Qt layer
//! hands the file contents to [`Catalog::load_from_str`], which keeps the core
//! free of any Qt (or filesystem) dependency.

use crate::types::{Inst, Pkg, QueryMode, RepoId, Repos, UpdateInfo};
use regex::RegexBuilder;
use std::collections::HashMap;

/// Packages whose updates are announced as security relevant, mirroring the
/// web app's `SECURITY_UPDATES` set.
pub const SECURITY_UPDATES: [&str; 4] = ["linux", "linux-headers", "xbps", "curl"];

/// Whether `name` belongs to the curated security set.
pub fn is_security_update(name: &str) -> bool {
    SECURITY_UPDATES.contains(&name)
}

/// Whether a repository is enabled. `current` is always enabled; the other
/// repositories depend on the user's `/etc/xbps.d` configuration.
pub fn repo_enabled(repo: RepoId, repos: &Repos) -> bool {
    match repo {
        RepoId::Current => true,
        RepoId::Nonfree => repos.nonfree,
        RepoId::Multilib => repos.multilib,
        RepoId::Debug => repos.debug,
    }
}

/// Everything that can go wrong while building a catalog.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The JSON does not match [`Pkg`].
    #[error("package catalog JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON parsed but contains no packages.
    #[error("package catalog is empty")]
    Empty,
}

/// The package catalog, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog(pub Vec<Pkg>);

impl Catalog {
    /// Parses a catalog from the JSON produced by `scripts/gen-catalog.mjs`.
    ///
    /// The Qt layer reads `:/qt/qml/org/kde/khaxbps/resources/catalog.json` with
    /// `QFile` and passes its contents here.
    pub fn load_from_str(json: &str) -> Result<Self, CatalogError> {
        let packages: Vec<Pkg> = serde_json::from_str(json)?;
        if packages.is_empty() {
            return Err(CatalogError::Empty);
        }
        Ok(Catalog(packages))
    }

    /// All packages, in catalog order.
    pub fn packages(&self) -> &[Pkg] {
        &self.0
    }

    /// Number of packages in the catalog.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the catalog holds no packages. Only reachable for a `Catalog`
    /// built by hand; [`Catalog::load_from_str`] rejects empty input.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Exact lookup by package name.
    pub fn by_name(&self, name: &str) -> Option<&Pkg> {
        self.0.iter().find(|pkg| pkg.name == name)
    }

    /// Packages that depend on `name`, in catalog order — `xbps-query -X`.
    pub fn revdeps(&self, name: &str) -> Vec<&Pkg> {
        self.0
            .iter()
            .filter(|pkg| pkg.depends.iter().any(|dep| dep == name))
            .collect()
    }

    /// Runs an `xbps-query` mode against the catalog and returns the matching
    /// packages, in the order the web app's `runQuery` would print them.
    ///
    /// `prop` is the `-p` property name. It is accepted for call-site parity
    /// with the web app (`runQuery(mode, pattern, prop, …)`), but a property
    /// only changes how a hit is *rendered*, never which package matches, so
    /// matching ignores it.
    ///
    /// Modes that select installed packages (`-s`, `-l`, `-m`, `-O`, `-H`)
    /// cannot be fully resolved from the catalog alone; the notes in the match
    /// arms below say what the catalog contributes and what the caller adds.
    pub fn search(&self, query: &str, mode: QueryMode, prop: &str) -> Vec<&Pkg> {
        let _ = prop;
        let pattern = query.trim();
        match mode {
            // `-Rs` searches the repository pool and `-s` the installed set, but
            // both use the same matcher: a case-insensitive regular expression,
            // falling back to a case-insensitive substring when the pattern is
            // not valid regex. The pool differs, so callers filter `-s` results
            // by the installed map (the catalog cannot know what is installed).
            QueryMode::Rs | QueryMode::S => self.search_text(pattern),
            // `-o` matches a path against the file lists.
            QueryMode::O => self.search_file(pattern),
            // Argument-less listings. The catalog contributes the package set;
            // the caller narrows it with the installed map (`-l` everything,
            // `-m` explicit, `-O` orphans, `-H` held) or with the repository
            // configuration (`-L`).
            QueryMode::L | QueryMode::M | QueryMode::Orphans | QueryMode::H | QueryMode::RepoL => {
                self.sorted_by_name()
            }
            // `-x` expands to the package plus its dependencies, `-X` to the
            // package plus its reverse dependencies, exactly like the web app.
            QueryMode::X | QueryMode::RevX => {
                let Some(pkg) = self.match_package(pattern) else {
                    return Vec::new();
                };
                let related: Vec<&Pkg> = if mode == QueryMode::X {
                    pkg.depends
                        .iter()
                        .filter_map(|dep| self.by_name(dep))
                        .collect()
                } else {
                    self.revdeps(&pkg.name)
                };
                let mut out = vec![pkg];
                for candidate in related {
                    if !out.iter().any(|seen| seen.name == candidate.name) {
                        out.push(candidate);
                    }
                }
                out
            }
            // Everything else takes a single package name: `-R`, `show`, `-f`,
            // `-p`.
            QueryMode::R | QueryMode::Show | QueryMode::F | QueryMode::P => {
                self.match_package(pattern).into_iter().collect()
            }
        }
    }

    /// Updates whose installed version differs from the catalog version.
    ///
    /// Mirrors the web app's `updates()`: held packages are skipped, packages
    /// from disabled repositories are skipped, and the result is sorted by name.
    pub fn updates(&self, installed: &HashMap<String, Inst>, repos: &Repos) -> Vec<UpdateInfo> {
        let mut list: Vec<UpdateInfo> = self
            .0
            .iter()
            .filter_map(|pkg| {
                let inst = installed.get(&pkg.name)?;
                if inst.held {
                    return None;
                }
                if !repo_enabled(pkg.repo, repos) {
                    return None;
                }
                if inst.version == pkg.version {
                    return None;
                }
                Some(UpdateInfo {
                    name: pkg.name.clone(),
                    from: inst.version.clone(),
                    to: pkg.version.clone(),
                    security: is_security_update(&pkg.name),
                    download: pkg.download,
                })
            })
            .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// All packages sorted by name.
    fn sorted_by_name(&self) -> Vec<&Pkg> {
        let mut packages: Vec<&Pkg> = self.0.iter().collect();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        packages
    }

    /// Regex-or-substring search over package name and short description.
    fn search_text(&self, pattern: &str) -> Vec<&Pkg> {
        if pattern.is_empty() {
            return Vec::new();
        }
        match RegexBuilder::new(pattern).case_insensitive(true).build() {
            Ok(re) => self
                .0
                .iter()
                .filter(|pkg| re.is_match(&pkg.name) || re.is_match(&pkg.short))
                .collect(),
            Err(_) => {
                // Same fallback as the web app when `new RegExp(pat, "i")` throws.
                let needle = pattern.to_lowercase();
                self.0
                    .iter()
                    .filter(|pkg| {
                        pkg.name.to_lowercase().contains(&needle)
                            || pkg.short.to_lowercase().contains(&needle)
                    })
                    .collect()
            }
        }
    }

    /// File-owner search: `xbps-query -o <path>`.
    fn search_file(&self, pattern: &str) -> Vec<&Pkg> {
        if pattern.is_empty() {
            return Vec::new();
        }
        self.0
            .iter()
            .filter(|pkg| {
                pkg.files.iter().any(|file| {
                    file == pattern || file.starts_with(pattern) || file.contains(pattern)
                })
            })
            .collect()
    }

    /// Resolves a package name the way the web app's `matchPkg` does: exact
    /// name, then the name without its `-<version>` suffix, then the first
    /// package whose name contains the pattern.
    fn match_package(&self, pattern: &str) -> Option<&Pkg> {
        let raw = pattern.strip_suffix(".x86_64").unwrap_or(pattern);
        if raw.is_empty() {
            return None;
        }
        if let Some(pkg) = self.by_name(raw) {
            return Some(pkg);
        }
        if let Some(cut) = version_suffix_start(raw)
            && let Some(pkg) = self.by_name(&raw[..cut])
        {
            return Some(pkg);
        }
        self.0.iter().find(|pkg| pkg.name.contains(raw))
    }
}

/// Byte index where a `-<digit>…` version suffix starts, matching the web app's
/// `raw.replace(/-\d.*$/, "")`.
fn version_suffix_start(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    (0..bytes.len().saturating_sub(1))
        .find(|&index| bytes[index] == b'-' && bytes[index + 1].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        Category, DialogState, PlanAction, PlanItem, PlanReason, Prefs, ProbeState,
    };

    /// The canonical catalog lives beside the QML module so it can be embedded
    /// into the binary. The core crate only reads it from tests, which keeps the
    /// crate itself free of Qt and of runtime file access.
    const CATALOG_JSON: &str = include_str!("../../khaxbps-qt/resources/catalog.json");

    fn catalog() -> Catalog {
        Catalog::load_from_str(CATALOG_JSON).expect("catalog JSON parses")
    }

    fn names<'a>(packages: &'a [&'a Pkg]) -> Vec<&'a str> {
        packages.iter().map(|pkg| pkg.name.as_str()).collect()
    }

    fn inst(version: &str, explicit: bool, held: bool) -> Inst {
        Inst {
            version: version.to_string(),
            explicit,
            held,
            at: 1_790_000_000_000,
        }
    }

    #[test]
    fn catalog_has_115_packages() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 115);
    }

    #[test]
    fn catalog_entries_are_complete() {
        let catalog = catalog();
        for pkg in catalog.packages() {
            assert!(!pkg.name.is_empty(), "package without a name");
            assert!(!pkg.version.is_empty(), "{} has no version", pkg.name);
            assert!(pkg.size > 0, "{} has no installed size", pkg.name);
            assert!(pkg.download > 0, "{} has no download size", pkg.name);
            assert!(!pkg.short.is_empty(), "{} has no description", pkg.name);
            assert!(!pkg.files.is_empty(), "{} has no files", pkg.name);
        }
        // Every dependency must resolve, otherwise plans and revdeps break.
        for pkg in catalog.packages() {
            for dep in &pkg.depends {
                assert!(
                    catalog.by_name(dep).is_some(),
                    "{} depends on unknown {dep}",
                    pkg.name
                );
            }
        }
    }

    #[test]
    fn catalog_round_trips_through_json() {
        let catalog = catalog();
        let json = serde_json::to_string(&catalog.0).expect("serialize");
        let reparsed = Catalog::load_from_str(&json).expect("reparse");
        assert_eq!(catalog, reparsed);
    }

    #[test]
    fn sizes_are_byte_counts() {
        let catalog = catalog();
        let vim = catalog.by_name("vim").expect("vim exists");
        assert_eq!(vim.size, 4 * 1024 * 1024);
        assert_eq!(vim.download, (1.4f64 * 1024.0 * 1024.0).round() as u64);
    }

    #[test]
    fn search_vim_in_repo_by_name() {
        let catalog = catalog();
        let hits = catalog.search("vim", QueryMode::Rs, "name");
        assert!(names(&hits).contains(&"vim"), "search did not return vim");
    }

    #[test]
    fn search_is_case_insensitive() {
        let catalog = catalog();
        assert!(names(&catalog.search("VIM", QueryMode::Rs, "name")).contains(&"vim"));
    }

    #[test]
    fn search_supports_regex_anchors() {
        let catalog = catalog();
        assert_eq!(
            names(&catalog.search("^git$", QueryMode::Rs, "name")),
            vec!["git"]
        );
    }

    #[test]
    fn search_matches_short_description() {
        let catalog = catalog();
        let hits = catalog.search("edytor", QueryMode::Rs, "name");
        assert!(names(&hits).contains(&"vim"));
        assert!(names(&hits).contains(&"nano"));
    }

    #[test]
    fn search_falls_back_to_substring_for_invalid_regex() {
        let catalog = catalog();
        // `[` cannot be compiled as a regex, so the substring path runs and
        // nothing matches — the important part is that it does not panic.
        assert!(catalog.search("[", QueryMode::Rs, "name").is_empty());
        // `Rust (` is invalid as a regex (unclosed group) but a literal
        // substring of rust's short description, "Kompilator Rust (rustc,
        // cargo)". This fails if the `Err(_)` branch ever stops matching
        // instead of only returning an empty list.
        assert_eq!(
            names(&catalog.search("Rust (", QueryMode::Rs, "name")),
            vec!["rust"]
        );
    }

    #[test]
    fn search_empty_pattern_returns_nothing() {
        let catalog = catalog();
        assert!(catalog.search("   ", QueryMode::Rs, "name").is_empty());
    }

    #[test]
    fn search_file_owner_finds_package() {
        let catalog = catalog();
        let hits = catalog.search("/usr/bin/vim", QueryMode::O, "");
        assert_eq!(names(&hits), vec!["vim"]);
        // Prefix matching, like `xbps-query -o /usr/bin`.
        assert!(names(&catalog.search("/usr/bin", QueryMode::O, "")).len() > 10);
    }

    #[test]
    fn single_package_modes_resolve_names() {
        let catalog = catalog();
        for mode in [QueryMode::R, QueryMode::Show, QueryMode::F, QueryMode::P] {
            assert_eq!(
                names(&catalog.search("vim", mode, "pkgver")),
                vec!["vim"],
                "{mode:?}"
            );
        }
        // The web app strips a trailing architecture and a version suffix.
        assert_eq!(
            names(&catalog.search("vim.x86_64", QueryMode::R, "")),
            vec!["vim"]
        );
        assert_eq!(
            names(&catalog.search("vim-9.1.1700_1", QueryMode::R, "")),
            vec!["vim"]
        );
        assert!(
            catalog
                .search("nope-not-a-package", QueryMode::R, "")
                .is_empty()
        );
    }

    #[test]
    fn search_x_expands_dependencies() {
        let catalog = catalog();
        let vim = catalog.by_name("vim").expect("vim exists");
        let hits = catalog.search("vim", QueryMode::X, "");
        let hit_names = names(&hits);

        // The package comes first, then its dependencies in catalog order.
        let expected: Vec<&str> = std::iter::once(vim.name.as_str())
            .chain(vim.depends.iter().map(String::as_str))
            .collect();
        assert_eq!(hit_names, expected);

        // Deduplicated, and the same as a version-suffixed argument.
        let mut unique = hit_names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), hit_names.len(), "duplicate hits");
        assert_eq!(
            names(&catalog.search("vim-9.1.1700_1", QueryMode::X, "")),
            hit_names
        );
    }

    #[test]
    fn search_revx_expands_reverse_dependencies() {
        let catalog = catalog();
        let hits = catalog.search("ncurses", QueryMode::RevX, "");
        let hit_names = names(&hits);

        assert_eq!(hit_names[0], "ncurses", "package must come first");
        assert_eq!(&hit_names[1..], names(&catalog.revdeps("ncurses")));
        assert!(hit_names.contains(&"vim"));
        assert!(!hit_names[1..].contains(&"ncurses"));

        // A package nobody depends on still lists itself, like the web app.
        assert_eq!(
            names(&catalog.search("vim", QueryMode::RevX, "")),
            vec!["vim"]
        );
    }

    #[test]
    fn listing_modes_return_sorted_packages() {
        let catalog = catalog();
        for mode in [
            QueryMode::L,
            QueryMode::M,
            QueryMode::Orphans,
            QueryMode::H,
            QueryMode::RepoL,
        ] {
            let hits = catalog.search("", mode, "");
            let hit_names = names(&hits);
            assert_eq!(hit_names.len(), 115, "{mode:?}");
            assert!(
                hit_names.windows(2).all(|pair| pair[0] < pair[1]),
                "{mode:?} not sorted"
            );
        }
    }

    #[test]
    fn revdeps_lists_dependents() {
        let catalog = catalog();
        let revs = catalog.revdeps("glibc");
        let rev_names = names(&revs);
        assert!(rev_names.contains(&"bash"));
        assert!(rev_names.contains(&"coreutils"));
        assert!(!rev_names.contains(&"glibc"));
    }

    #[test]
    fn updates_skip_held_unchanged_and_disabled_repos() {
        let catalog = catalog();
        let vim = catalog.by_name("vim").expect("vim exists");
        let bash = catalog.by_name("bash").expect("bash exists");
        let steam = catalog.by_name("steam").expect("steam exists");
        assert_eq!(steam.repo, RepoId::Nonfree);

        let mut installed = HashMap::new();
        installed.insert("vim".to_string(), inst("9.0.0_1", true, false));
        installed.insert("bash".to_string(), inst(&bash.version, true, false));
        installed.insert("linux".to_string(), inst("6.11.0_1", true, true));
        installed.insert("steam".to_string(), inst("1.0.0_1", true, false));

        let repos = Repos::default();
        let updates = catalog.updates(&installed, &repos);
        let update_names: Vec<&str> = updates.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(update_names, vec!["vim"]);

        let vim_update = &updates[0];
        assert_eq!(vim_update.from, "9.0.0_1");
        assert_eq!(vim_update.to, vim.version);
        assert_eq!(vim_update.download, vim.download);
        assert!(!vim_update.security);

        let repos = Repos {
            nonfree: true,
            ..Repos::default()
        };
        let with_nonfree = catalog.updates(&installed, &repos);
        let update_names: Vec<&str> = with_nonfree.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(update_names, vec!["steam", "vim"]);
    }

    #[test]
    fn updates_flag_security_packages() {
        let catalog = catalog();
        let mut installed = HashMap::new();
        installed.insert("linux".to_string(), inst("6.11.0_1", true, false));
        let updates = catalog.updates(&installed, &Repos::default());
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, "linux");
        assert!(updates[0].security);
    }

    #[test]
    fn repo_enabled_matches_web_app() {
        let none = Repos::default();
        assert!(repo_enabled(RepoId::Current, &none));
        assert!(!repo_enabled(RepoId::Nonfree, &none));
        let all = Repos {
            nonfree: true,
            multilib: true,
            debug: true,
        };
        assert!(repo_enabled(RepoId::Nonfree, &all));
        assert!(repo_enabled(RepoId::Multilib, &all));
        assert!(repo_enabled(RepoId::Debug, &all));
    }

    #[test]
    fn query_mode_wire_values_match_web_app() {
        let cases = [
            (QueryMode::Rs, "\"Rs\""),
            (QueryMode::S, "\"s\""),
            (QueryMode::L, "\"l\""),
            (QueryMode::R, "\"R\""),
            (QueryMode::Show, "\"show\""),
            (QueryMode::X, "\"x\""),
            (QueryMode::RevX, "\"X\""),
            (QueryMode::F, "\"f\""),
            (QueryMode::O, "\"o\""),
            (QueryMode::M, "\"m\""),
            (QueryMode::Orphans, "\"O\""),
            (QueryMode::H, "\"H\""),
            (QueryMode::RepoL, "\"L\""),
            (QueryMode::P, "\"p\""),
        ];
        for (mode, json) in cases {
            assert_eq!(serde_json::to_string(&mode).expect("serialize"), json);
            let parsed: QueryMode = serde_json::from_str(json).expect("deserialize");
            assert_eq!(parsed, mode);
        }
        assert_eq!(QueryMode::Show.flag(), "");
        assert_eq!(QueryMode::RevX.flag(), "-X");
        assert!(!QueryMode::L.needs_pattern());
        assert!(QueryMode::Rs.needs_pattern());
    }

    #[test]
    fn dialog_state_uses_kind_tag() {
        let dialog = DialogState::Plan {
            title: "Plan".to_string(),
            items: vec![PlanItem {
                name: "vim".to_string(),
                action: PlanAction::Install,
                from: None,
                to: Some("9.1.1700_1".to_string()),
                reason: PlanReason::Requested,
                download: 1,
                size: 2,
            }],
            note: None,
        };
        let json = serde_json::to_value(&dialog).expect("serialize");
        assert_eq!(json["kind"], "plan");
        assert_eq!(json["items"][0]["action"], "install");
        assert_eq!(json["items"][0]["reason"], "requested");
        assert!(json["items"][0].get("from").is_none());

        let blocked = DialogState::Blocked {
            blocked: vec![crate::types::Blocked {
                name: "glibc".to_string(),
                needed_by: vec!["bash".to_string()],
            }],
            names: vec!["glibc".to_string()],
        };
        let json = serde_json::to_value(&blocked).expect("serialize");
        assert_eq!(json["kind"], "blocked");
        assert_eq!(json["blocked"][0]["neededBy"][0], "bash");

        let about: DialogState = serde_json::from_str("{\"kind\":\"about\"}").expect("deserialize");
        assert_eq!(about, DialogState::About);
    }

    #[test]
    fn probe_state_uses_state_tag() {
        let ok = ProbeState::Ok { ms: 42 };
        assert_eq!(
            serde_json::to_string(&ok).expect("serialize"),
            "{\"state\":\"ok\",\"ms\":42}"
        );
        let idle: ProbeState = serde_json::from_str("{\"state\":\"idle\"}").expect("deserialize");
        assert_eq!(idle, ProbeState::Idle);
    }

    #[test]
    fn prefs_and_category_use_web_app_field_names() {
        let prefs = Prefs {
            notify_on_start: true,
            desktop: false,
            dismissed: String::new(),
        };
        let json = serde_json::to_value(&prefs).expect("serialize");
        assert_eq!(json["notifyOnStart"], true);
        assert!(json.get("notify_on_start").is_none());

        assert_eq!(
            serde_json::to_string(&Category::Devel).expect("serialize"),
            "\"devel\""
        );
    }
}
