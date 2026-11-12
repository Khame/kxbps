//! The engine behind `kxbps-mirror-apply`: validate, write, sync, verify or
//! revert.
//!
//! One helper process does the whole thing under one authorization:
//!
//! 1. **Refuse a bad URL before anything is written.** Only an absolute
//!    `https://host/` that is not already a `/current` repository qualifies;
//!    plain HTTP and junk stop here with exit code 2.
//! 2. **Write** `00-repository-main.conf` with a `.kxbps-bak` backup of the
//!    previous bytes (see [`crate::mirror_config`]). A write failure, including
//!    a symlinked target, exits 3.
//! 3. **Sync** with `xbps-install -S` — no `-y`, and stdin is `/dev/null`, so a
//!    repository that wants a new key gets an error instead of a hang.
//! 4. **Verify** with `xbps-query -L`: the new URL must be listed with a count
//!    of at least zero. `-1` is "no index" and is not success.
//! 5. **Revert** the previous bytes on any failure after the write: exit 4 when
//!    the revert worked, exit 5 when it did not.
//!
//! The commands and every path are injected, so the tests drive the whole
//! pipeline without root and without xbps. The production entry point is
//! [`Cli`] and its [`Cli::run`].
//!
//! ```
//! use kxbps_core::mirror_apply::{Exit, check_url};
//!
//! assert_eq!(Exit::from_code(4), Some(Exit::SyncFailed));
//! assert!(check_url("http://repo-default.voidlinux.org/").is_err());
//! ```

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::mirror_config::{
    Backup, ConfigError, Previous, check_base_url, render_config, revert, snapshot_previous,
    write_config_atomic,
};
use crate::probe::{ProbeError, check_probe_url};
use crate::xbps::parse_repos;

/// The helper's exit code: what the job shows and what the caller maps to a
/// message. Numbers are the documented contract, not an internal detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    /// Written, synced and verified.
    Ok = 0,
    /// The URL was refused before any write: plain HTTP, malformed, or not a
    /// base URL.
    UrlRefused = 2,
    /// The configuration could not be written (including a symlinked target).
    WriteFailed = 3,
    /// `xbps-install -S` failed and the previous file was restored.
    SyncFailed = 4,
    /// `xbps-install -S` failed and the revert failed too.
    SyncAndRevertFailed = 5,
}

impl Exit {
    /// The documented code as a process exit status.
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// The reverse mapping; `None` for a code this helper never returns.
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::Ok),
            2 => Some(Self::UrlRefused),
            3 => Some(Self::WriteFailed),
            4 => Some(Self::SyncFailed),
            5 => Some(Self::SyncAndRevertFailed),
            _ => None,
        }
    }

    /// Stable translation key for the UI; empty on success.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ok => "",
            Self::UrlRefused => "errorMirrorUrlRefused",
            Self::WriteFailed => "errorMirrorWrite",
            Self::SyncFailed => "errorMirrorSync",
            Self::SyncAndRevertFailed => "errorMirrorRevert",
        }
    }

    /// The arguments for [`Self::key`]; the URL for a refusal, nothing for
    /// the rest, whose detail the job attaches from the helper's stderr.
    pub fn args(self, url: &str) -> Vec<String> {
        match self {
            Self::UrlRefused => vec![url.to_owned()],
            _ => Vec::new(),
        }
    }
}

/// A base URL accepted by the helper, normalised to one trailing `/`.
pub fn check_url(url: &str) -> Result<String, String> {
    if let Err(ProbeError::PlainHttp) = check_probe_url(url) {
        return Err(String::from("plain HTTP is not allowed"));
    }
    check_base_url(url).map_err(|error| error.reason_text().to_owned())
}

/// Verifies one URL against the `xbps-query -L` listing. `Err` carries the
/// reason the job shows.
pub type VerifyFn<'a> = dyn FnMut(&str) -> Result<(), String> + 'a;

/// Runs `xbps-install -S`; `Ok(())` when it exited 0. `Err` carries the exit
/// code and the last stderr line.
pub type RunFn<'a> = dyn FnMut() -> Result<(), (i32, String)> + 'a;

/// Everything one Apply touches. All of it is outer-world state, so the tests
/// hand in a temporary directory and closures instead of root and xbps.
pub struct ApplyRequest<'a> {
    pub config_path: PathBuf,
    pub run_install: Box<RunFn<'a>>,
    pub verify: Box<VerifyFn<'a>>,
}

/// What one Apply did, for the report and the job label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyResult {
    pub exit: Exit,
    /// The URL xbps now resolves, `<base>current`.
    pub url: String,
    /// The normalised base that was written; empty when the URL was refused.
    pub base: String,
    /// The backup path, whether or not a backup was needed.
    pub backup: PathBuf,
    /// The last failure reason, in English, for the job's stderr tail.
    pub detail: String,
}

impl ApplyResult {
    /// True only for [`Exit::Ok`].
    pub fn ok(&self) -> bool {
        self.exit == Exit::Ok
    }
}

/// True when `xbps-query -L` lists `url` with a count of at least zero.
///
/// A malformed listing is a failed verification, not a pass: the caller has to
/// learn about a new format, and an unreadable list cannot confirm anything.
pub fn verify_repos(listing: &str, url: &str) -> Result<(), String> {
    let repos = parse_repos(listing).map_err(|error| error.to_string())?;
    match repos.iter().find(|repo| repo.url == url) {
        Some(repo) if repo.available() => Ok(()),
        Some(repo) => Err(format!(
            "{url} is listed with count {}, which means no index",
            repo.count
        )),
        None => Err(format!("{url} is not listed by xbps-query -L")),
    }
}

/// Applies a mirror: validate, write, sync, verify, revert on failure.
///
/// The URL is checked before any path is touched, so a refusal never leaves a
/// backup or a temp file behind.
pub fn apply(mut request: ApplyRequest<'_>, url: &str) -> ApplyResult {
    let backup = Backup::path_for(&request.config_path);
    let base = match check_url(url) {
        Ok(base) => base,
        Err(detail) => {
            return ApplyResult {
                exit: Exit::UrlRefused,
                url: url.to_owned(),
                base: String::new(),
                backup,
                detail,
            };
        }
    };
    let current = format!("{base}current");
    let previous: Previous = match snapshot_previous(&request.config_path) {
        Ok(previous) => previous,
        Err(error) => {
            return ApplyResult {
                exit: Exit::WriteFailed,
                url: current,
                base,
                backup,
                detail: describe(&error),
            };
        }
    };
    let content = render_config(&base);
    if let Err(error) = write_config_atomic(&request.config_path, content.as_bytes(), Backup::Keep)
    {
        // A write that failed halfway leaves nothing behind; a failed backup
        // has already been reported, and the target is still the previous file.
        return ApplyResult {
            exit: Exit::WriteFailed,
            url: current,
            base,
            backup,
            detail: describe(&error),
        };
    }

    if let Err((code, stderr)) = (request.run_install)() {
        let detail = format!("xbps-install -S exited {code}: {stderr}");
        return finish(previous, current, base, backup, detail);
    }
    if let Err(detail) = (request.verify)(&format!("{base}current")) {
        return finish(previous, current, base, backup, detail);
    }
    ApplyResult {
        exit: Exit::Ok,
        url: current,
        base,
        backup,
        detail: String::new(),
    }
}

/// The failure path after the write: revert, then report 4 or 5.
fn finish(
    previous: Previous,
    url: String,
    base: String,
    backup: PathBuf,
    detail: String,
) -> ApplyResult {
    let (exit, detail) = match revert(&previous) {
        Ok(()) => (Exit::SyncFailed, detail),
        Err(error) => (
            Exit::SyncAndRevertFailed,
            format!("{detail}; revert failed: {}", describe(&error)),
        ),
    };
    ApplyResult {
        exit,
        url,
        base,
        backup,
        detail,
    }
}

/// One English sentence describing a configuration error and its path.
pub fn describe(error: &ConfigError) -> String {
    match error {
        ConfigError::Parse { line, .. } => {
            format!("line {line:?}: {}", error.reason_text())
        }
        ConfigError::Read { path, reason } | ConfigError::Write { path, reason } => {
            format!("{}: {reason}", path.display())
        }
        ConfigError::Missing { path } | ConfigError::Symlink { path } => {
            format!("{}: {error}", path.display())
        }
    }
}

/// The argv of the sync command: `xbps-install -S`.
///
/// Deliberately no `-y`: it is a yes-to-everything flag and has nothing to
/// decide here; a missing key still fails on the `/dev/null` stdin.
pub fn sync_argv() -> Vec<String> {
    vec![String::from(XBPS_INSTALL), String::from("-S")]
}

/// The argv of the verification command: `xbps-query -L`.
pub fn verify_argv() -> Vec<String> {
    vec![String::from(XBPS_QUERY), String::from("-L")]
}

/// The absolute path of `xbps-install`, as `transaction` names it too.
pub const XBPS_INSTALL: &str = "/usr/bin/xbps-install";

/// The absolute path of `xbps-query`.
pub const XBPS_QUERY: &str = "/usr/bin/xbps-query";

/// The production entry point: one base URL argument, the real commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    pub config_path: PathBuf,
    pub url: Option<String>,
}

impl Cli {
    /// Parses the helper's arguments: exactly one base URL, nothing else.
    ///
    /// `--help` and no argument yield `url: None`, which [`Self::usage`]
    /// explains; a second argument is the same as `None` so the helper never
    /// guesses.
    pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Self {
        let mut values = args.into_iter();
        let url = values
            .next()
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.starts_with('-'))
            .filter(|_| values.next().is_none());
        Self {
            config_path: PathBuf::from(crate::mirror_config::SYSTEM_CONFIG_PATH),
            url,
        }
    }

    /// A short English usage line, printed on a refused argument list.
    pub fn usage(program: &str) -> String {
        format!("usage: {program} <https://mirror.example/>")
    }

    /// Runs the real pipeline. Only root can write the configuration, and
    /// polkit is what makes this process root.
    pub fn run(self) -> ApplyResult {
        let Some(url) = self.url else {
            return ApplyResult {
                exit: Exit::UrlRefused,
                url: String::new(),
                base: String::new(),
                backup: Backup::path_for(&self.config_path),
                detail: String::from("expected exactly one base URL"),
            };
        };
        let config_path = self.config_path;
        apply(
            ApplyRequest {
                config_path,
                run_install: Box::new(run_sync),
                verify: Box::new(run_verify),
            },
            &url,
        )
    }
}

/// Runs `xbps-install -S` with stdin on `/dev/null` and the output captured.
///
/// No shell, no `-y`, no timeout of our own: xbps is the one that knows how
/// long a repository may take, and a key prompt gets EOF instead of a hang.
fn run_sync() -> Result<(), (i32, String)> {
    let output = Command::new(XBPS_INSTALL)
        .arg("-S")
        .stdin(Stdio::null())
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => return Err((-1, error.to_string())),
    };
    if output.status.success() {
        Ok(())
    } else {
        let code = output.status.code().unwrap_or(-1);
        Err((code, last_line(&String::from_utf8_lossy(&output.stderr))))
    }
}

/// Runs `xbps-query -L` and checks the new URL is listed with an index.
fn run_verify(url: &str) -> Result<(), String> {
    let output = Command::new(XBPS_QUERY)
        .arg("-L")
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => {
            verify_repos(&String::from_utf8_lossy(&output.stdout), url)
        }
        Ok(output) => Err(format!(
            "xbps-query -L exited {}: {}",
            output.status.code().unwrap_or(-1),
            last_line(&String::from_utf8_lossy(&output.stderr))
        )),
        Err(error) => Err(error.to_string()),
    }
}

/// The last non-empty line of a command's stderr, for a job label.
pub fn last_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Where the helper's binary lives; polkit authorizes this exact path.
pub const HELPER_PATH: &str = "/usr/libexec/kxbps-mirror-apply";

/// The config path a test or the helper writes, for callers that only have the
/// file name; kept here so the binary stays a thin `main`.
pub fn config_path() -> &'static Path {
    Path::new(crate::mirror_config::SYSTEM_CONFIG_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const MAIN: &str = "repository=https://repo-de.voidlinux.org/current\n";
    const GOOD: &str = "https://repo-default.voidlinux.org/";

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kxbps-mirror-apply-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn listing(url: &str, count: i64) -> String {
        format!("{count} {url} (RSA signed)\n")
    }

    /// A happy-path request: the sync succeeds and `-L` lists the new URL.
    fn request<'a>(config_path: &Path) -> ApplyRequest<'a> {
        ApplyRequest {
            config_path: config_path.to_path_buf(),
            run_install: Box::new(|| Ok(())),
            verify: Box::new(|url| {
                if url == "https://repo-default.voidlinux.org/current" {
                    Ok(())
                } else {
                    Err(format!("{url} is not listed by xbps-query -L"))
                }
            }),
        }
    }

    /// Turns the target into a symlink once, so the revert cannot restore it.
    fn break_revert(path: &Path) {
        fs::remove_file(path).expect("remove the written file");
        symlink("/nonexistent", path).expect("symlink in the way");
    }

    #[test]
    fn the_exit_codes_are_2_3_4_5() {
        assert_eq!(Exit::Ok.code(), 0);
        assert_eq!(Exit::UrlRefused.code(), 2);
        assert_eq!(Exit::WriteFailed.code(), 3);
        assert_eq!(Exit::SyncFailed.code(), 4);
        assert_eq!(Exit::SyncAndRevertFailed.code(), 5);
        for exit in [
            Exit::Ok,
            Exit::UrlRefused,
            Exit::WriteFailed,
            Exit::SyncFailed,
            Exit::SyncAndRevertFailed,
        ] {
            assert_eq!(Exit::from_code(exit.code()), Some(exit));
        }
        assert_eq!(Exit::from_code(1), None);
        assert_eq!(Exit::from_code(126), None);
        assert_eq!(Exit::UrlRefused.key(), "errorMirrorUrlRefused");
        assert_eq!(Exit::SyncFailed.key(), "errorMirrorSync");
        assert_eq!(
            Exit::UrlRefused.args("https://x/"),
            [String::from("https://x/")]
        );
        assert!(Exit::WriteFailed.args("https://x/").is_empty());
    }

    #[test]
    fn a_plain_http_url_is_refused_before_any_write() {
        let dir = tempdir("refuse-http");
        let path = dir.join("00-repository-main.conf");
        let mut request = request(&path);
        request.run_install = Box::new(|| panic!("the sync must not run"));
        request.verify = Box::new(|_| panic!("the verification must not run"));

        let result = apply(request, "http://repo-default.voidlinux.org/");
        assert_eq!(result.exit, Exit::UrlRefused);
        assert_eq!(result.detail, String::from("plain HTTP is not allowed"));
        assert!(!path.exists(), "a refused URL writes nothing");
        assert!(!result.backup.exists(), "a refused URL backs nothing up");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_current_url_and_junk_are_refused() {
        let dir = tempdir("refuse-more");
        let path = dir.join("00-repository-main.conf");
        for url in [
            "https://repo-de.voidlinux.org/current",
            "https://repo-de.voidlinux.org/current/",
            "https://repo-de.voidlinux.org/current/multilib",
            "https://repo-de.voidlinux.org/current/multilib/nonfree",
            "mirror.example",
            "ftp://mirror.example/",
            "https://",
            "https://user@mirror.example/",
        ] {
            let result = apply(request(&path), url);
            assert_eq!(result.exit, Exit::UrlRefused, "{url}");
            assert!(!path.exists(), "{url} wrote something");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_good_url_is_written_synced_and_verified() {
        let dir = tempdir("ok");
        let path = dir.join("00-repository-main.conf");
        let result = apply(request(&path), GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(result.base, GOOD);
        assert_eq!(
            result.url, "https://repo-default.voidlinux.org/current",
            "the reported URL is what xbps resolves"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            render_config(GOOD)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_backs_up_an_existing_file_and_writes_a_0644_one() {
        let dir = tempdir("backup");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let result = apply(request(&path), GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(fs::read_to_string(&result.backup).expect("backup"), MAIN);
        assert_eq!(
            fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
            0o644
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_without_an_existing_file_writes_one_and_removes_it_on_revert() {
        let dir = tempdir("no-existing");
        let path = dir.join("00-repository-main.conf");
        let mut request = request(&path);
        request.run_install = Box::new(|| Err((19, String::from("broken mirror"))));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::SyncFailed);
        assert!(
            !path.exists(),
            "the file did not exist before the apply, so the revert removes it"
        );
        assert!(result.detail.contains("19"), "{}", result.detail);
        assert!(result.detail.contains("broken mirror"), "{}", result.detail);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_sync_restores_the_previous_bytes() {
        let dir = tempdir("sync-fail");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let before = fs::read(&path).expect("read");
        let mut request = request(&path);
        request.run_install = Box::new(|| Err((19, String::from("repo-default unreachable"))));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::SyncFailed, "{}", result.detail);
        assert_eq!(fs::read(&path).expect("read"), before);
        assert_eq!(
            fs::read_to_string(&result.backup).expect("backup"),
            MAIN,
            "the backup keeps the previous bytes even after a revert"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_listing_without_the_new_url_fails_and_reverts() {
        let dir = tempdir("verify-stale");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let mut request = request(&path);
        request.verify = Box::new(|url| Err(format!("{url} is not listed by xbps-query -L")));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::SyncFailed);
        assert!(result.detail.contains("not listed"), "{}", result.detail);
        assert_eq!(fs::read_to_string(&path).expect("read"), MAIN);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_sync_with_a_failing_revert_reports_both() {
        let dir = tempdir("revert-fail");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let mut request = request(&path);
        let target = path.clone();
        request.run_install = Box::new(move || {
            break_revert(&target);
            Err((19, String::from("sync broke")))
        });

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::SyncAndRevertFailed, "{}", result.detail);
        assert_eq!(result.exit.code(), 5);
        assert!(result.detail.contains("sync broke"), "{}", result.detail);
        assert!(result.detail.contains("revert failed"), "{}", result.detail);
        assert_eq!(
            fs::read_to_string(&result.backup).expect("backup"),
            MAIN,
            "the backup still holds the original bytes"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_config_is_refused_with_a_write_error() {
        let dir = tempdir("symlink");
        let real = dir.join("real.conf");
        fs::write(&real, MAIN).expect("fixture");
        let link = dir.join("00-repository-main.conf");
        symlink(&real, &link).expect("symlink");
        let mut request = request(&link);
        request.run_install = Box::new(|| panic!("the sync must not run"));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::WriteFailed);
        assert_eq!(result.exit.code(), 3);
        assert_eq!(fs::read_to_string(&real).expect("read"), MAIN);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_repos_accepts_the_new_url_and_rejects_a_stale_one() {
        let text = listing("https://repo-default.voidlinux.org/current", 42);
        assert_eq!(
            verify_repos(&text, "https://repo-default.voidlinux.org/current"),
            Ok(())
        );
        assert!(verify_repos(&text, "https://repo-de.voidlinux.org/current").is_err());
    }

    #[test]
    fn verify_repos_rejects_a_minus_one_index() {
        let text = listing("https://repo-default.voidlinux.org/current", -1);
        let error = verify_repos(&text, "https://repo-default.voidlinux.org/current")
            .expect_err("-1 is no index, not success");
        assert!(error.contains("-1"), "{error}");
    }

    #[test]
    fn verify_repos_fails_on_output_it_cannot_read() {
        assert!(verify_repos("not a listing\n", "https://a/current").is_err());
    }

    #[test]
    fn verify_repos_accepts_a_maybe_signed_repository() {
        let text =
            String::from("5 https://repo-default.voidlinux.org/current (RSA maybe-signed)\n");
        assert_eq!(
            verify_repos(&text, "https://repo-default.voidlinux.org/current"),
            Ok(())
        );
    }

    #[test]
    fn the_sync_argv_carries_no_yes_flag() {
        let argv = sync_argv();
        assert_eq!(argv, ["/usr/bin/xbps-install", "-S"]);
        assert!(
            !argv.iter().any(|arg| arg == "-y" || arg == "--yes"),
            "`-y` would answer prompts nobody asked: {argv:?}"
        );
        assert_eq!(verify_argv(), ["/usr/bin/xbps-query", "-L"]);
    }

    #[test]
    fn check_url_normalises_the_base_and_refuses_the_rest() {
        assert_eq!(
            check_url("https://repo-default.voidlinux.org").as_deref(),
            Ok(GOOD)
        );
        assert!(check_url("https://repo-default.voidlinux.org/current").is_err());
        assert!(check_url("http://repo-default.voidlinux.org/").is_err());
        assert!(check_url("").is_err());
    }

    #[test]
    fn the_cli_takes_exactly_one_url() {
        let cli = Cli::parse([OsString::from("https://repo-default.voidlinux.org/")]);
        assert_eq!(cli.url.as_deref(), Some(GOOD));
        assert_eq!(
            cli.config_path,
            PathBuf::from("/etc/xbps.d/00-repository-main.conf")
        );
        assert!(Cli::parse(std::iter::empty::<OsString>()).url.is_none());
        assert!(
            Cli::parse([OsString::from("https://a/"), OsString::from("https://b/")])
                .url
                .is_none(),
            "two URLs are not guessed between"
        );
        assert!(Cli::parse([OsString::from("--help")]).url.is_none());
        assert_eq!(
            Cli::usage("kxbps-mirror-apply"),
            "usage: kxbps-mirror-apply <https://mirror.example/>"
        );
    }

    #[test]
    fn a_refused_argument_list_ends_on_exit_code_2() {
        let normal = Cli::parse([OsString::from("xbps-install"), OsString::from("-S")]);
        assert!(normal.url.is_none(), "two arguments are never guessed");
        let refused = normal.run();
        assert_eq!(refused.exit, Exit::UrlRefused);
        assert_eq!(refused.exit.code(), 2, "the binary exits with this code");
        assert!(
            refused.detail.contains("expected exactly one base URL"),
            "{}",
            refused.detail
        );
        assert_eq!(refused.url, "", "nothing was attempted");
    }

    #[test]
    fn the_last_line_is_what_the_label_shows() {
        assert_eq!(last_line("a\nb\n\n"), "b");
        assert_eq!(last_line("only"), "only");
        assert_eq!(last_line(""), "");
    }

    #[test]
    fn a_base_from_the_mirror_catalog_is_accepted_and_renders_the_same_current() {
        for info in crate::mirrors::MIRRORS {
            if info.url.starts_with("http://") {
                assert_eq!(
                    apply_http_refusal(info.url),
                    Exit::UrlRefused,
                    "{}",
                    info.url
                );
                continue;
            }
            let base = check_url(info.url).unwrap_or_else(|error| panic!("{}: {error}", info.url));
            assert_eq!(base, info.url, "{} keeps its exact spelling", info.url);
            let text = render_config(&base);
            assert!(
                text.ends_with(&format!("repository={}current\n", info.url)),
                "{text}"
            );
        }
    }

    fn apply_http_refusal(url: &str) -> Exit {
        let dir = tempdir("catalog-http");
        let result = apply(request(&dir.join("00-repository-main.conf")), url);
        let _ = fs::remove_dir_all(&dir);
        result.exit
    }

    #[test]
    fn the_helper_path_is_the_one_the_policy_authorizes() {
        assert_eq!(HELPER_PATH, "/usr/libexec/kxbps-mirror-apply");
        assert_eq!(
            config_path(),
            Path::new("/etc/xbps.d/00-repository-main.conf")
        );
    }

    #[test]
    fn describe_names_the_line_of_a_parse_error_and_the_path_of_an_io_error() {
        let parse = ConfigError::Parse {
            line: String::from("repository=http://x/"),
            reason: crate::mirror_config::ParseReason::PlainHttp,
        };
        assert!(
            describe(&parse).contains("plain http"),
            "{}",
            describe(&parse)
        );
        let missing = ConfigError::Missing {
            path: PathBuf::from("/etc/xbps.d/00-repository-main.conf"),
        };
        assert!(describe(&missing).contains("/etc/xbps.d/00-repository-main.conf"));
    }
}
