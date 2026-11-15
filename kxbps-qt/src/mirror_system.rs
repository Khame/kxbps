// The system half of the mirror view: what xbps actually uses.
//
// Everything here is a plain function over values — the config file's text, the
// `xbps-query -L` listing and the `.conf` files — so the rules behind
// `systemMirror`, `systemRepos` and `pending` are testable without a QObject, a
// GUI or root. The backend only reads, starts the worker and stores the result.

use std::path::Path;

use kxbps_core::mirror_config::{ConfigError, SystemConfig, read_config};
use kxbps_core::xbps::{XbpsError, parse_repos};
use serde::{Deserialize, Serialize};

use crate::xbps_runner::{READ_TIMEOUT_MS, run};
use crate::xbps_snapshot::{Invoke, SharedInvoke};

/// The two directories xbps reads its repository files from, in its own order.
/// The paths come from the core, which writes into the first one.
pub const CONFIG_DIRS: &[&str] = &[
    kxbps_core::repository_edit::CONFIG_DIR,
    kxbps_core::repository_edit::SYSTEM_CONFIG_DIR,
];

/// The `systemRepos` wire value: a JSON array of `{url, count, signed, file}`.
///
/// `file` is the `.conf` file that defines the repository; empty when no file
/// names the URL. The array is camelCase, like every other snapshot that
/// crosses a shim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemRepo {
    pub url: String,
    pub count: i64,
    pub signed: bool,
    pub file: String,
}

/// Builds `systemRepos` from `xbps-query -L` and the config files.
///
/// The listing is the authority for what xbps uses and in what order; the files
/// only add the path that defines the URL, because `-L` prints URLs and not
/// paths. A URL no file claims keeps an empty `file` rather than an invented
/// one. A listing this version cannot read is an error, because a half-read
/// repository list is the one lie this layer must not tell.
pub fn repos_from_listing(
    listing: &str,
    files: &[(String, String)],
) -> Result<Vec<SystemRepo>, XbpsError> {
    let entries = parse_repos(listing)?;
    Ok(entries
        .into_iter()
        .map(|entry| {
            let needle = format!("repository={}", entry.url);
            let file = files
                .iter()
                .find(|(_, content)| content.lines().any(|line| line.trim() == needle))
                .map(|(path, _)| path.clone())
                .unwrap_or_default();
            SystemRepo {
                url: entry.url,
                count: entry.count,
                signed: entry.signed,
                file,
            }
        })
        .collect())
}

/// The whole system half of the backend: what xbps uses and what defines it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    /// The base URL of `00-repository-main.conf`; empty when unreadable.
    pub system_mirror: String,
    /// The `errorMirrorConfig*` key when the file could not be read or parsed.
    pub error_key: &'static str,
    /// The arguments of [`Self::error_key`], in `%1…` order.
    pub error_args: Vec<String>,
    /// `systemRepos` as JSON; `[]` when `-L` could not be read.
    pub repos_json: String,
    /// The full URL of the mirror xbps resolves, `<base>current`.
    pub current_url: String,
}

impl SystemState {
    /// `pending` for a saved selection: the selection is not what xbps uses.
    ///
    /// A base URL and its `current` URL are the same repository, so
    /// `https://x/` and a `-L` entry of `https://x/current` compare equal. An
    /// unreadable configuration counts as pending for a non-empty selection: the
    /// UI then offers the apply instead of claiming the selection is in use.
    ///
    /// `selected` is a value the mirror list offers or the store holds, and both
    /// end in exactly one `/` (`check_base_url` normalises it), so `<selection>`
    /// and `current` join without a separator.
    pub fn pending(&self, selected: &str) -> bool {
        if selected.is_empty() {
            return false;
        }
        if self.system_mirror.is_empty() && self.current_url.is_empty() {
            return true;
        }
        let selected_current = format!("{selected}current");
        selected != self.system_mirror && selected_current != self.current_url
    }
}

/// The error key and arguments of an unreadable configuration.
pub fn config_error(error: &ConfigError) -> (&'static str, Vec<String>) {
    (error.key(), error.args())
}

/// Turns one refresh's two answers into the backend's state.
pub fn system_state(
    config: Result<SystemConfig, ConfigError>,
    listing: &str,
    files: &[(String, String)],
) -> SystemState {
    let (system_mirror, error_key, error_args, current_url) = match config {
        Ok(config) => (config.base.clone(), "", Vec::new(), config.current_url()),
        Err(error) => {
            let (key, args) = config_error(&error);
            (String::new(), key, args, String::new())
        }
    };
    let repos_json = match repos_from_listing(listing, files) {
        Ok(repos) => serde_json::to_string(&repos).unwrap_or_else(|_| String::from("[]")),
        Err(error) => {
            eprintln!("kxbps: xbps-query -L was not understood: {error}");
            String::from("[]")
        }
    };
    SystemState {
        system_mirror,
        error_key,
        error_args,
        repos_json,
        current_url,
    }
}

/// Every `.conf` file under `dirs`, as `(path, content)`, sorted by path.
///
/// Read as the invoking user: `/etc/xbps.d` is world-readable, which is exactly
/// why the repositories tab needs no root. An unreadable file is skipped with a
/// log line instead of failing the whole list.
pub fn read_config_files(dirs: &[&str]) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        paths.sort();
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(content) => files.push((path.display().to_string(), content)),
                Err(error) => eprintln!("kxbps: {} unreadable: {error}", path.display()),
            }
        }
    }
    files
}

/// Runs `xbps-query -L` through the read transport and answers its stdout.
///
/// The listing is not parsed here: [`repos_from_listing`] does that once, so a
/// format this version does not know is reported next to the rows it would have
/// produced. A command that could not start, timed out or exited non-zero
/// answers an empty listing, which the caller renders as "no repositories" and
/// logs instead of inventing rows.
fn read_repo_listing(invoke: &Invoke) -> String {
    let argv = vec![
        String::from(crate::xbps_runner::XPBS_QUERY),
        String::from("-L"),
    ];
    let program = argv[0].clone();
    let output = match run(&argv, READ_TIMEOUT_MS, invoke) {
        Ok(output) => output,
        Err(error) => {
            eprintln!("kxbps: {program} failed: {error}");
            return String::new();
        }
    };
    if output.timed_out || output.exit_code != 0 {
        eprintln!(
            "kxbps: {program} -L exited {}: {}",
            output.exit_code,
            output.stderr.trim()
        );
        return String::new();
    }
    output.stdout
}

/// The production read of the system half: the config file, `xbps-query -L` and
/// the `.conf` files, all without root.
pub fn read_system_state(config_path: &Path, invoke: &Invoke) -> SystemState {
    let config = read_config(config_path);
    let listing = read_repo_listing(invoke);
    let files = read_config_files(CONFIG_DIRS);
    system_state(config, &listing, &files)
}

/// The production transport, the same one the installed backend reads with.
pub fn production_invoke() -> SharedInvoke {
    crate::xbps_bridge::production_invoke()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MAIN: &str = "repository=https://repo-de.voidlinux.org/current\n";
    /// The same repository as a test asks for it: the bare URL [`parse`] wraps.
    const MAIN_URL: &str = "https://repo-de.voidlinux.org/current";
    const LISTING: &str = "14836 https://repo-de.voidlinux.org/current (RSA signed)\n\
     5 https://khame.github.io/void-packages/ (RSA signed)\n\
     -1 https://repo-fi.voidlinux.org/current (RSA maybe-signed)\n";

    fn files() -> Vec<(String, String)> {
        vec![
            (
                String::from("/etc/xbps.d/10-khame.conf"),
                String::from("repository=https://khame.github.io/void-packages/\n"),
            ),
            (
                String::from("/usr/share/xbps.d/xbps.conf"),
                String::from("# repository-lock=/var/lock/xbps\n"),
            ),
        ]
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kxbps-mirror-system-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn system_mirror_is_read_from_the_config_file() {
        let dir = tempdir("read");
        let path = dir.join("00-repository-main.conf");
        std::fs::write(&path, MAIN).expect("fixture");
        let listing = "14836 https://repo-de.voidlinux.org/current (RSA signed)\n";
        let invoke: SharedInvoke = std::sync::Arc::new(move |_, _| {
            format!(
                r#"{{"exitCode":0,"stdout":{},"stderr":"","timedOut":false}}"#,
                serde_json::json!(listing)
            )
        });
        let state = read_system_state(&path, &*invoke);
        assert_eq!(state.system_mirror, "https://repo-de.voidlinux.org/");
        assert_eq!(state.current_url, "https://repo-de.voidlinux.org/current");
        assert_eq!(state.error_key, "");
        assert!(state.repos_json.contains("repo-de.voidlinux.org/current"));
        let repos: Vec<SystemRepo> = serde_json::from_str(&state.repos_json).expect("json");
        assert_eq!(repos[0].count, 14836);
        assert!(repos[0].signed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_config_is_a_key_and_pending() {
        let dir = tempdir("missing");
        let path = dir.join("00-repository-main.conf");
        let state = read_system_state(&path, &|_, _| {
            String::from(r#"{"exitCode":1,"stdout":"","stderr":"","timedOut":false}"#)
        });
        assert_eq!(state.system_mirror, "");
        assert_eq!(state.error_key, "errorMirrorConfigRead");
        assert_eq!(state.error_args, [path.display().to_string()]);
        assert_eq!(state.repos_json, "[]", "a failed -L invents no rows");
        assert!(
            state.pending("https://repo-default.voidlinux.org/"),
            "an unreadable config offers the apply"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_is_true_when_the_saved_mirror_differs_from_the_system() {
        let ok = |url: &str| system_state(Ok(parse(url)), "", &[]);
        assert!(
            !ok("https://repo-de.voidlinux.org/current").pending("https://repo-de.voidlinux.org/"),
            "the same mirror in its base spelling is not pending"
        );
        assert!(
            ok("https://repo-de.voidlinux.org/current")
                .pending("https://repo-default.voidlinux.org/")
        );
        assert!(
            !ok(MAIN_URL).pending(""),
            "no selection at all is not a pending change"
        );
        assert!(
            !ok("https://repo-de.voidlinux.org/").pending("https://repo-de.voidlinux.org/"),
            "a base in the file and a base in the selection agree"
        );
    }

    #[test]
    fn a_selection_compared_with_the_listing_also_clears_pending() {
        // The config file may be a base URL (`already_adapted == false`), and
        // then only the `-L` entry says what xbps resolves.
        let state = system_state(
            Ok(parse("https://repo-de.voidlinux.org/")),
            "14836 https://repo-de.voidlinux.org/current (RSA signed)\n",
            &[],
        );
        assert_eq!(state.system_mirror, "https://repo-de.voidlinux.org/");
        assert!(!state.pending("https://repo-de.voidlinux.org/"));
        assert!(state.pending("https://repo-default.voidlinux.org/"));
    }

    #[test]
    fn a_selection_whose_current_url_matches_the_listing_is_not_pending() {
        // The config file's base is empty in this state (a file this version
        // cannot read), so only the `-L` entry says what xbps resolves. A
        // selection whose base differs from `systemMirror` and whose
        // `<base>current` is exactly that entry is the repository xbps uses.
        let state = SystemState {
            system_mirror: String::new(),
            error_key: "errorMirrorConfigRead",
            error_args: vec![String::from("/etc/xbps.d/00-repository-main.conf")],
            repos_json: String::from("[]"),
            current_url: String::from("https://a.example/current"),
        };
        assert!(
            !state.pending("https://a.example/"),
            "the listing names the selection's current URL"
        );
        assert!(
            state.pending("https://b.example/"),
            "another base is still a pending change"
        );
        // The comparison is the selection as given: a value the list never
        // offers (no trailing `/`) is not what xbps uses until it is stored.
        assert!(state.pending("https://a.example"));
    }

    #[test]
    fn the_repositories_carry_the_file_that_defines_them() {
        let repos = repos_from_listing(LISTING, &files()).expect("the 0.60.7 listing parses");
        assert_eq!(repos.len(), 3);
        assert_eq!(repos[1].url, "https://khame.github.io/void-packages/");
        assert_eq!(repos[1].file, "/etc/xbps.d/10-khame.conf");
        assert_eq!(
            repos[0].file, "",
            "nothing defines repo-de in the config dir"
        );
        assert_eq!(repos[2].count, -1, "-1 is no index, not success");
        assert!(!repos[2].signed, "maybe-signed is not verified locally");
    }

    #[test]
    fn an_unreadable_listing_is_empty_and_logged_not_invented() {
        let state = system_state(Ok(parse(MAIN_URL)), "not a listing\n", &[]);
        assert_eq!(state.repos_json, "[]");
        assert_eq!(
            state.system_mirror, "https://repo-de.voidlinux.org/",
            "the config half still answers"
        );
    }

    #[test]
    fn a_comment_that_mentions_a_url_is_not_its_file() {
        let files = vec![(
            String::from("/etc/xbps.d/00-repository-main.conf"),
            String::from("# repository=https://repo-de.voidlinux.org/current\n"),
        )];
        let repos = repos_from_listing(
            "14836 https://repo-de.voidlinux.org/current (RSA signed)\n",
            &files,
        )
        .expect("parses");
        assert_eq!(repos[0].file, "", "a commented line claims nothing");
    }

    #[test]
    fn config_files_are_read_in_path_order_and_only_conf_files() {
        let dir = tempdir("files");
        std::fs::write(dir.join("20-b.conf"), "repository=https://b.example/\n").expect("b");
        std::fs::write(dir.join("10-a.conf"), "repository=https://a.example/\n").expect("a");
        std::fs::write(dir.join("notes.txt"), "not a config\n").expect("txt");
        let dir_text = dir.display().to_string();
        let files = read_config_files(&[&dir_text]);
        let names: Vec<&str> = files
            .iter()
            .map(|(path, _)| {
                Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(path.as_str())
            })
            .collect();
        assert_eq!(names, ["10-a.conf", "20-b.conf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The config a test asks for, from the URL it names.
    fn parse(url: &str) -> SystemConfig {
        let text = format!("repository={url}\n");
        kxbps_core::mirror_config::parse_repository_conf(&text).expect("valid configuration")
    }
}
