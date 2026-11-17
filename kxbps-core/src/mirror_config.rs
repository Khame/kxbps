// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The system repository configuration xbps actually reads.
//!
//! The mirror list in [`crate::mirrors`] is a catalog; xbps uses whatever the
//! `.conf` files under `/etc/xbps.d` say, and only
//! `00-repository-main.conf` decides which `current` repository the system
//! resolves against. This module reads that one file, parses it, and writes it
//! atomically, with a backup and a revert so a failed refresh leaves the
//! previous file in place:
//!
//! - The file holds a repository URL, `repository=https://repo-de.voidlinux.org/current`.
//!   The part before `current` is the mirror; the part after it is the system's
//!   repository tail (`musl` on a musl system, `multilib/nonfree` for a nested
//!   one, empty on glibc) and belongs to the system, not to the mirror. Writing
//!   a new mirror keeps the tail, so switching hosts on musl never points the
//!   system at the glibc repository. A bare base URL without `current` is
//!   accepted and normalised to `<base>current`.
//! - The backup goes to `00-repository-main.conf.kxbps-bak`. The hand-made
//!   `.old` next to it belongs to the human and is never touched.
//! - The new file is written as a temp file in the same directory, chmod
//!   `0644`, chowned `0:0` when that is possible, fsynced and renamed over the
//!   target. Target and temp are opened with `O_NOFOLLOW`, so a symlinked
//!   configuration is refused instead of replaced.
//!
//! Every path is handed in, so the tests run under a temporary directory and
//! nothing here ever hardcodes `/etc`.
//!
//! ```
//! use kxbps_core::mirror_config::parse_repository_conf;
//!
//! let text = "repository=https://repo-de.voidlinux.org/current\n";
//! assert_eq!(parse_repository_conf(text).unwrap().base, "https://repo-de.voidlinux.org/");
//! ```

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::probe::{ProbeError, check_probe_url};

/// The file under `/etc/xbps.d` that decides the system repository.
///
/// Any other name leaves the system `00` in place and adds a second `current`,
/// so the helper writes exactly this path.
pub const SYSTEM_CONFIG_PATH: &str = "/etc/xbps.d/00-repository-main.conf";

/// The packaged default xbps reads when the administrator's file is absent.
pub const SYSTEM_DEFAULT_PATH: &str = "/usr/share/xbps.d/00-repository-main.conf";

/// Suffix of the backup [`write_config_atomic`] leaves behind.
pub const BACKUP_SUFFIX: &str = ".kxbps-bak";

/// Suffix xbps appends to a base URL; it must never appear in the file.
const CURRENT: &str = "/current";

/// The comment line [`render_config`] writes above the repository.
const HEADER: &str = "# Written by kxbps; the backup of the previous file is next to this one.";

/// Any failure while validating, reading or writing the configuration.
///
/// Carries no translated text: a variant names the problem, and the caller
/// chooses the key and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    Read { path: PathBuf, reason: String },
    /// The value is not a base URL xbps can use.
    Parse { line: String, reason: ParseReason },
    /// The file is not there; a missing file is an error here, not a default.
    Missing { path: PathBuf },
    /// The target is a symlink and is refused instead of followed.
    Symlink { path: PathBuf },
    /// A write, rename, sync or backup failed.
    Write { path: PathBuf, reason: String },
}

impl ConfigError {
    /// Stable translation key for the UI; never shown to the user as is.
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Read { .. } | Self::Missing { .. } => "errorMirrorConfigRead",
            Self::Parse { .. } => "errorMirrorConfigParse",
            Self::Symlink { .. } | Self::Write { .. } => "errorMirrorWrite",
        }
    }

    /// The arguments for [`Self::key`]: the path, and the offending line when
    /// the problem is in the file's content.
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::Read { path, .. } | Self::Missing { path } => vec![display(path)],
            Self::Parse { line, .. } => vec![line.clone(), self.reason_text().to_owned()],
            Self::Symlink { path } | Self::Write { path, .. } => vec![display(path)],
        }
    }

    /// Why a parse failed, in English, for the log and the report.
    pub const fn reason_text(&self) -> &'static str {
        match self {
            Self::Parse { reason, .. } => reason.text(),
            _ => "",
        }
    }
}

/// Why a `repository=` value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseReason {
    /// `repository=` with nothing after it.
    EmptyValue,
    /// No `repository=` line at all: a commented-out line is not a value.
    NotFound,
    /// The value ends in `/current`; the file holds the base URL.
    EndsWithCurrent,
    /// The value is served over plain HTTP.
    PlainHttp,
    /// The value is not an absolute `https://host/` URL.
    NotAUrl,
    /// The path after `/current` is not a plain repository name.
    BadTail,
    /// Two active `repository=` lines; only one `current` may exist.
    TooManyValues,
}

impl ParseReason {
    /// Short English text used for logs and [`ConfigError::reason_text`].
    pub const fn text(self) -> &'static str {
        match self {
            Self::EmptyValue => "the value is empty",
            Self::NotFound => "there is no repository= line",
            Self::EndsWithCurrent => "the value ends in /current; write the base URL instead",
            Self::PlainHttp => "the value uses plain http",
            Self::NotAUrl => "the value is not a https://host/ URL",
            Self::BadTail => "the path after /current is not a repository name",
            Self::TooManyValues => "more than one repository= line",
        }
    }
}

/// A parsed configuration file.
///
/// `base` always ends in exactly one `/` and never in `/current`, so it
/// compares equal to the `MIRRORS[i].url` of the same mirror. `tail` is the
/// path after `current` without surrounding slashes: `musl` on a musl system,
/// `multilib/nonfree` for a nested repository, empty on glibc. `already_adapted`
/// means the file already names `/current`, which `xbps-query -L` shows as the
/// system repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemConfig {
    pub base: String,
    pub tail: String,
    pub already_adapted: bool,
}

impl SystemConfig {
    /// The URL xbps resolves, `<base>current[/<tail>]`.
    pub fn current_url(&self) -> String {
        current_url(&self.base, &self.tail)
    }
}

/// The URL xbps resolves for a mirror base and a repository tail.
pub fn current_url(base: &str, tail: &str) -> String {
    repository_path(base, tail)
}

/// Joins a base and a tail into `<base>current[/<tail>]`, both without
/// duplicate slashes.
fn repository_path(base: &str, tail: &str) -> String {
    let base = base.trim_end_matches('/');
    let tail = tail.trim_matches('/');
    if tail.is_empty() {
        format!("{base}{CURRENT}")
    } else {
        format!("{base}{CURRENT}/{tail}")
    }
}

/// `write_config_atomic` may be asked to leave a backup of an existing file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backup {
    /// Copy the previous bytes to `<path>.kxbps-bak` first.
    Keep,
    /// Do not write a backup; used for reverts and for tests that only care
    /// about the write.
    Skip,
}

impl Backup {
    /// The backup path for a target: `00-repository-main.conf.kxbps-bak`.
    pub fn path_for(config_path: &Path) -> PathBuf {
        let mut name = config_path.as_os_str().to_owned();
        name.push(BACKUP_SUFFIX);
        PathBuf::from(name)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { line, reason } => write!(f, "line {line:?}: {}", reason.text()),
            Self::Read { path, reason } | Self::Write { path, reason } => {
                write!(f, "{}: {reason}", path.display())
            }
            Self::Missing { path } => write!(f, "{}: file not found", path.display()),
            Self::Symlink { path } => write!(f, "{}: is a symlink", path.display()),
        }
    }
}

impl fmt::Display for SystemConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base)
    }
}

/// Parses the content of `00-repository-main.conf`.
///
/// Blank lines and lines starting with `#` are ignored. Exactly one active
/// `repository=` line is required; its value must be an absolute `https://`
/// URL with a host, may name `current` and a repository tail after it, and must
/// not be plain HTTP. Anything else is a [`ConfigError::Parse`].
///
/// # Examples
///
/// ```
/// use kxbps_core::mirror_config::{ParseReason, parse_repository_conf};
///
/// let config =
///     parse_repository_conf("# host\nrepository=https://repo-default.voidlinux.org/\n").unwrap();
/// assert_eq!(config.base, "https://repo-default.voidlinux.org/");
/// assert_eq!(config.tail, "");
/// assert_eq!(
///     parse_repository_conf("repository=https://repo-default.voidlinux.org/current/musl\n")
///         .unwrap()
///         .tail,
///     "musl"
/// );
/// assert_eq!(
///     parse_repository_conf("repository=http://mirror.example/\n").unwrap_err().reason_text(),
///     ParseReason::PlainHttp.text()
/// );
/// ```
pub fn parse_repository_conf(input: &str) -> Result<SystemConfig, ConfigError> {
    let mut found: Option<String> = None;
    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "repository" {
            continue;
        }
        if found.is_some() {
            return Err(parse_error(line, ParseReason::TooManyValues));
        }
        let value = value.trim();
        if value.is_empty() {
            return Err(parse_error(line, ParseReason::EmptyValue));
        }
        found = Some(value.to_owned());
    }
    let value = found.ok_or_else(|| parse_error("", ParseReason::NotFound))?;
    check_repository_value(&value)
}

/// Checks one `repository=` value and splits it into base and tail.
///
/// Refused: plain HTTP, a missing scheme, a missing host, a dot segment in the
/// path, a tail that is not plain repository names, and anything that is not
/// `https://host[/path][/current[/tail]]`. A value that names `/current` is
/// returned with the base before it, the tail after it and `already_adapted`,
/// so the read side can report it and the write side can keep the tail.
pub fn check_repository_value(value: &str) -> Result<SystemConfig, ConfigError> {
    let line = format!("repository={value}");
    if let Err(ProbeError::PlainHttp) = check_probe_url(value) {
        return Err(parse_error(&line, ParseReason::PlainHttp));
    }
    let rest = value
        .strip_prefix("https://")
        .ok_or_else(|| parse_error(&line, ParseReason::NotAUrl))?;
    let (host_path, current) = match split_current(rest) {
        Some((host, tail)) => (host, Some(tail)),
        None => (rest, None),
    };
    if host_path.is_empty() || host_path.starts_with('/') {
        return Err(parse_error(&line, ParseReason::NotAUrl));
    }
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    let valid_host = !host.is_empty()
        && !host.starts_with('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    let valid_path = !path.split('/').any(|segment| segment.starts_with('.'));
    if !valid_host || !valid_path {
        return Err(parse_error(&line, ParseReason::NotAUrl));
    }
    let tail = match current {
        None => String::new(),
        Some(tail) if valid_tail(tail) => tail.to_owned(),
        Some(_) => return Err(parse_error(&line, ParseReason::BadTail)),
    };
    let trimmed = format!("https://{}", host_path.trim_end_matches('/'));
    Ok(SystemConfig {
        base: format!("{trimmed}/"),
        tail,
        already_adapted: current.is_some(),
    })
}

/// Splits `host[/path]` from `/current[/tail]` at the first `current` segment.
///
/// Returns `None` when the value names no `current` segment at all; the tail is
/// empty for `.../current` and `.../current/`.
fn split_current(rest: &str) -> Option<(&str, &str)> {
    let mut from = 0;
    while let Some(offset) = rest[from..].find(CURRENT) {
        let start = from + offset;
        let after = &rest[start + CURRENT.len()..];
        if after.is_empty() {
            return Some((&rest[..start], ""));
        }
        if let Some(tail) = after.strip_prefix('/') {
            return Some((&rest[..start], tail));
        }
        from = start + CURRENT.len();
    }
    None
}

/// Whether a tail is a plain relative path: no empty or dot segments.
fn valid_tail(tail: &str) -> bool {
    tail.is_empty()
        || tail.split('/').all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('.')
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
}

/// A value the helper is about to write: refuses `/current` in every
/// spelling, plain HTTP and anything [`check_repository_value`] rejects.
pub fn check_base_url(url: &str) -> Result<String, ConfigError> {
    let config = check_repository_value(url)?;
    if config.already_adapted {
        let line = format!("repository={url}");
        return Err(parse_error(&line, ParseReason::EndsWithCurrent));
    }
    Ok(config.base)
}

/// The exact bytes [`write_config_atomic`] writes for a base and a tail.
///
/// The base must already be normalised by [`check_base_url`]; the content is a
/// header naming kxbps and one `repository=<base>current[/<tail>]` line.
pub fn render_config(base: &str, tail: &str) -> String {
    format!("{HEADER}\nrepository={}\n", repository_path(base, tail))
}

fn parse_error(line: &str, reason: ParseReason) -> ConfigError {
    ConfigError::Parse {
        line: line.to_owned(),
        reason,
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn read_error(path: &Path, error: &io::Error) -> ConfigError {
    ConfigError::Read {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

fn write_error(path: &Path, error: &io::Error) -> ConfigError {
    ConfigError::Write {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

/// Reads and parses a configuration file.
///
/// A missing file is [`ConfigError::Missing`], never the default mirror: the
/// caller has to distinguish "xbps uses something we did not write" from
/// "there is no configuration at all". A symlink is refused before it is read,
/// so a link is never reported as the system repository.
pub fn read_config(path: &Path) -> Result<SystemConfig, ConfigError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ConfigError::Missing {
                path: path.to_path_buf(),
            });
        }
        Err(error) => return Err(read_error(path, &error)),
    };
    if meta.file_type().is_symlink() {
        return Err(ConfigError::Symlink {
            path: path.to_path_buf(),
        });
    }
    let text = fs::read_to_string(path).map_err(|error| read_error(path, &error))?;
    parse_repository_conf(&text)
}

/// Reads the administrator's file, falling back to the packaged default.
///
/// xbps merges `/etc/xbps.d` over `/usr/share/xbps.d`, so a missing local
/// `00-repository-main.conf` is not an error: the packaged one is what xbps
/// uses. Any other failure, a broken file included, is still reported: a
/// configuration that exists but cannot be read must not be hidden.
pub fn read_config_with_default(path: &Path, default: &Path) -> Result<SystemConfig, ConfigError> {
    match read_config(path) {
        Err(error @ ConfigError::Missing { .. }) => match read_config(default) {
            // Both missing: report the path the administrator would edit.
            Err(ConfigError::Missing { .. }) => Err(error),
            other => other,
        },
        other => other,
    }
}

/// The previous bytes of a target, as a revert needs them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Previous {
    /// The file existed and held exactly these bytes.
    Bytes { path: PathBuf, bytes: Vec<u8> },
    /// The file did not exist; a revert removes what was written.
    Absent { path: PathBuf },
}

impl Previous {
    /// The target this previous state restores.
    pub fn path(&self) -> &Path {
        match self {
            Self::Bytes { path, .. } | Self::Absent { path } => path,
        }
    }
}

/// Captures what a revert has to put back, before anything is written.
pub fn snapshot_previous(path: &Path) -> Result<Previous, ConfigError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Previous::Absent {
                path: path.to_path_buf(),
            });
        }
        Err(error) => return Err(read_error(path, &error)),
    };
    if meta.file_type().is_symlink() {
        return Err(ConfigError::Symlink {
            path: path.to_path_buf(),
        });
    }
    let bytes = fs::read(path).map_err(|error| read_error(path, &error))?;
    Ok(Previous::Bytes {
        path: path.to_path_buf(),
        bytes,
    })
}

/// Writes `content` to `path` without ever leaving a half-written file.
///
/// The temp file (`.<name>.kxbps-tmp`) lives in the same directory so the
/// rename stays atomic. It is chmod `0644`, chowned to `0:0` when the process
/// may (it runs as root in the helper; in a test it cannot and the chown is
/// skipped), fsynced, renamed over the target, and the directory is fsynced
/// once so the rename survives a crash. A symlink at the target is refused and
/// the link itself is left in place.
pub fn write_config_atomic(path: &Path, content: &[u8], backup: Backup) -> Result<(), ConfigError> {
    // Refuse a symlink before anything is created or renamed.
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(ConfigError::Symlink {
                path: path.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(read_error(path, &error)),
    }

    if backup == Backup::Keep && path.exists() {
        let bytes = fs::read(path).map_err(|error| read_error(path, &error))?;
        backup_bytes(&Backup::path_for(path), &bytes)?;
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| ConfigError::Write {
            path: path.to_path_buf(),
            reason: String::from("the path has no file name"),
        })?
        .to_string_lossy()
        .into_owned();
    let temp_path = dir.join(format!(".{name}.kxbps-tmp"));
    let _ = fs::remove_file(&temp_path);

    {
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc_no_follow())
            .mode(0o644)
            .open(&temp_path)
            .map_err(|error| write_error(&temp_path, &error))?;
        temp.write_all(content)
            .map_err(|error| write_error(&temp_path, &error))?;
        temp.set_permissions(fs::Permissions::from_mode(0o644))
            .map_err(|error| write_error(&temp_path, &error))?;
        chown_root(&temp);
        temp.sync_all()
            .map_err(|error| write_error(&temp_path, &error))?;
    }

    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(write_error(path, &error));
    }
    if let Ok(dir_handle) = File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}

/// Restores the bytes a previous [`snapshot_previous`] captured.
///
/// A file that did not exist is removed again; a file that did is rewritten
/// through [`write_config_atomic`] and keeps its backup. A file that had to be
/// removed and is already gone is a success, so a revert can be retried.
pub fn revert(previous: &Previous) -> Result<(), ConfigError> {
    match previous {
        Previous::Bytes { path, bytes } => {
            write_config_atomic(path, bytes, Backup::Skip)?;
            Ok(())
        }
        Previous::Absent { path } => match fs::symlink_metadata(path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !meta.is_file() {
                    return Err(ConfigError::Symlink {
                        path: path.to_path_buf(),
                    });
                }
                fs::remove_file(path).map_err(|error| write_error(path, &error))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(write_error(path, &error)),
        },
    }
}

/// Chowns a file to `0:0` when the process may; a test under a normal user
/// cannot, and the file stays owned by that user.
fn chown_root(file: &File) {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `fchown` only touches the fd that is open here. A failed chown is
    // ignored on purpose: the caller is either root (and it worked) or not (and
    // it cannot).
    unsafe {
        fchown(file.as_raw_fd(), 0, 0);
    }
}

unsafe extern "C" {
    fn fchown(fd: i32, owner: u32, group: u32) -> i32;
}

fn backup_bytes(backup: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    match fs::symlink_metadata(backup) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(ConfigError::Symlink {
                path: backup.to_path_buf(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(write_error(backup, &error)),
    }
    fs::write(backup, bytes).map_err(|error| write_error(backup, &error))?;
    let _ = fs::set_permissions(backup, fs::Permissions::from_mode(0o644));
    if let Ok(handle) = File::open(backup) {
        let _ = handle.sync_all();
    }
    Ok(())
}

/// `O_NOFOLLOW` without a `libc` dependency, matching the Linux value.
///
/// Anything that is not Linux still works; it just loses the refusal to
/// follow a symlink, which the explicit `symlink_metadata` check above
/// already covers for the final component.
fn libc_no_follow() -> i32 {
    #[cfg(target_os = "linux")]
    {
        // `/usr/include/asm-generic/fcntl.h`: `#define O_NOFOLLOW 0400000`.
        0o400000
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kxbps-mirror-config-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn parse(text: &str) -> SystemConfig {
        parse_repository_conf(text).expect("valid configuration")
    }

    fn reason(text: &str) -> ParseReason {
        match parse_repository_conf(text) {
            Err(ConfigError::Parse { reason, .. }) => reason,
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    const MAIN: &str = "repository=https://repo-de.voidlinux.org/current\n";

    #[test]
    fn main_file_with_current_parses_base() {
        let config = parse(MAIN);
        assert_eq!(config.base, "https://repo-de.voidlinux.org/");
        assert!(config.already_adapted);
        assert_eq!(
            config.current_url(),
            MAIN.trim().trim_start_matches("repository=")
        );
    }

    #[test]
    fn a_base_without_current_parses_and_is_not_adapted() {
        let config = parse("repository=https://repo-default.voidlinux.org/\n");
        assert_eq!(config.base, "https://repo-default.voidlinux.org/");
        assert!(!config.already_adapted);
        assert_eq!(
            config.current_url(),
            "https://repo-default.voidlinux.org/current"
        );
    }

    #[test]
    fn the_base_keeps_exactly_one_trailing_slash() {
        assert_eq!(
            parse("repository=https://repo-de.voidlinux.org").base,
            "https://repo-de.voidlinux.org/"
        );
        assert_eq!(
            parse("repository=https://khame.github.io/void-packages/").base,
            "https://khame.github.io/void-packages/"
        );
        assert_eq!(
            parse("repository=https://repo-de.voidlinux.org/current/").base,
            "https://repo-de.voidlinux.org/"
        );
    }

    #[test]
    fn a_current_sub_repository_is_not_a_mirror() {
        let config = parse("repository=https://repo-de.voidlinux.org/current/multilib\n");
        assert_eq!(config.base, "https://repo-de.voidlinux.org/");
        assert_eq!(config.tail, "multilib");
        assert!(config.already_adapted);
        assert_eq!(
            check_base_url("https://repo-de.voidlinux.org/current/multilib"),
            Err(ConfigError::Parse {
                line: String::from("repository=https://repo-de.voidlinux.org/current/multilib"),
                reason: ParseReason::EndsWithCurrent,
            })
        );
    }

    #[test]
    fn a_musl_value_keeps_the_repository_tail() {
        let config = parse("repository=https://repo-de.voidlinux.org/current/musl\n");
        assert_eq!(config.base, "https://repo-de.voidlinux.org/");
        assert_eq!(config.tail, "musl");
        assert!(config.already_adapted);
        assert_eq!(
            config.current_url(),
            "https://repo-de.voidlinux.org/current/musl"
        );
    }

    #[test]
    fn a_glibc_value_has_an_empty_tail() {
        let config = parse(MAIN);
        assert_eq!(config.tail, "");
        assert_eq!(
            config.current_url(),
            "https://repo-de.voidlinux.org/current"
        );
    }

    #[test]
    fn a_nested_tail_is_kept_and_rendered_back() {
        let config = parse("repository=https://repo-de.voidlinux.org/current/multilib/nonfree\n");
        assert_eq!(config.tail, "multilib/nonfree");
        assert_eq!(
            render_config(&config.base, &config.tail),
            format!(
                "{HEADER}\nrepository=https://repo-de.voidlinux.org/current/multilib/nonfree\n"
            )
        );
    }

    #[test]
    fn a_tail_with_an_empty_or_dot_segment_is_refused() {
        for text in [
            "repository=https://repo-de.voidlinux.org/current//musl\n",
            "repository=https://repo-de.voidlinux.org/current/../musl\n",
            "repository=https://repo-de.voidlinux.org/current/musl/.\n",
        ] {
            assert_eq!(reason(text), ParseReason::BadTail, "{text:?}");
        }
    }

    #[test]
    fn render_config_writes_the_tail_after_current() {
        assert_eq!(
            render_config("https://repo-default.voidlinux.org/", "musl"),
            format!("{HEADER}\nrepository=https://repo-default.voidlinux.org/current/musl\n")
        );
        assert_eq!(
            render_config("https://repo-default.voidlinux.org/", ""),
            format!("{HEADER}\nrepository=https://repo-default.voidlinux.org/current\n")
        );
    }

    #[test]
    fn commented_repository_is_ignored() {
        let text = "# repository=https://old.example/current\n\n   \nrepository=https://repo-de.voidlinux.org/\n";
        assert_eq!(parse(text).base, "https://repo-de.voidlinux.org/");
    }

    #[test]
    fn empty_value_is_an_error() {
        assert_eq!(reason("repository=\n"), ParseReason::EmptyValue);
        assert_eq!(reason("repository=   \n"), ParseReason::EmptyValue);
        assert_eq!(
            reason("# repository=https://repo-de.voidlinux.org/\n"),
            ParseReason::NotFound,
            "a commented line is not a value"
        );
    }

    #[test]
    fn plain_http_is_rejected() {
        assert_eq!(
            reason("repository=http://mirror.example/\n"),
            ParseReason::PlainHttp
        );
    }

    #[test]
    fn a_value_on_a_non_https_scheme_or_without_a_host_is_rejected() {
        for text in [
            "repository=mirror.example/\n",
            "repository=ftp://mirror.example/\n",
            "repository=https://\n",
            "repository=https:///path/\n",
            "repository=https://user@mirror.example/\n",
        ] {
            assert_eq!(reason(text), ParseReason::NotAUrl, "{text:?}");
        }
    }

    #[test]
    fn two_repository_lines_are_an_error() {
        let text = "repository=https://a.example/\nrepository=https://b.example/\n";
        assert_eq!(reason(text), ParseReason::TooManyValues);
        assert_eq!(
            ConfigError::Parse {
                line: String::from("repository=https://b.example/"),
                reason: ParseReason::TooManyValues,
            }
            .key(),
            "errorMirrorConfigParse"
        );
    }

    #[test]
    fn other_keys_are_not_repository_values() {
        let text = "repository-lock=/var/lock/xbps\nrepository=https://a.example/\n";
        assert_eq!(parse(text).base, "https://a.example/");
    }

    #[test]
    fn check_base_url_accepts_a_mirror_and_refuses_current() {
        assert_eq!(
            check_base_url("https://repo-default.voidlinux.org/").as_deref(),
            Ok("https://repo-default.voidlinux.org/")
        );
        assert_eq!(
            check_base_url("https://repo-default.voidlinux.org").as_deref(),
            Ok("https://repo-default.voidlinux.org/")
        );
        assert_eq!(
            check_base_url("https://repo-de.voidlinux.org/current")
                .unwrap_err()
                .reason_text(),
            ParseReason::EndsWithCurrent.text()
        );
        assert_eq!(
            check_base_url("http://repo-default.voidlinux.org/")
                .unwrap_err()
                .reason_text(),
            ParseReason::PlainHttp.text()
        );
    }

    #[test]
    fn the_rendered_file_names_kxbps_and_one_current_repository() {
        let text = render_config("https://repo-default.voidlinux.org/", "");
        assert_eq!(
            text,
            "# Written by kxbps; the backup of the previous file is next to this one.\n\
             repository=https://repo-default.voidlinux.org/current\n"
        );
        let config = parse_repository_conf(&text).expect("what we write parses back");
        assert_eq!(config.base, "https://repo-default.voidlinux.org/");
        assert!(config.already_adapted);
    }

    #[test]
    fn missing_file_is_read_error_not_default() {
        let dir = tempdir("missing");
        let path = dir.join("00-repository-main.conf");
        let error = read_config(&path).expect_err("a missing file is an error");
        assert_eq!(error.key(), "errorMirrorConfigRead");
        assert_eq!(error.args(), [display(&path)]);
        assert!(matches!(error, ConfigError::Missing { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_config_returns_the_base_of_a_real_file() {
        let dir = tempdir("read");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("write fixture");
        assert_eq!(
            read_config(&path).expect("reads").base,
            "https://repo-de.voidlinux.org/"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_config_refuses_a_symlink() {
        let dir = tempdir("read-symlink");
        let real = dir.join("real.conf");
        fs::write(&real, MAIN).expect("write fixture");
        let link = dir.join("00-repository-main.conf");
        symlink(&real, &link).expect("symlink");
        assert!(matches!(
            read_config(&link),
            Err(ConfigError::Symlink { .. })
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_backs_up_the_existing_bytes_and_leaves_a_0644_file() {
        let dir = tempdir("write");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("write fixture");
        let old = fs::read(&path).expect("read fixture");

        let new = render_config("https://repo-default.voidlinux.org/", "");
        write_config_atomic(&path, new.as_bytes(), Backup::Keep).expect("writes");

        assert_eq!(fs::read(&path).expect("read"), new.as_bytes());
        assert_eq!(
            fs::read(Backup::path_for(&path)).expect("backup"),
            old,
            "the backup holds the previous bytes"
        );
        assert_eq!(
            Backup::path_for(&path)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "00-repository-main.conf.kxbps-bak"
        );
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o7777;
        assert_eq!(mode, 0o644);
        assert!(
            !dir.join(".00-repository-main.conf.kxbps-tmp").exists(),
            "the temp file is renamed away, not left behind"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_never_touches_the_hand_made_old_backup() {
        let dir = tempdir("old");
        let path = dir.join("00-repository-main.conf");
        let old = dir.join("00-repository-main.conf.old");
        fs::write(&path, MAIN).expect("write fixture");
        fs::write(&old, "repository=https://old.example/current\n").expect("write old");
        write_config_atomic(
            &path,
            render_config("https://repo-default.voidlinux.org/", "").as_bytes(),
            Backup::Keep,
        )
        .expect("writes");
        assert_eq!(
            fs::read(&old).expect("read old"),
            b"repository=https://old.example/current\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_without_an_existing_file_creates_one_and_a_revert_removes_it() {
        let dir = tempdir("no-existing");
        let path = dir.join("00-repository-main.conf");
        let previous = snapshot_previous(&path).expect("snapshot");
        assert_eq!(previous, Previous::Absent { path: path.clone() });
        write_config_atomic(
            &path,
            render_config("https://repo-default.voidlinux.org/", "").as_bytes(),
            Backup::Keep,
        )
        .expect("writes");
        assert!(path.exists());
        assert!(
            !Backup::path_for(&path).exists(),
            "no backup when there was nothing to back up"
        );

        revert(&previous).expect("reverts");
        assert!(
            !path.exists(),
            "the file did not exist before and is gone again"
        );
        revert(&previous).expect("a second revert of a gone file is fine");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_revert_puts_the_previous_bytes_back() {
        let dir = tempdir("revert");
        let path = dir.join("00-repository-main.conf");
        fs::write(&path, MAIN).expect("write fixture");
        let previous = snapshot_previous(&path).expect("snapshot");
        write_config_atomic(
            &path,
            render_config("https://repo-default.voidlinux.org/", "").as_bytes(),
            Backup::Keep,
        )
        .expect("writes");
        assert_ne!(fs::read(&path).expect("read"), MAIN.as_bytes());
        revert(&previous).expect("reverts");
        assert_eq!(fs::read(&path).expect("read"), MAIN.as_bytes());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlinked_config_is_refused_and_the_link_survives() {
        let dir = tempdir("symlink");
        let real = dir.join("real.conf");
        fs::write(&real, MAIN).expect("write fixture");
        let link = dir.join("00-repository-main.conf");
        symlink(&real, &link).expect("symlink");

        let error = write_config_atomic(
            &link,
            render_config("https://repo-default.voidlinux.org/", "").as_bytes(),
            Backup::Keep,
        )
        .expect_err("a symlink is refused");
        assert_eq!(error.key(), "errorMirrorWrite");
        assert!(matches!(error, ConfigError::Symlink { .. }));
        assert_eq!(fs::read(&real).expect("read"), MAIN.as_bytes());
        assert!(
            fs::symlink_metadata(&link)
                .expect("link still there")
                .file_type()
                .is_symlink()
        );
        assert!(
            !Backup::path_for(&link).exists(),
            "nothing is backed up or written through the link"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_parse_error_key_and_arguments_name_the_line() {
        let error = ConfigError::Parse {
            line: String::from("repository=http://mirror.example/"),
            reason: ParseReason::PlainHttp,
        };
        assert_eq!(error.key(), "errorMirrorConfigParse");
        assert_eq!(
            error.args(),
            [
                String::from("repository=http://mirror.example/"),
                String::from(ParseReason::PlainHttp.text())
            ]
        );
    }
}
