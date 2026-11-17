// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The engine behind `kxbps-mirror-apply`: validate, write, verify or revert.
//!
//! One helper process does the whole thing under one authorization:
//!
//! 1. **Refuse a bad URL before anything is written.** Only an absolute
//!    `https://host/` that is not already a `/current` repository qualifies;
//!    plain HTTP and junk stop here with exit code 2.
//! 2. **Keep the repository tail.** The tail after `current` (`musl` on a musl
//!    system) is read from the file being replaced, or from the packaged
//!    default when the file is missing, and written back with the new host.
//! 3. **Write** `00-repository-main.conf` with a `.kxbps-bak` backup of the
//!    previous bytes (see [`crate::mirror_config`]). A write failure, including
//!    a symlinked target, exits 3.
//! 4. **Verify** with `xbps-query -L`: the new URL must be listed. The count is
//!    not checked: a mirror that was never synced has no local index yet, and
//!    that is exactly the state a URL change leaves behind. No network command
//!    runs here.
//! 5. **Revert** the previous bytes on a failed verification: exit 4 when the
//!    revert worked, exit 5 when it did not.
//!
//! A verified configuration is then **synced** with `xbps-install -S`, because
//! `xbps-query` skips a repository that has no index and the package list would
//! stay short without a word. A failed sync exits 6 and keeps the file.
//!
//! The commands and every path are injected, so the tests drive the whole
//! pipeline without root and without xbps. The production entry point is
//! [`Cli`] and its [`Cli::run`].
//!
//! ```
//! use kxbps_core::mirror_apply::{Exit, check_url};
//!
//! assert_eq!(Exit::from_code(4), Some(Exit::VerifyFailed));
//! assert!(check_url("http://repo-default.voidlinux.org/").is_err());
//! ```

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::mirror_config::{
    Backup, ConfigError, Previous, check_base_url, current_url, read_config, render_config, revert,
    snapshot_previous, write_config_atomic,
};
use crate::probe::{ProbeError, check_probe_url};
use crate::xbps::parse_repos;

/// The helper's exit code: what the job shows and what the caller maps to a
/// message. Numbers are the documented contract, not an internal detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    /// Written and verified.
    Ok = 0,
    /// The URL was refused before any write: plain HTTP, malformed, or not a
    /// base URL.
    UrlRefused = 2,
    /// The configuration could not be written (including a symlinked target).
    WriteFailed = 3,
    /// `xbps-query -L` does not list the new URL, and the previous file was
    /// restored.
    VerifyFailed = 4,
    /// The verification failed and the revert failed too.
    VerifyAndRevertFailed = 5,
    /// The configuration is written and verified, but `xbps-install -S` failed.
    /// The mirror is set; the index for it is not, so every package list stays
    /// incomplete until the sync succeeds. The file is kept: a mirror that is
    /// merely unreachable for a minute is not a wrong mirror.
    SyncFailed = 6,
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
            4 => Some(Self::VerifyFailed),
            5 => Some(Self::VerifyAndRevertFailed),
            6 => Some(Self::SyncFailed),
            _ => None,
        }
    }

    /// Stable translation key for the UI; empty on success.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ok => "",
            Self::UrlRefused => "errorMirrorUrlRefused",
            Self::WriteFailed => "errorMirrorWrite",
            Self::VerifyFailed => "errorMirrorVerify",
            Self::VerifyAndRevertFailed => "errorMirrorRevert",
            Self::SyncFailed => "mirrorSyncFailed",
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

/// Fetches the indexes of the configured repositories. `Err` carries the
/// reason the job shows.
///
/// A repository whose index is missing is skipped by `xbps-query` without a
/// word, so a mirror that was written but never synced silently shrinks every
/// package list. The sync belongs to the same authorization as the write: one
/// password sets the mirror *and* makes it readable.
pub type SyncFn<'a> = dyn FnMut() -> Result<(), String> + 'a;

/// Everything one Apply touches. All of it is outer-world state, so the tests
/// hand in a temporary directory and a closure instead of root and xbps.
pub struct ApplyRequest<'a> {
    pub config_path: PathBuf,
    /// The packaged default, read only when `config_path` is missing or
    /// unreadable: it names the repository tail of this system (`musl`).
    pub system_default_path: PathBuf,
    pub verify: Box<VerifyFn<'a>>,
    pub sync: Box<SyncFn<'a>>,
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

/// True when `xbps-query -L` lists `url`, whatever index count it carries.
///
/// The count is deliberately ignored: a repository that was just written has no
/// local index yet, so `-1` is the expected state after a URL change, not a
/// failure. A malformed listing is a failed verification, not a pass: the caller
/// has to learn about a new format, and an unreadable list cannot confirm
/// anything.
pub fn verify_repos(listing: &str, url: &str) -> Result<(), String> {
    let repos = parse_repos(listing).map_err(|error| error.to_string())?;
    match repos.iter().find(|repo| repo.url == url) {
        Some(_) => Ok(()),
        None => Err(format!("{url} is not listed by xbps-query -L")),
    }
}

/// Applies a mirror: validate, write, verify, revert on failure.
///
/// The URL is checked before any path is touched, so a refusal never leaves a
/// backup or a temp file behind. The repository tail comes from the file being
/// replaced, or from the packaged default when that file is missing.
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
    let tail = repository_tail(&request.config_path, &request.system_default_path);
    let current = current_url(&base, &tail);
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
    let content = render_config(&base, &tail);
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

    if let Err(detail) = (request.verify)(&current) {
        return finish(previous, current, base, backup, detail);
    }

    // The file is correct at this point, so a failed sync keeps it and only
    // reports that the index is still missing.
    match (request.sync)() {
        Ok(()) => ApplyResult {
            exit: Exit::Ok,
            url: current,
            base,
            backup,
            detail: String::new(),
        },
        Err(detail) => ApplyResult {
            exit: Exit::SyncFailed,
            url: current,
            base,
            backup,
            detail,
        },
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
        Ok(()) => (Exit::VerifyFailed, detail),
        Err(error) => (
            Exit::VerifyAndRevertFailed,
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

/// The argv of the verification command: `xbps-query -L`.
pub fn verify_argv() -> Vec<String> {
    vec![String::from(XBPS_QUERY), String::from("-L")]
}

/// The argv of the sync command: `xbps-install -S`.
pub fn sync_argv() -> Vec<String> {
    vec![String::from(XBPS_INSTALL), String::from("-S")]
}

/// The absolute path of `xbps-query`.
pub const XBPS_QUERY: &str = "/usr/bin/xbps-query";

/// The absolute path of `xbps-install`.
pub const XBPS_INSTALL: &str = "/usr/bin/xbps-install";

/// The production entry point: one base URL argument, the real commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    pub config_path: PathBuf,
    pub system_default_path: PathBuf,
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
            system_default_path: PathBuf::from(SYSTEM_DEFAULT_CONFIG_PATH),
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
        apply(
            ApplyRequest {
                config_path: self.config_path,
                system_default_path: self.system_default_path,
                verify: Box::new(run_verify),
                sync: Box::new(sync_indexes),
            },
            &url,
        )
    }
}

/// Runs `xbps-query -L` and checks the new URL is listed.
fn run_verify(url: &str) -> Result<(), String> {
    let output = Command::new(command_path(XBPS_QUERY))
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

/// Runs `xbps-install -S`; `Err` is the one line the job shows.
///
/// A repository without an index is skipped by `xbps-query` without a word, so
/// every path that writes a repository configuration calls this in the same
/// authorization: one password sets the repository *and* makes it readable.
pub fn sync_indexes() -> Result<(), String> {
    let output = Command::new(command_path(XBPS_INSTALL))
        .arg("-S")
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "xbps-install -S exited {}: {}",
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
///
/// [`crate::transaction`] owns the string because it builds the argv; this
/// alias keeps the helper binary's own name for it.
pub use crate::transaction::MIRROR_APPLY_PATH as HELPER_PATH;

/// The config path a test or the helper writes, for callers that only have the
/// file name; kept here so the binary stays a thin `main`.
pub fn config_path() -> &'static Path {
    Path::new(crate::mirror_config::SYSTEM_CONFIG_PATH)
}

/// The packaged default repository file, read for the tail only when the
/// administrator's file is missing or unreadable.
pub const SYSTEM_DEFAULT_CONFIG_PATH: &str = "/usr/share/xbps.d/00-repository-main.conf";

/// The repository tail this system uses.
///
/// The file being replaced is the truth; the packaged default is the fallback
/// for a system that never had a local override, and it names `musl` on a musl
/// installation. An unreadable pair means glibc (`""`), the historical shape.
pub fn repository_tail(config_path: &Path, default_path: &Path) -> String {
    read_config(config_path)
        .or_else(|_| read_config(default_path))
        .map(|config| config.tail)
        .unwrap_or_default()
}

/// The production path of an xbps command.
///
/// `KXBPS_XBPS_BIN_DIR` exists so a test can point both commands at the same
/// fake script: the helper is root-only, and a test that needs the whole
/// pipeline must not shell out to a real xbps. Unset, the documented absolute
/// paths are used, which is what polkit authorizes in a real run.
pub fn command_path(name: &str) -> String {
    match std::env::var_os("KXBPS_XBPS_BIN_DIR") {
        Some(dir) => {
            let file = Path::new(name)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new(name));
            Path::new(&dir).join(file).to_string_lossy().into_owned()
        }
        None => name.to_owned(),
    }
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

    /// A happy-path request: `-L` lists the new URL.
    fn request<'a>(config_path: &Path) -> ApplyRequest<'a> {
        ApplyRequest {
            config_path: config_path.to_path_buf(),
            system_default_path: default_beside(config_path),
            verify: Box::new(|url| {
                if url == "https://repo-default.voidlinux.org/current" {
                    Ok(())
                } else {
                    Err(format!("{url} is not listed by xbps-query -L"))
                }
            }),
            sync: Box::new(|| Ok(())),
        }
    }

    /// A packaged default that does not exist, so the fixture alone decides.
    fn default_beside(config_path: &Path) -> PathBuf {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("00-repository-main.packaged.conf")
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
        assert_eq!(Exit::VerifyFailed.code(), 4);
        assert_eq!(Exit::VerifyAndRevertFailed.code(), 5);
        for exit in [
            Exit::Ok,
            Exit::UrlRefused,
            Exit::WriteFailed,
            Exit::VerifyFailed,
            Exit::VerifyAndRevertFailed,
        ] {
            assert_eq!(Exit::from_code(exit.code()), Some(exit));
        }
        assert_eq!(Exit::from_code(1), None);
        assert_eq!(Exit::from_code(126), None);
        assert_eq!(Exit::SyncFailed.code(), 6);
        assert_eq!(Exit::from_code(6), Some(Exit::SyncFailed));
        assert_eq!(Exit::SyncFailed.key(), "mirrorSyncFailed");
        assert_eq!(Exit::UrlRefused.key(), "errorMirrorUrlRefused");
        assert_eq!(Exit::VerifyFailed.key(), "errorMirrorVerify");
        assert_eq!(
            Exit::UrlRefused.args("https://x/"),
            [String::from("https://x/")]
        );
        assert!(Exit::WriteFailed.args("https://x/").is_empty());
    }

    #[test]
    fn a_failed_sync_keeps_the_file_and_reports_six() {
        let dir = tempdir("sync-fails");
        let path = dir.join("00-repository-main.conf");
        let mut request = request(&path);
        request.sync = Box::new(|| Err(String::from("network is down")));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::SyncFailed);
        assert_eq!(result.detail, "network is down");
        assert!(
            fs::read_to_string(&path)
                .expect("the written file stays")
                .contains("repository=https://repo-default.voidlinux.org/current"),
            "a failed sync must not undo a correct configuration"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sync_does_not_run_when_the_verification_failed() {
        let dir = tempdir("no-sync");
        let path = dir.join("00-repository-main.conf");
        let mut request = request(&path);
        request.verify = Box::new(|_| Err(String::from("not listed")));
        request.sync = Box::new(|| panic!("a rejected configuration must not be synced"));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::VerifyFailed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sync_argv_is_xbps_install_dash_s() {
        assert_eq!(sync_argv(), ["/usr/bin/xbps-install", "-S"]);
    }

    #[test]
    fn a_plain_http_url_is_refused_before_any_write() {
        let dir = tempdir("refuse-http");
        let path = dir.join("00-repository-main.conf");
        let mut request = request(&path);
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
            render_config(GOOD, "")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn applying_to_a_musl_config_keeps_the_musl_tail() {
        let dir = tempdir("musl");
        let path = dir.join("00-repository-main.conf");
        fs::write(
            &path,
            "repository=https://repo-de.voidlinux.org/current/musl\n",
        )
        .expect("fixture");
        let mut request = request(&path);
        request.verify = Box::new(|url| {
            assert_eq!(url, "https://repo-default.voidlinux.org/current/musl");
            Ok(())
        });

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(
            result.url, "https://repo-default.voidlinux.org/current/musl",
            "the reported URL carries the tail xbps resolves"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            render_config(GOOD, "musl")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_config_takes_the_tail_from_the_packaged_default() {
        let dir = tempdir("musl-default");
        let path = dir.join("00-repository-main.conf");
        let default = default_beside(&path);
        fs::write(
            &default,
            "repository=https://repo-default.voidlinux.org/current/musl\n",
        )
        .expect("packaged default");
        let mut request = request(&path);
        request.system_default_path = default;
        request.verify = Box::new(|url| {
            assert_eq!(url, "https://repo-default.voidlinux.org/current/musl");
            Ok(())
        });

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            render_config(GOOD, "musl")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_config_falls_back_to_the_packaged_tail() {
        let dir = tempdir("musl-broken");
        let path = dir.join("00-repository-main.conf");
        let default = default_beside(&path);
        fs::write(&path, "repository=http://broken.example/\n").expect("fixture");
        fs::write(
            &default,
            "repository=https://repo-default.voidlinux.org/current/musl\n",
        )
        .expect("packaged default");
        let mut request = request(&path);
        request.system_default_path = default;
        request.verify = Box::new(|url| {
            assert_eq!(url, "https://repo-default.voidlinux.org/current/musl");
            Ok(())
        });

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            render_config(GOOD, "musl")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_any_tail_source_the_written_url_is_glibc() {
        let dir = tempdir("no-tail-source");
        let path = dir.join("00-repository-main.conf");
        let request = request(&path);
        assert!(!default_beside(&path).exists());

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::Ok, "{}", result.detail);
        assert_eq!(result.url, "https://repo-default.voidlinux.org/current");
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            render_config(GOOD, "")
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
        request.verify = Box::new(|_| Err(String::from("broken mirror")));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::VerifyFailed);
        assert!(
            !path.exists(),
            "the file did not exist before the apply, so the revert removes it"
        );
        assert!(result.detail.contains("broken mirror"), "{}", result.detail);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_verify_restores_the_previous_bytes() {
        let dir = tempdir("verify-fail");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let before = fs::read(&path).expect("read");
        let mut request = request(&path);
        request.verify = Box::new(|_| Err(String::from("repo-default unreachable")));

        let result = apply(request, GOOD);
        assert_eq!(result.exit, Exit::VerifyFailed, "{}", result.detail);
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
        assert_eq!(result.exit, Exit::VerifyFailed);
        assert!(result.detail.contains("not listed"), "{}", result.detail);
        assert_eq!(fs::read_to_string(&path).expect("read"), MAIN);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_verify_with_a_failing_revert_reports_both() {
        let dir = tempdir("revert-fail");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("fixture");
        let mut request = request(&path);
        // The verification fails, and it leaves a symlink in the target's place
        // so the revert cannot put the previous bytes back.
        let target = path.clone();
        request.verify = Box::new(move |_| {
            break_revert(&target);
            Err(String::from("verify broke"))
        });

        let result = apply(request, GOOD);
        assert_eq!(
            result.exit,
            Exit::VerifyAndRevertFailed,
            "{}",
            result.detail
        );
        assert_eq!(result.exit.code(), 5);
        assert!(result.detail.contains("verify broke"), "{}", result.detail);
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
        let request = request(&link);

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
    fn a_repo_listed_without_an_index_still_verifies() {
        let text = listing("https://repo-default.voidlinux.org/current", -1);
        assert_eq!(
            verify_repos(&text, "https://repo-default.voidlinux.org/current"),
            Ok(())
        );
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
    fn the_verify_argv_is_xbps_query_l() {
        let argv = verify_argv();
        assert_eq!(argv, ["/usr/bin/xbps-query", "-L"]);
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
        let normal = Cli::parse([
            OsString::from("https://a.example/"),
            OsString::from("https://b.example/"),
        ]);
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

    /// A CLI over a private config path, so `run` never touches `/etc`.
    fn cli_over(config_path: &Path, url: Option<&str>) -> Cli {
        Cli {
            config_path: config_path.to_path_buf(),
            system_default_path: default_beside(config_path),
            url: url.map(str::to_owned),
        }
    }

    #[test]
    fn the_cli_writes_and_verifies_with_the_fake_commands() {
        let dir = tempdir("cli-ok");
        let path = dir.join("00-repository-main.conf");
        let listing = format!("7 {GOOD}current (RSA signed)");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  -L) echo '{listing}' ;;\n  -S) exit 0 ;;\n  *) exit 9 ;;\nesac\n"
        );
        let bin = write_fake_xbps(&dir, &script);

        let result = with_fake_xbps(&bin, || cli_over(&path, Some(GOOD)).run());
        assert_eq!(
            result.exit,
            Exit::Ok,
            "{} | bin dir {}",
            result.detail,
            bin.display()
        );
        assert_eq!(result.base, GOOD);
        assert_eq!(
            fs::read_to_string(&path).expect("written"),
            render_config(GOOD, "")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_sync_through_the_cli_ends_on_six_and_keeps_the_config() {
        let dir = tempdir("cli-sync");
        let path = dir.join("00-repository-main.conf");
        let listing = format!("7 {GOOD}current (RSA signed)");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  -L) echo '{listing}' ;;\n  -S) echo 'index fetch failed' >&2; exit 1 ;;\n  *) exit 9 ;;\nesac\n"
        );
        let bin = write_fake_xbps(&dir, &script);

        let result = with_fake_xbps(&bin, || cli_over(&path, Some(GOOD)).run());
        assert_eq!(result.exit, Exit::SyncFailed);
        assert!(
            result.detail.contains("index fetch failed"),
            "{}",
            result.detail
        );
        assert!(path.exists(), "the mirror file stays written");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_verify_ends_on_a_documented_code_and_leaves_no_config() {
        // The fake fails the way a real `xbps-query -L` does when the database
        // is unreadable: a non-zero exit and one stderr line.
        let dir = tempdir("cli-verify-fail");
        let path = dir.join("00-repository-main.conf");
        let script = "#!/bin/sh\ncase \"$1\" in\n  -L) echo 'xbps-query: database locked' >&2; exit 1 ;;\n  *) exit 9 ;;\nesac\n";
        let bin = write_fake_xbps(&dir, script);

        let result = with_fake_xbps(&bin, || cli_over(&path, Some(GOOD)).run());
        assert_eq!(
            result.exit,
            Exit::VerifyFailed,
            "a verification failure is code 4, not a panic: {}",
            result.detail
        );
        assert!(
            result.detail.contains("database locked"),
            "the helper keeps xbps' own sentence: {}",
            result.detail
        );
        assert!(!path.exists(), "the failed config was reverted");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Writes one executable script under both xbps names, so either command
    /// reaches it; `body` is the whole script.
    fn write_fake_xbps(dir: &Path, body: &str) -> PathBuf {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).expect("bin dir");
        for name in ["xbps-install", "xbps-query"] {
            let path = bin.join(name);
            fs::write(&path, body).expect("script");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable");
        }
        bin
    }

    /// The lock every test that sets `KXBPS_XBPS_BIN_DIR` holds.
    static BIN_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Runs `body` with both xbps commands answered by `bin/fake-xbps`.
    fn with_fake_xbps<T>(bin: &Path, body: impl FnOnce() -> T) -> T {
        let guard = BIN_DIR_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let saved = std::env::var_os("KXBPS_XBPS_BIN_DIR");
        // SAFETY: the lock above is the only access to this variable in the
        // whole suite, so no other thread reads it while it points at a test
        // directory.
        let bin = bin.to_path_buf();
        unsafe { std::env::set_var("KXBPS_XBPS_BIN_DIR", &bin) };
        let answer = body();
        unsafe {
            match saved {
                Some(value) => std::env::set_var("KXBPS_XBPS_BIN_DIR", value),
                None => std::env::remove_var("KXBPS_XBPS_BIN_DIR"),
            }
        }
        drop(guard);
        answer
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
            let text = render_config(&base, "");
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
