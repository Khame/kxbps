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
/// A count of `-1` means xbps could not read the index; it is not an empty
/// repository and not a success, which is why [`RepoEntry::available`] exists.
///
/// xbps 0.60.7 prints one of three shapes, taken from the binary's format
/// strings:
///
/// - `<count> <url> (RSA signed)` — the index is signed and verified with a
///   local key, so [`RepoEntry::signed`] is `true`;
/// - `<count> <url> (RSA maybe-signed)` — the index is signed but no local key
///   verifies it (only `-1` counts were seen in the wild), so `signed` is
///   `false`;
/// - `<count> <url>` — no note at all for an unsigned repository, `signed`
///   `false`.
///
/// Any other note is a new format the caller has to learn about, so it fails.
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
            "" | "(RSA maybe-signed)" => false,
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

/// Parses the pkgver-per-line listings: `xbps-query -m` (manual packages),
/// `-H` (held packages) and `-O` (orphans), returning the package names.
///
/// All three modes print one bare `name-version_revision` per line and nothing
/// else, so one parser serves them all. Empty output is a valid empty list: a
/// system with nothing held or orphaned prints nothing and still exits 0.
pub fn parse_pkgver_list(input: &str) -> Result<Vec<String>, XbpsError> {
    let mut names = Vec::new();
    for (line_no, line) in body_lines(input) {
        let (pkgver, rest) = split_first_field(line)
            .ok_or_else(|| malformed(line_no, line, "expected one package name per line"))?;
        if !rest.is_empty() {
            return Err(malformed(
                line_no,
                line,
                "expected one package name per line",
            ));
        }
        let (name, _) = split_pkgver(pkgver)
            .ok_or_else(|| malformed(line_no, line, "cannot split package name and version"))?;
        names.push(name.to_owned());
    }
    Ok(names)
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

/// Parses `xbps-query -p <prop>[,<prop>…] <pkg>`: one raw value per line, in
/// the requested order, keyed by property name.
///
/// The property list is the comma-separated `--property` value, so the caller
/// asks for exactly the facts it renders (`pkgver,short_desc,homepage,license,
/// installed_size,repository,state`) and reads the answer back by name. A line
/// that is not there means the property was not reported, and the field stays
/// empty rather than being invented; the count must still match the request,
/// because an xbps answer that drops or repeats a value is a format change.
///
/// Empty output is an error: a package that exists has its properties printed,
/// so an empty answer means the package or the property does not exist.
pub fn parse_properties(
    input: &str,
    properties: &[&str],
) -> Result<std::collections::HashMap<String, String>, XbpsError> {
    let values: Vec<String> = body_lines(input)
        .map(|(line_no, line)| {
            if line.trim().is_empty() {
                return Err(XbpsError::MalformedLine {
                    line: line_no,
                    text: line.to_owned(),
                    reason: "expected one value per requested property",
                });
            }
            Ok(line.trim().to_owned())
        })
        .collect::<Result<Vec<_>, XbpsError>>()?;
    if values.is_empty() {
        return Err(XbpsError::Empty);
    }
    if values.len() != properties.len() {
        let last = values.last().expect("non-empty");
        return Err(XbpsError::MalformedLine {
            line: values.len(),
            text: last.clone(),
            reason: "expected one value per requested property",
        });
    }
    Ok(properties
        .iter()
        .zip(values.iter())
        .map(|(property, value)| ((*property).to_owned(), value.clone()))
        .collect())
}

/// Parses `xbps-query -x <pkg>`: one dependency per line, as
/// `<name>[<op> <version>]`.
///
/// Empty output is a valid empty list: a leaf package has no dependencies and
/// xbps still exits 0. The version constraint travels with the name, so the
/// answer is the raw lines — the UI joins them for display and takes the names
/// for any graph work.
pub fn parse_depends(input: &str) -> Result<Vec<String>, XbpsError> {
    let mut depends = Vec::new();
    for (line_no, line) in body_lines(input) {
        let dependency = line.trim();
        if dependency.is_empty() {
            return Err(malformed(line_no, line, "expected one dependency per line"));
        }
        depends.push(dependency.to_owned());
    }
    Ok(depends)
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

/// One size from a bulk `xbps-query -p <property> -s ''` listing.
///
/// The line reads `<pkgver>: <size>`, with ` (<repository>)` appended in
/// repository mode. The size is humanised by xbps (B/KB/MB, base 1024), so
/// [`SizeEntry::bytes`] is the best reconstruction, not the original byte
/// count; the dry-run table is the exact source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeEntry {
    /// Package name without the version.
    pub name: String,
    /// Version and revision the size belongs to.
    pub version: String,
    /// Size in bytes, as reconstructed from xbps' humanised field.
    pub bytes: u64,
    /// Repository URL when the line came from a `-R` take.
    pub repository: Option<String>,
}

/// Parses `xbps-query [-R] -p <size-property> -s ''`.
///
/// One package per line: `<pkgver>: <size> [ (<repository>)]`. An empty answer
/// is valid — a repository index that has never been synced prints nothing —
/// so it returns an empty list instead of an error.
pub fn parse_size_list(input: &str) -> Result<Vec<SizeEntry>, XbpsError> {
    let mut entries = Vec::new();
    for (line_no, line) in body_lines(input) {
        let (pkgver, rest) = line
            .split_once(": ")
            .ok_or_else(|| malformed(line_no, line, "expected `<pkgver>: <size>`"))?;
        let (name, version) = split_pkgver(pkgver.trim())
            .ok_or_else(|| malformed(line_no, line, "cannot split package name and version"))?;
        let (size_text, repository) = match rest.split_once(" (") {
            Some((size, repo)) => (size.trim(), Some(repo.trim_end_matches(')').to_owned())),
            None => (rest.trim(), None),
        };
        let bytes = parse_human_size(size_text)
            .ok_or_else(|| malformed(line_no, line, "size is not a byte count"))?;
        entries.push(SizeEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            bytes,
            repository,
        });
    }
    Ok(entries)
}

/// Reconstructs a byte count from xbps' humanised size (`215KB`, `9.8MB`).
///
/// xbps uses base 1024 and prints `B`, `KB` or `MB`; a value below ten keeps
/// one decimal, everything else is an integer, so the result is the printed
/// value rounded to bytes and never the exact original size.
pub fn parse_human_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    let multiplier: u64 = match unit.trim() {
        "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier as f64) as u64)
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
    const REPOS_0607: &str = include_str!("../tests/fixtures/xbps/repos-0.60.7.txt");
    const PROPERTY: &str = include_str!("../tests/fixtures/xbps/property.txt");
    const MANUAL: &str = include_str!("../tests/fixtures/xbps/manual.txt");

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
    fn repos_accepts_the_notes_xbps_0607_prints() {
        let repos = parse_repos(REPOS_0607).expect("the 0.60.7 output parses");
        assert_eq!(repos.len(), 5);
        assert_eq!(repos[0].url, "https://repo-fi.voidlinux.org/current");
        assert_eq!(repos[4].url, "https://repo-de.voidlinux.org/current");
        assert_eq!(repos[4].count, 14_836);
        assert!(repos[4].signed);
    }

    #[test]
    fn maybe_signed_repository_parses_without_a_local_key() {
        let repos = parse_repos("   -1 https://repo-fi.voidlinux.org/current (RSA maybe-signed)\n")
            .expect("a maybe-signed repository parses");
        assert_eq!(repos[0].count, -1);
        assert_eq!(repos[0].url, "https://repo-fi.voidlinux.org/current");
        assert!(
            !repos[0].signed,
            "maybe-signed is not verified locally, so it is not signed"
        );
    }

    #[test]
    fn a_minus_one_count_is_not_an_available_repository() {
        let repos = parse_repos(REPOS_0607).expect("the 0.60.7 output parses");
        assert_eq!(repos[0].count, -1);
        assert!(!repos[0].available(), "-1 means no index");
        assert!(repos[1].available());
    }

    #[test]
    fn an_unsigned_repository_without_a_note_parses() {
        let repos = parse_repos("10 https://repo.example.invalid/current\n")
            .expect("an unsigned repository without a note parses");
        assert_eq!(repos[0].count, 10);
        assert_eq!(repos[0].url, "https://repo.example.invalid/current");
        assert!(!repos[0].signed);
        assert!(repos[0].available());
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
    fn properties_map_one_value_per_requested_property_in_order() {
        let properties = [
            "pkgver",
            "short_desc",
            "homepage",
            "license",
            "installed_size",
            "repository",
            "state",
        ];
        let input = "bash-5.3_2\nGNU Bourne Again Shell\nhttps://www.gnu.org\nGPL-2.0-or-later\n7809KB\nhttps://r/current\ninstalled\n";
        let map = parse_properties(input, &properties).expect("the answer maps");
        assert_eq!(map["pkgver"], "bash-5.3_2");
        assert_eq!(map["short_desc"], "GNU Bourne Again Shell");
        assert_eq!(map["homepage"], "https://www.gnu.org");
        assert_eq!(map["license"], "GPL-2.0-or-later");
        assert_eq!(map["installed_size"], "7809KB");
        assert_eq!(map["repository"], "https://r/current");
        assert_eq!(map["state"], "installed");
    }

    #[test]
    fn properties_trim_each_value() {
        let properties = ["pkgver", "short_desc"];
        let map = parse_properties(" vim-9.2.1031_1 \n Vim editor \n", &properties).expect("trims");
        assert_eq!(map["pkgver"], "vim-9.2.1031_1");
        assert_eq!(map["short_desc"], "Vim editor");
    }

    #[test]
    fn properties_reject_empty_output() {
        let properties = ["pkgver", "short_desc"];
        assert_eq!(parse_properties("", &properties), Err(XbpsError::Empty));
    }

    #[test]
    fn properties_reject_a_wrong_value_count() {
        let properties = ["pkgver", "short_desc", "homepage"];
        let error = parse_properties("bash-5.3_2\nGNU Bourne Again Shell\n", &properties)
            .expect_err("three properties need three values");
        assert!(matches!(error, XbpsError::MalformedLine { .. }), "{error}");
    }

    #[test]
    fn properties_reject_an_interior_blank_line() {
        let properties = ["pkgver", "short_desc"];
        assert!(parse_properties("bash-5.3_2\n\nGNU Bourne Again Shell\n", &properties).is_err());
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

    #[test]
    fn pkgver_list_reads_the_manual_fixture() {
        let names = parse_pkgver_list(MANUAL).expect("fixture parses");
        assert_eq!(
            names,
            [
                "NetworkManager".to_owned(),
                "Signal-Desktop".to_owned(),
                "alsa-plugins-pulseaudio".to_owned(),
            ]
        );
    }

    #[test]
    fn pkgver_list_accepts_empty_output() {
        // Nothing held, nothing orphaned and nothing installed manually are
        // all real answers; the commands exit 0 with no output then.
        assert_eq!(parse_pkgver_list(""), Ok(Vec::new()));
        assert_eq!(parse_pkgver_list("\n \n"), Ok(Vec::new()));
    }

    #[test]
    fn pkgver_list_rejects_a_pkgver_without_a_revision() {
        let (line, text) = line_error(parse_pkgver_list(
            "good-1.0_1\n\
             broken-no-revision\n",
        ));
        assert_eq!(line, 2);
        assert_eq!(text, "broken-no-revision");
    }

    #[test]
    fn pkgver_list_rejects_extra_fields_on_a_line() {
        let (line, _) = line_error(parse_pkgver_list("good-1.0_1\nbad-1.0_1 unexpected\n"));
        assert_eq!(line, 2);
        assert!(
            parse_pkgver_list("bad 1.0_1\n").is_err(),
            "spaces are not part of a pkgver"
        );
    }

    #[test]
    fn pkgver_list_allows_trailing_blank_lines_only() {
        let names = parse_pkgver_list("good-1.0_1\n\n\n").expect("trailing blanks parse");
        assert_eq!(names, vec!["good".to_owned()]);
        let (line, _) = line_error(parse_pkgver_list("good-1.0_1\n\nother-1.0_1\n"));
        assert_eq!(line, 2, "an interior blank line is a format change");
    }

    #[test]
    fn depends_reads_one_line_per_dependency() {
        let depends = parse_depends("glibc>=2.41_1\nbash>=5.0\n").expect("dependencies parse");
        assert_eq!(depends, vec!["glibc>=2.41_1", "bash>=5.0"]);
    }

    #[test]
    fn depends_accepts_empty_output() {
        assert_eq!(parse_depends(""), Ok(Vec::new()));
    }

    #[test]
    fn depends_rejects_an_interior_blank_line() {
        let (line, text) = line_error(parse_depends("glibc>=2.41_1\n\nbash>=5.0\n"));
        assert_eq!(line, 2);
        assert_eq!(text, "");
    }

    #[test]
    fn human_sizes_use_base_1024_and_keep_the_printed_precision() {
        assert_eq!(parse_human_size("215KB"), Some(220_160));
        assert_eq!(parse_human_size("9.8MB"), Some(10_276_044));
        assert_eq!(parse_human_size("762B"), Some(762));
        assert_eq!(parse_human_size("2MB"), Some(2_097_152));
        assert_eq!(parse_human_size("nonsense"), None);
        assert_eq!(parse_human_size(""), None);
    }

    #[test]
    fn size_list_reads_installed_and_repository_takes() {
        let installed = parse_size_list("hello-2.12.3_1: 215KB\nzramen-0.2.4_1: 9.8MB\n")
            .expect("installed sizes parse");
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].name, "hello");
        assert_eq!(installed[0].version, "2.12.3_1");
        assert_eq!(installed[0].bytes, 220_160);
        assert_eq!(installed[0].repository, None);

        let repository =
            parse_size_list("0ad-0.28.0_1: 4381KB (https://repo-de.voidlinux.org/current)\n")
                .expect("repository sizes parse");
        assert_eq!(repository[0].name, "0ad");
        assert_eq!(repository[0].bytes, 4_486_144);
        assert_eq!(
            repository[0].repository.as_deref(),
            Some("https://repo-de.voidlinux.org/current")
        );
    }

    #[test]
    fn size_list_parses_the_four_digit_megabytes_the_repository_printer_emits() {
        // The real `-R` take prints past 1024 MB without switching unit
        // (`0ad-data-1: 1633MB`), so a GB-only parser would reject it and take
        // the whole repository size list down with it.
        let entries =
            parse_size_list("0ad-data-0.28.0_1: 1633MB (https://repo-de.voidlinux.org/current)\n")
                .expect("a four-digit megabyte value parses");
        assert_eq!(entries[0].bytes, 1_712_324_608);
    }

    #[test]
    fn size_list_accepts_an_empty_answer() {
        assert_eq!(parse_size_list(""), Ok(Vec::new()));
    }

    #[test]
    fn size_list_rejects_a_line_without_a_size() {
        let (line, _) = line_error(parse_size_list("hello-2.12.3_1\n"));
        assert_eq!(line, 1);
    }
}
