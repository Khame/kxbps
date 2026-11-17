// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Adding and removing one repository file under `/etc/xbps.d`.
//!
//! The engine behind `kxbps-repo-apply`, the second privileged helper. It is
//! deliberately narrower than a general file editor:
//!
//! - only `https://` URLs are accepted, with no whitespace, query or fragment;
//! - a file it writes is named `20-kxbps-<slug>.conf` and holds one
//!   `repository=` line;
//! - a removal only deletes a file with that prefix, so a repository the user
//!   wrote by hand is never touched;
//! - a file that already exists with other content is left alone and reported.
//!
//! The directory is injected, so the tests drive the whole thing in a temporary
//! directory without root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where xbps reads extra repository definitions.
pub const CONFIG_DIR: &str = "/etc/xbps.d";

/// Where the distribution keeps its own repository definitions; xbps reads
/// these after [`CONFIG_DIR`].
pub const SYSTEM_CONFIG_DIR: &str = "/usr/share/xbps.d";

/// Prefix of every file this helper owns. Removal refuses anything else.
pub const FILE_PREFIX: &str = "20-kxbps-";

/// The exit code of "the file is written, but the index could not be fetched".
///
/// Not a [`RepoError`]: the edit succeeded, and the caller shows a warning
/// rather than a failure.
pub const SYNC_FAILED: i32 = 6;

/// Why an edit was refused. Each variant is one exit code and one i18n key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoError {
    /// Not an absolute `https://` URL, or it carries whitespace, a query or a
    /// fragment.
    UrlRefused,
    /// A file for this URL already exists with different content.
    AlreadyPresent,
    /// The file is not one this helper owns, or it holds more than the one
    /// `repository=` line.
    NotManaged,
    /// The write or the removal failed.
    Io,
}

impl RepoError {
    /// The documented process exit status.
    pub const fn code(self) -> i32 {
        match self {
            Self::UrlRefused => 2,
            Self::AlreadyPresent => 3,
            Self::NotManaged => 4,
            Self::Io => 5,
        }
    }

    /// The reverse mapping; `None` for a code this helper never returns.
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            2 => Some(Self::UrlRefused),
            3 => Some(Self::AlreadyPresent),
            4 => Some(Self::NotManaged),
            5 => Some(Self::Io),
            _ => None,
        }
    }

    /// The translation key QML maps to a sentence; empty for success.
    pub const fn key(self) -> &'static str {
        match self {
            Self::UrlRefused => "errorRepoUrl",
            Self::AlreadyPresent => "errorRepoPresent",
            Self::NotManaged => "errorRepoNotManaged",
            Self::Io => "errorRepoIo",
        }
    }
}

/// The URL exactly as xbps should read it, or [`RepoError::UrlRefused`].
///
/// A repository URL is used verbatim: third-party repositories do not all end
/// in `/current`, so appending or trimming a path would point xbps somewhere
/// the user did not name. A trailing slash is kept as typed.
pub fn check_url(url: &str) -> Result<&str, RepoError> {
    let trimmed = url.trim();
    if trimmed != url || url.is_empty() {
        return Err(RepoError::UrlRefused);
    }
    if !url.starts_with("https://") {
        return Err(RepoError::UrlRefused);
    }
    if url
        .chars()
        .any(|ch| ch.is_whitespace() || ch == '?' || ch == '#')
    {
        return Err(RepoError::UrlRefused);
    }
    let rest = &url["https://".len()..];
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() || host.starts_with('.') {
        return Err(RepoError::UrlRefused);
    }
    Ok(url)
}

/// The file name this helper uses for `url`: stable, readable and inside the
/// prefix removal accepts.
///
/// The name is the host plus the whole path, because that is what tells two
/// repositories apart: `repo-de.voidlinux.org/current/multilib` and `…/nonfree`
/// share a host *and* the `current` segment and differ only at the end.
/// Cutting the slug after three hyphen-separated parts dropped the top-level
/// domain and the path, so the second repository of a host was refused as
/// already present.
pub fn file_name(url: &str) -> String {
    let rest = url.trim_start_matches("https://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));

    let mut slug = slugify(host);
    let tail = slugify(path);
    if !tail.is_empty() {
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&tail);
    }
    // A long path would make a name nobody can scan in the list, and the two
    // URLs that differ past the cap are told apart by the helper's refusal, not
    // by a silent overwrite. ASCII only, so cutting at a byte is safe.
    slug.truncate(MAX_SLUG);
    let slug = slug.trim_matches('-');
    format!("{FILE_PREFIX}{slug}.conf")
}

/// How much of the slug a name keeps; the prefix and `.conf` add 14 bytes.
const MAX_SLUG: usize = 48;

/// Lowercase ASCII alphanumerics, with one `-` for every run of anything else.
fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

/// The exact bytes of a repository file.
pub fn content(url: &str) -> String {
    format!("# Written by kxbps.\nrepository={url}\n")
}

/// Writes the file for `url`, or reports why it did not.
///
/// An existing file with the same content is success (the repository is already
/// configured); any other existing file at that name is refused, because it was
/// not written by this helper.
pub fn add(dir: &Path, url: &str) -> Result<PathBuf, RepoError> {
    let url = check_url(url)?;
    let path = dir.join(file_name(url));
    match fs::read(&path) {
        Ok(existing) if existing == content(url).as_bytes() => return Ok(path),
        Ok(_) => return Err(RepoError::AlreadyPresent),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(RepoError::Io),
    }
    fs::create_dir_all(dir).map_err(|_| RepoError::Io)?;
    // The temporary file lives in the same directory, so the rename is atomic.
    let temporary = path.with_extension("conf.kxbps-tmp");
    fs::write(&temporary, content(url)).map_err(|_| RepoError::Io)?;
    fs::rename(&temporary, &path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        RepoError::Io
    })?;
    Ok(path)
}

/// Removes one file this helper owns, named by its bare file name.
///
/// A path with a directory component, a name without the prefix, or a file with
/// anything besides one `repository=` line is refused: the helper must not
/// become a way to delete arbitrary files as root.
pub fn remove(dir: &Path, file: &str) -> Result<(), RepoError> {
    if file.is_empty()
        || file.contains('/')
        || file.contains('\\')
        || !file.starts_with(FILE_PREFIX)
        || !file.ends_with(".conf")
    {
        return Err(RepoError::NotManaged);
    }
    let path = dir.join(file);
    let bytes = fs::read(&path).map_err(|_| RepoError::Io)?;
    if !is_one_repository(&bytes) {
        return Err(RepoError::NotManaged);
    }
    fs::remove_file(&path).map_err(|_| RepoError::Io)
}

/// Whether the file holds exactly one `repository=` line and nothing else but
/// comments and blank lines.
fn is_one_repository(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut repositories = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("repository=") {
            repositories += 1;
            continue;
        }
        return false;
    }
    repositories == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kxbps-repo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn only_absolute_https_urls_are_accepted() {
        assert!(check_url("https://repo.example.org/current").is_ok());
        assert!(check_url("https://khame.github.io/void-packages/").is_ok());
        assert_eq!(
            check_url("http://repo.example.org/"),
            Err(RepoError::UrlRefused)
        );
        assert_eq!(
            check_url("https://repo.example.org/?x=1"),
            Err(RepoError::UrlRefused)
        );
        assert_eq!(
            check_url("https://repo.example.org/#top"),
            Err(RepoError::UrlRefused)
        );
        assert_eq!(
            check_url(" https://repo.example.org/"),
            Err(RepoError::UrlRefused)
        );
        assert_eq!(check_url("https://"), Err(RepoError::UrlRefused));
    }

    #[test]
    fn two_repositories_on_one_host_get_different_names() {
        // The common Void layout: multilib and nonfree live under one host and
        // differ only in the last path segment. A name built from the host
        // alone would make the second add a refusal, not a repository.
        let multilib = file_name("https://repo-de.voidlinux.org/current/multilib");
        let nonfree = file_name("https://repo-de.voidlinux.org/current/nonfree");
        assert_ne!(multilib, nonfree, "{multilib} names both repositories");
        assert!(multilib.starts_with(FILE_PREFIX) && nonfree.starts_with(FILE_PREFIX));
    }

    #[test]
    fn the_file_name_is_stable_and_inside_the_prefix() {
        assert_eq!(
            file_name("https://repo.example.org/current"),
            "20-kxbps-repo-example-org-current.conf"
        );
        assert_eq!(
            file_name("https://khame.github.io/void-packages/"),
            "20-kxbps-khame-github-io-void-packages.conf"
        );
        assert_eq!(
            file_name("https://repo.example.org/"),
            "20-kxbps-repo-example-org.conf"
        );
        // A path longer than the cap is cut, not rejected: the name still has
        // the prefix, the host and a readable start.
        let long = format!("https://repo.example.org/{}", "segment/".repeat(40));
        let name = file_name(&long);
        assert!(
            name.starts_with("20-kxbps-repo-example-org-segment"),
            "{name}"
        );
        assert!(name.len() <= FILE_PREFIX.len() + MAX_SLUG + ".conf".len());
    }

    #[test]
    fn adding_writes_one_repository_line() {
        let dir = temp_dir("add");
        let url = "https://repo.example.org/current";
        let path = add(&dir, url).expect("written");
        assert_eq!(path, dir.join(file_name(url)));
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("repository=https://repo.example.org/current"));
        assert_eq!(fs::read_dir(&dir).expect("list").count(), 1, "no temp left");
    }

    #[test]
    fn adding_the_same_url_twice_is_not_an_error() {
        let dir = temp_dir("twice");
        let url = "https://repo.example.org/current";
        add(&dir, url).expect("first");
        add(&dir, url).expect("second");
    }

    #[test]
    fn a_foreign_file_at_the_same_name_is_left_alone() {
        let dir = temp_dir("foreign");
        let url = "https://repo.example.org/current";
        let path = dir.join(file_name(url));
        fs::write(&path, "repository=https://other.example.org/current\n").expect("write");
        assert_eq!(add(&dir, url), Err(RepoError::AlreadyPresent));
        assert!(path.exists());
    }

    #[test]
    fn removing_only_touches_this_helpers_files() {
        let dir = temp_dir("remove");
        let url = "https://repo.example.org/current";
        let path = add(&dir, url).expect("written");
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        remove(&dir, &name).expect("removed");
        assert!(!path.exists());

        fs::write(
            dir.join("00-repository-main.conf"),
            "repository=https://x/\n",
        )
        .expect("write");
        assert_eq!(
            remove(&dir, "00-repository-main.conf"),
            Err(RepoError::NotManaged)
        );
        assert_eq!(remove(&dir, "../etc/passwd"), Err(RepoError::NotManaged));
    }

    #[test]
    fn a_file_with_two_repositories_is_not_removed() {
        let dir = temp_dir("two");
        let name = format!("{FILE_PREFIX}two.conf");
        fs::write(
            dir.join(&name),
            "repository=https://a/\nrepository=https://b/\n",
        )
        .expect("write");
        assert_eq!(remove(&dir, &name), Err(RepoError::NotManaged));
        assert!(dir.join(&name).exists());
    }
}
