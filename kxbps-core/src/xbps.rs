//! Parsers for the xbps command output the read layer consumes.
//!
//! Every format here was captured from xbps 0.60.7 on Void Linux and is listed
//! in `phase2-real-xbps-plan.md` (Task 17, "Verified against xbps 0.60.7").
//! xbps prints human-readable tables with no versioned interface, so each
//! parser is strict: a line that does not match the documented shape returns
//! [`XbpsError`] with its line number and text instead of being skipped. An
//! empty listing is only accepted where empty output is a real answer (no
//! updates, no search hits); `xbps-query -l` and `xbps-query -L` print at
//! least one line on any working system, so empty output from them is a
//! failure, not an empty result.
//!
//! Blank lines at the end of the output are tolerated because a command ends
//! with a newline. A blank line anywhere else is a format change and fails.
//!
//! Nothing here runs a process; the runners live in `kxbps-qt`.

use serde::{Deserialize, Serialize};

/// A line of xbps output that could not be understood.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum XbpsError {
    /// The line did not match the documented format.
    #[error("xbps output line {line} is not understood ({reason}): {text:?}")]
    MalformedLine {
        /// One-based line number in the command's output.
        line: usize,
        /// The line as printed, untrimmed.
        text: String,
        /// What the parser expected, short enough for a log line.
        reason: &'static str,
    },
    /// The command printed nothing where at least one line was expected.
    #[error("xbps printed no output where at least one line was expected")]
    Empty,
}

/// State column of `xbps-query -l`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstallState {
    /// `ii`: installed and configured.
    #[serde(rename = "ii")]
    Installed,
    /// `uu`: unpacked but not configured.
    #[serde(rename = "uu")]
    Unpacked,
}

impl InstallState {
    /// The state as xbps prints it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "ii",
            Self::Unpacked => "uu",
        }
    }

    /// Parses the state column; `None` for anything xbps did not document.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ii" => Some(Self::Installed),
            "uu" => Some(Self::Unpacked),
            _ => None,
        }
    }
}

/// One installed package (`xbps-query -l`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    /// Package name without the version (`lib32-glibc`).
    pub name: String,
    /// Version and revision (`2.41_1`).
    pub version: String,
    /// The raw `name-version_revision` field (`lib32-glibc-2.41_1`).
    pub pkgver: String,
    pub state: InstallState,
    /// Short description; empty when the package database has none.
    pub description: String,
}

/// Action column of `xbps-install -un` and of every `-n` dry run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum XbpsAction {
    Install,
    Update,
    Remove,
    Configure,
    Reinstall,
}

impl XbpsAction {
    /// The action as xbps prints it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Configure => "configure",
            Self::Reinstall => "reinstall",
        }
    }

    /// Parses the action column; `None` for a word xbps has not used so far.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "install" => Some(Self::Install),
            "update" => Some(Self::Update),
            "remove" => Some(Self::Remove),
            "configure" => Some(Self::Configure),
            "reinstall" => Some(Self::Reinstall),
            _ => None,
        }
    }
}

/// One line of `xbps-install -un`, which is also the line format of a `-n`
/// dry run, so the Updates list and the Plan dialog read the same type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEntry {
    /// Package name without the version.
    pub name: String,
    /// Version and revision the action applies.
    pub version: String,
    /// The raw `name-version_revision` field.
    pub pkgver: String,
    pub action: XbpsAction,
    /// Target architecture (`x86_64`, `noarch`, …).
    pub arch: String,
    /// Repository URL the package comes from.
    pub repository: String,
    /// Installed size in bytes.
    pub installed_size: u64,
    /// Download size in bytes.
    pub download_size: u64,
}

/// One hit of `xbps-query -Rs <pattern>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// `[*]` in the output: the package is installed (`[-]` means it is not).
    pub installed: bool,
    /// Package name without the version.
    pub name: String,
    /// Version and revision in the repository index.
    pub version: String,
    /// The raw `name-version_revision` field.
    pub pkgver: String,
    pub description: String,
}

/// One repository of `xbps-query -L`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    /// Package count in the repository index; `-1` when it is unavailable.
    pub count: i64,
    pub url: String,
    /// The index is signed (the `(RSA signed)` note is present).
    pub signed: bool,
}

impl RepoEntry {
    /// Whether xbps could read the repository index.
    pub const fn available(&self) -> bool {
        self.count >= 0
    }
}

/// Splits `name-version_revision` into its name and version parts.
///
/// xbps splits a pkgver at the last `-`: `xbps-query -R -p pkgname` returns
/// everything before it, so the name is that prefix and the version is the
/// rest. Both halves may contain hyphens and digits (`lib32-glibc`,
/// `linux6.18-headers`, `python3`), and versions may start with a letter
/// (`UEFITool-A76_1`, `mksh-R59c_2`), which is why nothing here looks at the
/// version's first character. The version must end in a numeric `_revision`;
/// a pkgver without one is a format change, so the caller fails loudly
/// instead of inventing a split.
///
/// # Examples
///
/// ```
/// use kxbps_core::xbps::split_pkgver;
///
/// assert_eq!(split_pkgver("lib32-glibc-2.41_1"), Some(("lib32-glibc", "2.41_1")));
/// assert_eq!(split_pkgver("python3-3.13.1_1"), Some(("python3", "3.13.1_1")));
/// assert_eq!(
///     split_pkgver("linux6.18-headers-6.18.63_1"),
///     Some(("linux6.18-headers", "6.18.63_1"))
/// );
/// assert_eq!(split_pkgver("mksh-R59c_2"), Some(("mksh", "R59c_2")));
/// assert_eq!(split_pkgver("foo-1.0"), None);
/// ```
pub fn split_pkgver(pkgver: &str) -> Option<(&str, &str)> {
    let (name, version) = pkgver.rsplit_once('-')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    let (_, revision) = version.rsplit_once('_')?;
    if revision.is_empty() || !revision.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((name, version))
}

/// Parses `xbps-query -l`: `ii <pkgver> <description>`, one package per line.
pub fn parse_installed(input: &str) -> Result<Vec<InstalledPackage>, XbpsError> {
    let mut packages = Vec::new();
    for (line_no, line) in body_lines(input) {
        let (state_text, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| malformed(line_no, line, "expected a state and a package name"))?;
        let state = InstallState::parse(state_text)
            .ok_or_else(|| malformed(line_no, line, "unknown package state"))?;
        let (pkgver, description) = split_first_field(rest)
            .ok_or_else(|| malformed(line_no, line, "missing package name"))?;
        let (name, version) = split_pkgver(pkgver)
            .ok_or_else(|| malformed(line_no, line, "cannot split package name and version"))?;
        packages.push(InstalledPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            state,
            description: description.to_owned(),
        });
    }
    if packages.is_empty() {
        return Err(XbpsError::Empty);
    }
    Ok(packages)
}

/// Parses `xbps-install -un` and `xbps-install -n`:
/// `<pkgver> <action> <arch> <repository> <installedsize> <downloadsize>`.
///
/// Empty output means the system is up to date and is a valid empty list.
pub fn parse_updates(input: &str) -> Result<Vec<UpdateEntry>, XbpsError> {
    let mut entries = Vec::new();
    for (line_no, line) in body_lines(input) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 6 {
            return Err(malformed(
                line_no,
                line,
                "expected six fields: pkgver, action, arch, repository, sizes",
            ));
        }
        let (name, version) = split_pkgver(fields[0])
            .ok_or_else(|| malformed(line_no, line, "cannot split package name and version"))?;
        let action = XbpsAction::parse(fields[1])
            .ok_or_else(|| malformed(line_no, line, "unknown action"))?;
        let installed_size = parse_size(line_no, line, fields[4])?;
        let download_size = parse_size(line_no, line, fields[5])?;
        entries.push(UpdateEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: fields[0].to_owned(),
            action,
            arch: fields[2].to_owned(),
            repository: fields[3].to_owned(),
            installed_size,
            download_size,
        });
    }
    Ok(entries)
}

/// Parses `xbps-query -Rs <pattern>`: `[-] <pkgver> <description>` per hit,
/// with `[*]` instead of `[-]` for an installed package. xbps never prints
/// `[ii]` here, even though `xbps-query -l` uses `ii` as its state column.
///
/// Empty output means the pattern matched nothing and is a valid empty list.
pub fn parse_search(input: &str) -> Result<Vec<SearchHit>, XbpsError> {
    let mut hits = Vec::new();
    for (line_no, line) in body_lines(input) {
        let (tag, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| malformed(line_no, line, "expected a state tag and a package name"))?;
        let installed = match tag {
            "[*]" => true,
            "[-]" => false,
            _ => {
                return Err(malformed(line_no, line, "expected [*] or [-] as state tag"));
            }
        };
        let (pkgver, description) = split_first_field(rest)
            .ok_or_else(|| malformed(line_no, line, "missing package name"))?;
        let (name, version) = split_pkgver(pkgver)
            .ok_or_else(|| malformed(line_no, line, "cannot split package name and version"))?;
        hits.push(SearchHit {
            installed,
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            description: description.to_owned(),
        });
    }
    Ok(hits)
}

/// Parses `xbps-query -L`: `<count> <url> (RSA signed)` per repository.
///
/// A count of `-1` means xbps could not read the index. The parenthetical is
/// optional; when present it must be exactly `(RSA signed)`, because any other
/// note would be a new format the caller has to learn about.
pub fn parse_repos(input: &str) -> Result<Vec<RepoEntry>, XbpsError> {
    let mut repos = Vec::new();
    for (line_no, line) in body_lines(input) {
        let trimmed = line.trim_start();
        let (count_text, rest) = trimmed
            .split_once(char::is_whitespace)
            .ok_or_else(|| malformed(line_no, line, "expected a count and a URL"))?;
        let count = count_text
            .parse::<i64>()
            .map_err(|_| malformed(line_no, line, "repository count is not a number"))?;
        let (url, note) = split_first_field(rest)
            .ok_or_else(|| malformed(line_no, line, "missing repository URL"))?;
        let signed = match note {
            "" => false,
            "(RSA signed)" => true,
            _ => return Err(malformed(line_no, line, "unknown repository note")),
        };
        repos.push(RepoEntry {
            count,
            url: url.to_owned(),
            signed,
        });
    }
    if repos.is_empty() {
        return Err(XbpsError::Empty);
    }
    Ok(repos)
}

/// Parses `xbps-query -p <prop> <pkg>`: one raw property value, trimmed.
///
/// Empty output is an error: a property that exists always prints something,
/// so an empty answer means the package or the property does not exist.
pub fn parse_property(input: &str) -> Result<String, XbpsError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(XbpsError::Empty);
    }
    Ok(value.to_owned())
}

/// One-based `(line number, line)` pairs, with trailing blank lines dropped.
fn body_lines(input: &str) -> impl Iterator<Item = (usize, &str)> {
    input
        .trim_end()
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
}

/// Splits the first whitespace-delimited field from the rest and trims it.
///
/// The remainder keeps its internal spacing, which matters for descriptions.
/// Returns `None` when there is no field at all.
fn split_first_field(rest: &str) -> Option<(&str, &str)> {
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    match rest.find(char::is_whitespace) {
        Some(index) => Some((&rest[..index], rest[index..].trim())),
        None => Some((rest, "")),
    }
}

fn malformed(line: usize, text: &str, reason: &'static str) -> XbpsError {
    XbpsError::MalformedLine {
        line,
        text: text.to_owned(),
        reason,
    }
}

fn parse_size(line: usize, text: &str, field: &str) -> Result<u64, XbpsError> {
    field
        .parse::<u64>()
        .map_err(|_| malformed(line, text, "size is not a byte count"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = include_str!("../tests/fixtures/xbps/installed.txt");
    const UPDATES: &str = include_str!("../tests/fixtures/xbps/updates.txt");
    const SEARCH: &str = include_str!("../tests/fixtures/xbps/search.txt");
    const REPOS: &str = include_str!("../tests/fixtures/xbps/repos.txt");
    const PROPERTY: &str = include_str!("../tests/fixtures/xbps/property.txt");

    fn line_error(result: Result<impl std::fmt::Debug, XbpsError>) -> (usize, String) {
        match result {
            Err(XbpsError::MalformedLine { line, text, .. }) => (line, text),
            other => panic!("expected a malformed line, got {other:?}"),
        }
    }

    #[test]
    fn installed_happy_path() {
        let packages = parse_installed(INSTALLED).expect("fixture parses");
        assert_eq!(packages.len(), 6);
        let first = &packages[0];
        assert_eq!(first.name, "AppStream");
        assert_eq!(first.version, "1.1.2_1");
        assert_eq!(first.pkgver, "AppStream-1.1.2_1");
        assert_eq!(first.state, InstallState::Installed);
        assert_eq!(
            first.description,
            "Tools and libraries to work with AppStream metadata"
        );
    }

    #[test]
    fn installed_splits_tricky_package_names() {
        let packages = parse_installed(
            "ii lib32-glibc-2.41_1 32-bit glibc\n\
             ii python3-3.13.1_1 Python 3\n\
             ii linux6.18-headers-6.18.63_1 Headers\n\
             ii UEFITool-A76_1 UEFI firmware tool\n",
        )
        .expect("names with digits, hyphens and letter versions parse");
        let names: Vec<(&str, &str)> = packages
            .iter()
            .map(|p| (p.name.as_str(), p.version.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("lib32-glibc", "2.41_1"),
                ("python3", "3.13.1_1"),
                ("linux6.18-headers", "6.18.63_1"),
                ("UEFITool", "A76_1"),
            ]
        );
    }

    #[test]
    fn installed_accepts_unpacked_state_and_empty_description() {
        let packages = parse_installed("uu foo-1.0_1\n").expect("uu parses");
        assert_eq!(packages[0].state, InstallState::Unpacked);
        assert_eq!(packages[0].description, "");
    }

    #[test]
    fn installed_rejects_a_malformed_line() {
        let (line, text) = line_error(parse_installed(
            "ii good-1.0_1 Fine\n\
             ii broken-without-revision Description\n",
        ));
        assert_eq!(line, 2);
        assert_eq!(text, "ii broken-without-revision Description");
    }

    #[test]
    fn installed_rejects_an_unknown_state() {
        let (line, _) = line_error(parse_installed("ii good-1.0_1 Fine\nxx bad-1.0_1 Odd\n"));
        assert_eq!(line, 2);
    }

    #[test]
    fn installed_rejects_an_interior_blank_line() {
        let (line, _) = line_error(parse_installed("ii good-1.0_1 Fine\n\nii two-1.0_1 Fine\n"));
        assert_eq!(line, 2);
    }

    #[test]
    fn installed_allows_trailing_blank_lines() {
        let packages = parse_installed("ii good-1.0_1 Fine\n\n\n").expect("trailing blanks parse");
        assert_eq!(packages.len(), 1);
    }

    #[test]
    fn installed_rejects_empty_output() {
        assert_eq!(parse_installed(""), Err(XbpsError::Empty));
        assert_eq!(parse_installed("\n \n"), Err(XbpsError::Empty));
    }

    #[test]
    fn updates_happy_path() {
        let entries = parse_updates(UPDATES).expect("fixture parses");
        assert_eq!(entries.len(), 7);
        let chromium = &entries[0];
        assert_eq!(chromium.name, "chromium");
        assert_eq!(chromium.version, "157.0.8103.50_1");
        assert_eq!(chromium.action, XbpsAction::Update);
        assert_eq!(chromium.arch, "x86_64");
        assert_eq!(chromium.repository, "https://repo-de.voidlinux.org/current");
        assert_eq!(chromium.installed_size, 426_243_959);
        assert_eq!(chromium.download_size, 153_176_879);
    }

    #[test]
    fn updates_reads_every_documented_action() {
        let input = "a-1.0_1 install x86_64 https://r/current 1 2\n\
                     b-1.0_1 update x86_64 https://r/current 3 4\n\
                     c-1.0_1 remove x86_64 https://r/current 5 6\n\
                     d-1.0_1 configure x86_64 https://r/current 7 8\n\
                     e-1.0_1 reinstall x86_64 https://r/current 9 10\n";
        let actions: Vec<XbpsAction> = parse_updates(input)
            .expect("all actions parse")
            .iter()
            .map(|entry| entry.action)
            .collect();
        assert_eq!(
            actions,
            [
                XbpsAction::Install,
                XbpsAction::Update,
                XbpsAction::Remove,
                XbpsAction::Configure,
                XbpsAction::Reinstall,
            ]
        );
    }

    #[test]
    fn updates_rejects_a_short_line() {
        let (line, text) = line_error(parse_updates(
            "a-1.0_1 update x86_64 https://r/current 1 2\n\
             b-1.0_1 update x86_64 https://r/current\n",
        ));
        assert_eq!(line, 2);
        assert_eq!(text, "b-1.0_1 update x86_64 https://r/current");
    }

    #[test]
    fn updates_rejects_an_unknown_action_and_a_bad_size() {
        assert!(parse_updates("a-1.0_1 upgrade x86_64 https://r 1 2\n").is_err());
        assert!(parse_updates("a-1.0_1 install x86_64 https://r 1 2x\n").is_err());
    }

    #[test]
    fn updates_accept_empty_output() {
        assert_eq!(parse_updates(""), Ok(Vec::new()));
    }

    #[test]
    fn search_happy_path() {
        let hits = parse_search(SEARCH).expect("fixture parses");
        assert_eq!(hits.len(), 9);
        let first = &hits[0];
        assert_eq!(first.name, "apvlv");
        assert_eq!(first.version, "0.5.0_1");
        assert!(!first.installed);
        assert_eq!(first.description, "Vim-like PDF/DJVU/TXT viewer");
        let installed = hits.last().expect("fixture has an installed hit");
        assert_eq!(installed.name, "AppStream");
        assert!(installed.installed);
    }

    #[test]
    fn search_marks_installed_hits() {
        let hits = parse_search("[-] apvlv-0.5.0_1 Viewer\n[*] vim-9.2.1031_1 Editor\n")
            .expect("both tags parse");
        assert!(!hits[0].installed);
        assert!(hits[1].installed);
        assert_eq!(hits[1].name, "vim");
    }

    #[test]
    fn search_rejects_a_malformed_tag() {
        let (line, text) = line_error(parse_search("[-] good-1.0_1 Fine\n[iu] bad-1.0_1 Odd\n"));
        assert_eq!(line, 2);
        assert_eq!(text, "[iu] bad-1.0_1 Odd");
        // `ii` belongs to `xbps-query -l`, not to search output.
        assert!(parse_search("[ii] vim-9.2.1031_1 Editor\n").is_err());
    }

    #[test]
    fn search_rejects_a_pkgver_without_a_revision() {
        let (line, text) = line_error(parse_search("[-] good-1.0_1 Fine\n[-] bad-1.0 Odd\n"));
        assert_eq!(line, 2);
        assert_eq!(text, "[-] bad-1.0 Odd");
    }

    #[test]
    fn search_accepts_empty_output() {
        assert_eq!(parse_search(""), Ok(Vec::new()));
    }

    #[test]
    fn repos_happy_path() {
        let repos = parse_repos(REPOS).expect("fixture parses");
        assert_eq!(repos.len(), 5);
        assert_eq!(repos[0].count, 14_835);
        assert_eq!(repos[0].url, "https://repo-de.voidlinux.org/current");
        assert!(repos[0].signed);
        assert!(repos[0].available());
    }

    #[test]
    fn repos_reads_an_unavailable_repository_without_note() {
        let repos = parse_repos("-1 https://repo.example.invalid/current\n")
            .expect("an unavailable repository parses");
        assert_eq!(repos[0].count, -1);
        assert!(!repos[0].signed);
        assert!(!repos[0].available());
    }

    #[test]
    fn repos_rejects_a_bad_count_or_note() {
        let (line, text) = line_error(parse_repos(
            "10 https://r/current (RSA signed)\n\
             many https://r/current (RSA signed)\n",
        ));
        assert_eq!(line, 2);
        assert_eq!(text, "many https://r/current (RSA signed)");
        assert!(parse_repos("10 https://r/current (unsigned)\n").is_err());
    }

    #[test]
    fn repos_rejects_empty_output() {
        assert_eq!(parse_repos(""), Err(XbpsError::Empty));
    }

    #[test]
    fn property_returns_the_trimmed_value() {
        assert_eq!(parse_property(PROPERTY), Ok("AppStream-1.1.2_1".to_owned()));
        assert_eq!(parse_property("  1.2.3_1 \n"), Ok("1.2.3_1".to_owned()));
    }

    #[test]
    fn property_rejects_empty_output() {
        assert_eq!(parse_property(""), Err(XbpsError::Empty));
        assert_eq!(parse_property(" \n\t\n"), Err(XbpsError::Empty));
    }

    #[test]
    fn split_pkgver_rejects_a_missing_revision() {
        assert_eq!(split_pkgver("foo-1.0"), None);
        assert_eq!(split_pkgver("1.0_1"), None);
        assert_eq!(split_pkgver(""), None);
    }

    #[test]
    fn split_pkgver_handles_a_hyphenated_name() {
        // xbps splits at the last hyphen, so the name is `foo-1.0`.
        assert_eq!(split_pkgver("foo-1.0-rc1_1"), Some(("foo-1.0", "rc1_1")));
    }

    #[test]
    fn split_pkgver_handles_letter_versions() {
        assert_eq!(split_pkgver("UEFITool-A76_1"), Some(("UEFITool", "A76_1")));
        assert_eq!(split_pkgver("mksh-R59c_2"), Some(("mksh", "R59c_2")));
        assert_eq!(
            split_pkgver("libui-32bit-alpha4.1_2"),
            Some(("libui-32bit", "alpha4.1_2"))
        );
        assert_eq!(
            split_pkgver("vapoursynth-devel-32bit-R66_4"),
            Some(("vapoursynth-devel-32bit", "R66_4"))
        );
    }
}
