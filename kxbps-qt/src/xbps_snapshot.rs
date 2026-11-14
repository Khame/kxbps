//! One consistent take of the system state, read through the xbps runner.
//!
//! Five read-only commands answer in one go — `xbps-query -l` (installed
//! packages), `xbps-install -un` (pending updates), and `xbps-query -m`, `-H`
//! and `-O` (explicit, held and orphaned names) — plus three bulk size takes
//! (`-p installed_size -s ''` and two `-R` takes), and every line is parsed by
//! `kxbps-core`. Nothing here touches Qt: the transport is the `Invoke`
//! closure, so production installs the C++ shim and the tests install a fake.
//!
//! A snapshot is all or nothing. A list that is half real would be the one lie
//! this layer must never tell, so one failing command fails the whole take and
//! the caller shows why instead of an empty window.
//!
//! The sizes are the exception, on purpose: they are decoration next to the
//! package list, and a repository index that was never synced prints nothing or
//! fails without meaning the installed set is unknown. A failed size take logs
//! its reason and leaves its list empty, so a system that only wants to list
//! what it has keeps working.

use kxbps_core::{
    InstalledPackage, SearchHit, SizeEntry, UpdateEntry, XbpsError, parse_depends, parse_installed,
    parse_pkgver_list, parse_properties, parse_search, parse_size_list, parse_updates,
    split_pkgver,
};

use crate::xbps_runner::{
    READ_TIMEOUT_MS, REMOTE_TIMEOUT_MS, XPBS_QUERY, XbpsOutput, depends_argv, held_argv,
    installed_argv, installed_search_argv, installed_sizes_argv, manual_argv, orphans_argv,
    property_argv, property_repo_argv, repo_filename_sizes_argv, repo_installed_sizes_argv,
    required_by_argv, run, search_argv, updates_argv, updates_remote_argv,
};

/// The injectable process transport: the encoded argv and the timeout go in,
/// the shim's JSON reply comes out.
///
/// The backend stores one of these so every test can answer canned replies
/// instead of running xbps; production installs the closure
/// [`crate::production_invoke`] returns, which calls `kxbps_run_xbps`.
pub type Invoke = dyn Fn(&str, u32) -> String + Send + Sync;

/// Everything the five read commands answered in one consistent take.
///
/// This is the wire format of the package model's state, so it serialises in
/// camelCase like every snapshot that crosses a C++ shim.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// `xbps-query -l`, sorted by package name.
    pub installed: Vec<InstalledPackage>,
    /// `xbps-install -un`, sorted by package name.
    pub updates: Vec<UpdateEntry>,
    /// Package names from `xbps-query -m`, sorted and deduplicated.
    pub explicit: Vec<String>,
    /// Package names from `xbps-query -H`, sorted and deduplicated.
    pub held: Vec<String>,
    /// Package names from `xbps-query -O`, sorted and deduplicated.
    pub orphans: Vec<String>,
    /// `xbps-query -p installed_size -s ''`: the installed size of every
    /// installed package, sorted by name.
    pub installed_sizes: Vec<SizeEntry>,
    /// `xbps-query -R -p installed_size -s ''`: the unpacked size of every
    /// package the repository indexes offer, sorted by name.
    pub repo_installed_sizes: Vec<SizeEntry>,
    /// `xbps-query -R -p filename-size -s ''`: the archive size of every
    /// package the repository indexes offer, sorted by name.
    pub repo_download_sizes: Vec<SizeEntry>,
}

impl Snapshot {
    /// The version the pending update of `name` would install, if any.
    pub fn update_of(&self, name: &str) -> Option<&str> {
        self.updates
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.version.as_str())
    }

    /// The installed size of `name`, reconstructed from xbps' humanised field.
    pub fn installed_size_of(&self, name: &str) -> Option<u64> {
        size_of(&self.installed_sizes, name)
    }

    /// The unpacked size the repositories report for `name`.
    pub fn repo_installed_size_of(&self, name: &str) -> Option<u64> {
        size_of(&self.repo_installed_sizes, name)
    }

    /// The archive size the repositories report for `name`.
    pub fn repo_download_size_of(&self, name: &str) -> Option<u64> {
        size_of(&self.repo_download_sizes, name)
    }
}

/// The byte count one size list reports for `name`.
fn size_of(entries: &[SizeEntry], name: &str) -> Option<u64> {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.bytes)
}

/// Why a snapshot could not be taken, as an i18n key plus its arguments.
///
/// The key travels to QML, which owns the translated sentence; the arguments
/// are the values its positional `%1…%3` stand-ins need. Command output stays
/// untranslated on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The process could not be started: xbps is missing or not executable.
    Start { program: String, reason: String },
    /// The process did not finish within the read timeout.
    Timeout { program: String },
    /// The process exited non-zero.
    Exit {
        program: String,
        code: i32,
        stderr: String,
    },
    /// A line of output did not match the documented format.
    Output {
        program: String,
        line: usize,
        text: String,
    },
    /// A fresh check ran but at least one repository could not be read.
    ///
    /// `xbps-install -M -un` exits 0 for this, so the exit code cannot say it:
    /// the stderr lines do. `failed` of `total` repositories failed and `detail`
    /// is the first line verbatim, untranslated.
    Repositories {
        program: String,
        failed: usize,
        total: usize,
        detail: String,
    },
    /// The command printed nothing where output was expected.
    Empty { program: String },
    /// The shim's reply could not be decoded; an internal error.
    Reply { reason: String },
}

impl SnapshotError {
    /// The i18n key QML knows; `PackagesView.qml` maps it to a sentence and
    /// every other key reads the same way there.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Start { .. } => "errorXbpsStart",
            Self::Timeout { .. } => "errorXbpsTimeout",
            Self::Exit { .. } => "errorXbpsExit",
            Self::Output { .. } => "errorXbpsOutput",
            Self::Repositories { .. } => "checkRepositoriesFailed",
            Self::Empty { .. } => "errorXbpsEmpty",
            Self::Reply { .. } => "errorXbpsReply",
        }
    }

    /// The arguments of the translated sentence, in `%1…` order.
    ///
    /// `Start` and `Timeout` carry the program and never a path, so QML can
    /// name the command without knowing where the system keeps it.
    pub fn args(&self) -> Vec<String> {
        match self {
            // A failed start reports its reason in the stderr channel; a
            // timeout that was killed after printing its own error reports it
            // there as well.
            Self::Start { program, reason } => vec![program.clone(), reason.clone()],
            Self::Timeout { program } => {
                vec![program.clone(), (READ_TIMEOUT_MS / 1000).to_string()]
            }
            Self::Exit {
                program,
                code,
                stderr,
            } => vec![program.clone(), code.to_string(), stderr.clone()],
            Self::Output {
                program,
                line,
                text,
            } => vec![program.clone(), line.to_string(), text.clone()],
            Self::Repositories {
                failed,
                total,
                detail,
                ..
            } => vec![failed.to_string(), total.to_string(), detail.clone()],
            Self::Empty { program } => vec![program.clone()],
            Self::Reply { reason } => vec![reason.clone()],
        }
    }
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start { program, reason } => write!(f, "could not start {program}: {reason}"),
            Self::Timeout { program } => write!(f, "{program} timed out"),
            Self::Exit {
                program,
                code,
                stderr,
            } => write!(f, "{program} exited with {code}: {stderr}"),
            Self::Output {
                program,
                line,
                text,
            } => write!(f, "{program} printed an unparsable line {line}: {text:?}"),
            Self::Repositories {
                program,
                failed,
                total,
                detail,
            } => write!(
                f,
                "{program} could not read {failed} of {total} repositories: {detail}"
            ),
            Self::Empty { program } => write!(f, "{program} printed no output"),
            Self::Reply { reason } => write!(f, "invalid runner reply: {reason}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Runs one command through `invoke` and parses its stdout.
///
/// Everything that is not a clean exit with parsable output becomes a
/// [`SnapshotError`] the caller can show. A line the parser rejects also goes
/// to stderr as a log line: the user sees the translated sentence, and whoever
/// reads the terminal sees the exact text xbps printed.
pub(crate) fn run_parsed<T>(
    argv: &[String],
    parse: fn(&str) -> Result<T, XbpsError>,
    invoke: &Invoke,
) -> Result<T, SnapshotError> {
    let program = argv.first().cloned().unwrap_or_default();
    let output = run(argv, READ_TIMEOUT_MS, invoke).map_err(|err| SnapshotError::Reply {
        reason: err.to_string(),
    })?;
    // A timeout is checked first: the shim reports `-1` for both, and only
    // `timedOut` says which one happened.
    if output.timed_out {
        return Err(SnapshotError::Timeout { program });
    }
    if output.exit_code == -1 {
        return Err(SnapshotError::Start {
            program,
            reason: output.stderr.trim().to_owned(),
        });
    }
    if output.exit_code != 0 {
        return Err(SnapshotError::Exit {
            program,
            code: output.exit_code,
            stderr: output.stderr.trim().to_owned(),
        });
    }
    parse(&output.stdout).map_err(|err| match err {
        XbpsError::MalformedLine { line, text, .. } => {
            eprintln!("kxbps: {program} line {line} was not understood: {text}");
            SnapshotError::Output {
                program,
                line,
                text,
            }
        }
        XbpsError::Empty => SnapshotError::Empty { program },
    })
}

/// Reads the pending updates against freshly fetched repository indexes.
///
/// `xbps-install -M -un` downloads the indexes into memory, so this is the only
/// read that can notice an update published since the last sync. It exits 0
/// even when a repository fails, so the decision comes from stderr: any line
/// starting with `ERROR` is one unreadable repository, and no list is trusted
/// when even one is missing. The first such line is logged verbatim and carried
/// in the error, because xbps' own wording is what a bug report needs.
///
/// `total` is the number of configured repositories, which only `xbps-query -L`
/// knows; the caller reads it, so the sentence can name both numbers.
/// One command's raw output, in the shape the log drawer shows: the command
/// line first, then what xbps printed on each stream.
///
/// The two streams are appended rather than interleaved: the shim collects
/// them separately, and inventing an order would misreport what xbps did.
pub fn command_lines(argv: &[String], output: &XbpsOutput) -> Vec<String> {
    let mut lines = vec![format!("$ {}", argv.join(" "))];
    lines.extend(output.stdout.lines().map(str::to_owned));
    lines.extend(output.stderr.lines().map(str::to_owned));
    lines
}

/// The answer of a fresh check, plus the lines it printed.
pub struct RemoteUpdates {
    /// What the check found, or why it failed.
    pub result: Result<Vec<UpdateEntry>, SnapshotError>,
    /// The command and its output, for the log drawer.
    pub lines: Vec<String>,
}

pub fn read_remote_updates(
    invoke: &Invoke,
    total: usize,
) -> Result<Vec<UpdateEntry>, SnapshotError> {
    read_remote_updates_logged(invoke, total).result
}

/// [`read_remote_updates`], plus the command and its output.
///
/// The drawer shows what xbps printed, and a check is an xbps run like any
/// other, so the lines travel with the answer instead of being dropped.
pub fn read_remote_updates_logged(invoke: &Invoke, total: usize) -> RemoteUpdates {
    let argv = updates_remote_argv();
    let output = match run(&argv, REMOTE_TIMEOUT_MS, invoke) {
        Ok(output) => output,
        Err(err) => {
            return RemoteUpdates {
                result: Err(SnapshotError::Reply {
                    reason: err.to_string(),
                }),
                lines: vec![format!("$ {}", argv.join(" "))],
            };
        }
    };
    let lines = command_lines(&argv, &output);
    RemoteUpdates {
        result: judge_remote(&argv, &output, total),
        lines,
    }
}

/// Turns one `xbps-install -M -un` answer into the pending list or an error.
fn judge_remote(
    argv: &[String],
    output: &XbpsOutput,
    total: usize,
) -> Result<Vec<UpdateEntry>, SnapshotError> {
    let program = argv[0].clone();
    if output.timed_out {
        return Err(SnapshotError::Timeout { program });
    }
    if output.exit_code == -1 {
        return Err(SnapshotError::Start {
            program,
            reason: output.stderr.trim().to_owned(),
        });
    }
    let errors: Vec<&str> = output
        .stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("ERROR"))
        .collect();
    if let Some(first) = errors.first() {
        eprintln!("kxbps: {program} -M: {first}");
        // One error line per repository. `total` comes from `xbps-query -L`,
        // which may itself have failed; never report more failures than the
        // total the sentence claims.
        let failed = errors.len();
        let total = total.max(failed);
        return Err(SnapshotError::Repositories {
            program,
            failed,
            total,
            detail: (*first).to_owned(),
        });
    }
    if output.exit_code != 0 {
        return Err(SnapshotError::Exit {
            program,
            code: output.exit_code,
            stderr: output.stderr.trim().to_owned(),
        });
    }
    if !output.stderr.trim().is_empty() {
        // Not a failure, but xbps said something; the log keeps it for the
        // report without turning it into a sentence.
        eprintln!("kxbps: {program} -M: {}", output.stderr.trim());
    }
    parse_updates(&output.stdout).map_err(|err| match err {
        XbpsError::MalformedLine { line, text, .. } => {
            eprintln!("kxbps: {program} -M line {line} was not understood: {text}");
            SnapshotError::Output {
                program,
                line,
                text,
            }
        }
        XbpsError::Empty => SnapshotError::Empty { program },
    })
}

/// Reads the whole system snapshot through `invoke`:
/// `xbps-query -l`, `xbps-install -un`, `xbps-query -m`, `-H` and `-O`, plus
/// the three bulk size takes.
///
/// The package lists are all or nothing: any failing command fails the
/// snapshot, because a list that is half real is the one lie this module must
/// never tell. The size takes are the exception ([`read_sizes`]): a failed one
/// leaves its list empty and logs why, so a repository index that was never
/// synced cannot cost the user the installed list.
///
/// The result is sorted, so the wire format is stable and the model can diff
/// it.
pub fn read_snapshot(invoke: &Invoke) -> Result<Snapshot, SnapshotError> {
    let mut installed = run_parsed(&installed_argv(), parse_installed, invoke)?;
    let mut updates = run_parsed(&updates_argv(), parse_updates, invoke)?;
    let explicit = run_parsed(&manual_argv(), parse_pkgver_list, invoke)?;
    let held = run_parsed(&held_argv(), parse_pkgver_list, invoke)?;
    let orphans = run_parsed(&orphans_argv(), parse_pkgver_list, invoke)?;
    let installed_sizes = read_sizes(&installed_sizes_argv(), invoke);
    let repo_installed_sizes = read_sizes(&repo_installed_sizes_argv(), invoke);
    let repo_download_sizes = read_sizes(&repo_filename_sizes_argv(), invoke);

    installed.sort_by(|left, right| left.name.cmp(&right.name));
    updates.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Snapshot {
        installed,
        updates,
        explicit: sorted_names(explicit),
        held: sorted_names(held),
        orphans: sorted_names(orphans),
        installed_sizes: sorted_sizes(installed_sizes),
        repo_installed_sizes: sorted_sizes(repo_installed_sizes),
        repo_download_sizes: sorted_sizes(repo_download_sizes),
    })
}

/// Runs one bulk size take, answering with an empty list when it fails.
///
/// A size is decoration next to the package list, so a take that cannot start,
/// times out, exits non-zero or prints a line this version does not understand
/// is logged and dropped instead of failing the snapshot. An empty answer is
/// not a failure: `-s ''` over a repository index that was never synced prints
/// nothing and exits 0.
fn read_sizes(argv: &[String], invoke: &Invoke) -> Vec<SizeEntry> {
    match run_parsed(argv, parse_size_list, invoke) {
        Ok(entries) => entries,
        Err(err) => {
            eprintln!("kxbps: sizes unavailable: {err}");
            Vec::new()
        }
    }
}

/// Sorted and deduplicated by name, so a repeated line cannot double a size.
fn sorted_sizes(mut entries: Vec<SizeEntry>) -> Vec<SizeEntry> {
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries.dedup_by(|left, right| left.name == right.name);
    entries
}

/// Sorted and deduplicated, so a repeated answer cannot double a count.
fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

/// Which database a query searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSource {
    /// `xbps-query -Rs <pattern>`: the repository pool.
    Repository,
    /// `xbps-query -s <pattern>`: the installed packages only.
    Installed,
}

impl SearchSource {
    fn argv(self, pattern: &str) -> Vec<String> {
        match self {
            Self::Repository => search_argv(pattern),
            Self::Installed => installed_search_argv(pattern),
        }
    }
}

/// Reads the hits of one pattern through `invoke`.
///
/// Empty output is a valid empty answer — a pattern that matches nothing — so
/// the search path answers honestly instead of failing.
pub fn read_search(
    invoke: &Invoke,
    source: SearchSource,
    pattern: &str,
) -> Result<Vec<SearchHit>, SnapshotError> {
    run_parsed(&source.argv(pattern), parse_search, invoke)
}

/// Runs one `xbps-query` take and answers with its output verbatim.
///
/// The caller builds the argv: the Query page runs takes that are not a package
/// list (`-x`, `-X`, `-f`, `-o`, `-p`), and a row invented from their lines
/// would misrepresent what they print. The text is shown next to the command it
/// belongs to; a failing take is still a translated key, so it reads like every
/// other failure.
pub fn read_raw(invoke: &Invoke, command: &[String]) -> Result<RawOutput, SnapshotError> {
    let Some(program) = command.first().cloned() else {
        return Err(SnapshotError::Start {
            program: XPBS_QUERY.to_owned(),
            reason: "no command".to_owned(),
        });
    };
    let output = run(command, READ_TIMEOUT_MS, invoke).map_err(|err| SnapshotError::Reply {
        reason: err.to_string(),
    })?;
    if output.timed_out {
        return Err(SnapshotError::Timeout { program });
    }
    if output.exit_code == -1 {
        return Err(SnapshotError::Start {
            program,
            reason: output.stderr.trim().to_owned(),
        });
    }
    if output.exit_code != 0 {
        return Err(SnapshotError::Exit {
            program,
            code: output.exit_code,
            stderr: output.stderr.trim().to_owned(),
        });
    }
    Ok(RawOutput {
        command: command.join(" "),
        text: output.stdout,
    })
}

/// What a take that is not a package list printed, and the command that
/// printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawOutput {
    /// The argv that produced `text`, for the label above the block.
    pub command: String,
    /// Standard output, verbatim.
    pub text: String,
}

/// The property list of the plain answer: what the details pane shows for a
/// package that is installed.
const FACT_PROPERTIES: &[&str] = &[
    "pkgver",
    "short_desc",
    "homepage",
    "license",
    "installed_size",
    "repository",
    "state",
];

/// The property list of the repository-mode answer, which also carries the
/// download size the plain take cannot, and answers for a package that is not
/// installed.
const DOWNLOAD_PROPERTIES: &[&str] = &[
    "pkgver",
    "short_desc",
    "homepage",
    "license",
    // The unpacked size of the version in the index: the plain take only has
    // it for an installed package, and a package page wants the number before
    // the install as well.
    "installed_size",
    "filename-size",
    "repository",
];

/// What one package reports, from the `-p` takes and the two dependency
/// lists. Every field is exactly what xbps printed; a take that could not
/// answer leaves its fields empty rather than inventing them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageFacts {
    pub installed: bool,
    /// The raw `name-version_revision` the property take printed.
    pub pkgver: String,
    pub short_desc: String,
    pub homepage: String,
    pub license: String,
    /// The installed size, as xbps prints it; empty when not installed.
    pub installed_size: String,
    /// The repository the package comes from.
    pub repository: String,
    /// `installed` / `not installed`, derived from which take answered.
    pub state: String,
    /// The compressed download size, from the repository-mode take.
    pub filename_size: String,
    /// The dependencies, as the `-x` lines.
    pub depends: Vec<String>,
    /// The reverse dependencies, as the `-X` lines.
    pub required_by: Vec<String>,
}

impl PackageFacts {
    /// The version part of the pkgver a take printed.
    pub fn version(&self) -> &str {
        split_pkgver(&self.pkgver)
            .map(|(_, version)| version)
            .unwrap_or(&self.pkgver)
    }
}

/// Reads the facts of one package: the plain property take, the repository-mode
/// take, and the two dependency lists. One package at a time, on selection.
///
/// The takes are not all-or-nothing, because xbps 0.60.7 answers differently
/// depending on whether the package is installed:
///
/// - the plain take fails for a package that is not installed, and that is an
///   answer — the package is not installed — not an error;
/// - the dependency lists fail for any package the installed database does not
///   know, and then there are no dependencies to list;
/// - only when neither property take answers does the package not exist, and
///   that is the one case that fails.
///
/// A property take that *does* answer must parse whole: a short or garbled
/// answer is a format change and fails loudly.
pub fn read_package(
    invoke: &Invoke,
    name: &str,
    pkgver: &str,
) -> Result<PackageFacts, SnapshotError> {
    let plain_argv = property_argv(&FACT_PROPERTIES.join(","), pkgver);
    let download_argv = property_repo_argv(&DOWNLOAD_PROPERTIES.join(","), pkgver);
    let plain = optional_properties(invoke, &plain_argv, FACT_PROPERTIES)?;
    let download = optional_properties(invoke, &download_argv, DOWNLOAD_PROPERTIES)?;
    // The plain take failing means "not installed": an answer, as long as the
    // repository take knows the package. Only when the repository take fails
    // too does nobody know it.
    let download = match download {
        Some(download) => download,
        // Installed but the index cannot see it (a local package, a removed
        // repository): the installed record alone is a complete answer.
        None if plain.is_some() => std::collections::HashMap::new(),
        // Nobody knows it. Run the repository take once more so the user reads
        // the error xbps printed, not a sentence this code invented.
        None => {
            return run_parsed(&download_argv, parse_repository_properties, invoke).and_then(
                |_| {
                    Err(SnapshotError::Empty {
                        program: XPBS_QUERY.to_owned(),
                    })
                },
            );
        }
    };
    let plain = plain.unwrap_or_default();
    let installed = plain.get("state").is_some_and(|state| state == "installed");
    let depends = if installed {
        read_list(invoke, &depends_argv(name), parse_depends)
    } else {
        Vec::new()
    };
    let required_by = if installed {
        read_list(invoke, &required_by_argv(name), parse_pkgver_list)
    } else {
        Vec::new()
    };

    Ok(PackageFacts {
        installed,
        pkgver: preferred(&plain, &download, "pkgver"),
        // The repository take carries the definition of the package; the plain
        // take is the installed record and wins where it has something to say.
        short_desc: preferred(&plain, &download, "short_desc"),
        homepage: preferred(&plain, &download, "homepage"),
        license: preferred(&plain, &download, "license"),
        // The unpacked size: the installed record's when the package is
        // installed, otherwise the repository's, which is what installing it
        // would take on disk.
        installed_size: preferred(&plain, &download, "installed_size"),
        repository: preferred(&plain, &download, "repository"),
        state: if installed {
            "installed"
        } else {
            "not installed"
        }
        .to_owned(),
        filename_size: value_of(&download, "filename-size"),
        depends,
        required_by,
    })
}

/// One value of a property take, by name; a property that was not reported is
/// empty rather than invented.
fn value_of(facts: &std::collections::HashMap<String, String>, key: &str) -> String {
    facts.get(key).cloned().unwrap_or_default()
}

/// The plain value when it says something, the repository value otherwise.
fn preferred(
    plain: &std::collections::HashMap<String, String>,
    download: &std::collections::HashMap<String, String>,
    key: &str,
) -> String {
    let plain_value = value_of(plain, key);
    if plain_value.is_empty() {
        value_of(download, key)
    } else {
        plain_value
    }
}

/// A property take that may legitimately fail: `None` when xbps rejected the
/// package, the parsed map when it answered, and a loud error when the answer
/// arrived but could not be understood.
///
/// A timeout or a program that never started is still an error — those are not
/// answers about the package.
fn optional_properties(
    invoke: &Invoke,
    argv: &[String],
    properties: &[&str],
) -> Result<Option<std::collections::HashMap<String, String>>, SnapshotError> {
    let program = program_of(argv);
    let output = run(argv, READ_TIMEOUT_MS, invoke).map_err(|err| SnapshotError::Reply {
        reason: err.to_string(),
    })?;
    if output.timed_out {
        return Err(SnapshotError::Timeout { program });
    }
    if output.exit_code == -1 {
        return Err(SnapshotError::Start {
            program,
            reason: output.stderr.trim().to_owned(),
        });
    }
    if output.exit_code != 0 {
        // xbps rejected the package: "not installed" or "not found". Neither is
        // a broken answer; the caller decides what it means.
        return Ok(None);
    }
    parse_properties(&output.stdout, properties)
        .map(Some)
        .map_err(|err| output_error(argv, err))
}

/// A dependency list: empty when xbps rejects the package, because a package
/// the installed database does not know has nothing to list.
fn read_list<T>(invoke: &Invoke, argv: &[String], parse: fn(&str) -> Result<T, XbpsError>) -> T
where
    T: Default,
{
    let Ok(output) = run(argv, READ_TIMEOUT_MS, invoke) else {
        return T::default();
    };
    if output.timed_out || output.exit_code != 0 {
        return T::default();
    }
    parse(&output.stdout).unwrap_or_default()
}

/// The parse half of the repository take, for the one case that has to report
/// its error rather than treat it as "not installed".
fn parse_repository_properties(
    input: &str,
) -> Result<std::collections::HashMap<String, String>, XbpsError> {
    parse_properties(input, DOWNLOAD_PROPERTIES)
}

/// The program of an argv list; every builder starts with it.
fn program_of(argv: &[String]) -> String {
    argv.first().cloned().unwrap_or_default()
}

/// Turns a rejected parse into the error the user sees, and logs the line.
fn output_error(argv: &[String], err: XbpsError) -> SnapshotError {
    let program = program_of(argv);
    match err {
        XbpsError::MalformedLine { line, text, .. } => {
            eprintln!("kxbps: {program} line {line} was not understood: {text}");
            SnapshotError::Output {
                program,
                line,
                text,
            }
        }
        XbpsError::Empty => SnapshotError::Empty { program },
    }
}

/// The transport every started worker thread shares.
///
/// The backend hands one transport to each worker thread it spawns, so the
/// closure must be callable from several threads at once: it is stateless
/// (encode an argv, call the shim), which is why `Send + Sync` costs nothing.
/// Production installs the closure [`crate::production_invoke`] returns.
pub type SharedInvoke = std::sync::Arc<Invoke>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xbps_runner::repos_argv;

    /// A shim reply with the given stdout and a clean exit.
    fn ok_reply(stdout: &str) -> String {
        format!(
            r#"{{"exitCode":0,"stdout":{},"stderr":"","timedOut":false}}"#,
            serde_json::json!(stdout)
        )
    }

    /// A shim reply for a process that could not be started.
    const START_FAILURE: &str =
        r#"{"exitCode":-1,"stdout":"","stderr":"No such file or directory","timedOut":false}"#;

    /// A shim reply with a given exit code, stdout and stderr; the shape xbps
    /// uses when a repository fails but it still exits 0.
    fn reply_of(exit_code: i32, stdout: &str, stderr: &str, timed_out: bool) -> String {
        format!(
            r#"{{"exitCode":{exit_code},"stdout":{},"stderr":{},"timedOut":{timed_out}}}"#,
            serde_json::json!(stdout),
            serde_json::json!(stderr)
        )
    }

    /// The documented answer of a fresh check: one line per pending update.
    const REMOTE_ONE_UPDATE: &str = "vim-9.2.2000_1 update x86_64 https://r/current 10 20\n";

    /// A transport that answers every request with the reply.
    fn fixed(reply: String) -> impl Fn(&str, u32) -> String {
        move |_, _| reply.clone()
    }

    #[test]
    fn a_fresh_check_reads_the_update_lines() {
        let remote = read_remote_updates(&fixed(ok_reply(REMOTE_ONE_UPDATE)), 2)
            .expect("a clean answer lands");
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].name, "vim");
        assert_eq!(remote[0].version, "9.2.2000_1");
    }

    #[test]
    fn a_fresh_check_with_nothing_pending_is_an_empty_list() {
        let remote = read_remote_updates(&fixed(ok_reply("")), 2).expect("lands");
        assert!(remote.is_empty());
    }

    #[test]
    fn a_fresh_check_logs_the_command_and_what_xbps_printed() {
        let logged = read_remote_updates_logged(&fixed(ok_reply(REMOTE_ONE_UPDATE)), 2);
        assert!(logged.result.is_ok());
        assert_eq!(
            logged.lines,
            [
                "$ xbps-install -M -un".to_string(),
                "vim-9.2.2000_1 update x86_64 https://r/current 10 20".to_string(),
            ]
        );
    }

    #[test]
    fn a_failing_repository_keeps_its_error_line_in_the_log() {
        let reply = reply_of(
            0,
            "",
            "ERROR: failed to open repository: https://r/current: Input/output error\n",
            false,
        );
        let logged = read_remote_updates_logged(&fixed(reply), 1);
        assert!(logged.result.is_err());
        assert!(
            logged
                .lines
                .iter()
                .any(|line| line.contains("ERROR: failed to open repository")),
            "the error line xbps printed is what a bug report needs: {:?}",
            logged.lines
        );
    }

    #[test]
    fn a_failing_repository_fails_the_check_even_with_exit_zero() {
        // Measured on this machine: xbps prints the error and exits 0.
        let reply = reply_of(
            0,
            REMOTE_ONE_UPDATE,
            "ERROR: failed to open repository: https://r/current: Input/output error\n",
            false,
        );
        let error =
            read_remote_updates(&fixed(reply), 2).expect_err("a failed repo fails the read");
        match &error {
            SnapshotError::Repositories {
                failed,
                total,
                detail,
                ..
            } => {
                assert_eq!(
                    (*failed, *total),
                    (1, 2),
                    "one of the two repositories failed"
                );
                assert_eq!(
                    detail,
                    "ERROR: failed to open repository: https://r/current: Input/output error"
                );
            }
            other => panic!("expected the repository error, got {other:?}"),
        }
        assert_eq!(error.key(), "checkRepositoriesFailed");
    }

    #[test]
    fn every_repository_failing_reports_the_count() {
        let reply = reply_of(
            0,
            "",
            "ERROR: failed to open repository: https://a/current: Input/output error\n\
             ERROR: failed to open repository: https://b/current: Input/output error\n",
            false,
        );
        let error = read_remote_updates(&fixed(reply), 2).expect_err("both repos failed");
        match error {
            SnapshotError::Repositories { failed, total, .. } => {
                assert_eq!((failed, total), (2, 2));
            }
            other => panic!("expected the repository error, got {other:?}"),
        }
    }

    #[test]
    fn a_timed_out_check_is_reported_as_a_timeout() {
        let error = read_remote_updates(&fixed(reply_of(-1, "", "", true)), 2)
            .expect_err("a timeout fails the read");
        assert!(matches!(error, SnapshotError::Timeout { .. }));
    }

    #[test]
    fn an_unparsable_line_fails_the_check_and_is_logged() {
        let error = read_remote_updates(&fixed(ok_reply("this is not a plan line\n")), 2)
            .expect_err("garbage fails the read");
        assert!(matches!(error, SnapshotError::Output { line: 1, .. }));
    }

    #[test]
    fn silent_empty_output_is_reported_as_empty() {
        let error = read_remote_updates(&fixed(ok_reply("")), 0)
            .map(|updates| updates.len())
            .err();
        assert!(error.is_none(), "an empty list is a valid answer");
    }

    #[test]
    fn a_fresh_check_uses_the_remote_argv_and_its_own_timeout() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new((String::new(), 0)));
        let recorder = seen.clone();
        let invoke = move |request: &str, timeout: u32| {
            *recorder.lock().expect("lock") = (request.to_owned(), timeout);
            ok_reply("").to_owned()
        };
        read_remote_updates(&invoke, 5).expect("lands");
        let (request, timeout) = &*seen.lock().expect("lock");
        assert_eq!(request, r#"["xbps-install","-M","-un"]"#);
        assert_eq!(*timeout, REMOTE_TIMEOUT_MS);
    }

    /// The command an argv selects, as the fake's table spells it: the program
    /// is dropped and every leading flag kept, because `-R -p` and `-R` are
    /// different takes of the same program. For a `-p` take the property is
    /// part of the key too — `-R -p filename-size` and `-R -p installed_size`
    /// are different commands of the same shape. Patterns and pkgvers never
    /// enter the key.
    fn command_key(argv: &[String]) -> String {
        let mut key: Vec<&str> = Vec::new();
        let mut property_next = false;
        for arg in argv.iter().skip(1) {
            if !arg.starts_with('-') {
                if property_next {
                    key.push(arg);
                }
                break;
            }
            key.push(arg);
            property_next = arg == "-p";
        }
        key.join(" ")
    }

    /// A transport that answers each request by the command its argv selects,
    /// so one fake covers a whole read. A table entry without the property in
    /// its key matches any property take of that shape, which is what the
    /// single-package tests use.
    fn transport_by_mode(modes: Vec<(&str, String)>) -> impl Fn(&str, u32) -> String {
        move |request, _| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            let key = command_key(&argv);
            let flags = key
                .split(' ')
                .take_while(|part| part.starts_with('-'))
                .fold(String::new(), |mut flags, part| {
                    if !flags.is_empty() {
                        flags.push(' ');
                    }
                    flags.push_str(part);
                    flags
                });
            modes
                .iter()
                .find(|(flag, _)| *flag == key || *flag == flags)
                .map(|(_, reply)| reply.clone())
                .unwrap_or_else(|| panic!("unexpected command: {argv:?} (key {key:?})"))
        }
    }

    /// The canned answers of a healthy system: two installed packages, one
    /// pending update, one explicit package, nothing held, nothing orphaned.
    fn healthy_modes() -> Vec<(&'static str, String)> {
        vec![
            (
                "-l",
                ok_reply("ii vim-9.2.1031_1 Editor\nii ark-25.08.0_1 Archives\n"),
            ),
            (
                "-un",
                ok_reply("vim-9.2.2000_1 update x86_64 https://r/current 10 20\n"),
            ),
            ("-m", ok_reply("vim-9.2.1031_1\n")),
            ("-H", ok_reply("")),
            ("-O", ok_reply("")),
            (
                "-p installed_size",
                ok_reply("vim-9.2.1031_1: 2125KB\nark-25.08.0_1: 5MB\n"),
            ),
            (
                "-R -p filename-size",
                ok_reply(
                    "vim-9.2.2000_1: 1735KB (https://r/current)\n\
                     ark-25.08.0_1: 1600KB (https://r/current)\n",
                ),
            ),
            (
                "-R -p installed_size",
                ok_reply(
                    "vim-9.2.2000_1: 1800KB (https://r/current)\n\
                     ark-25.08.0_1: 4500KB (https://r/current)\n",
                ),
            ),
        ]
    }

    #[test]
    fn read_snapshot_assembles_and_sorts_the_five_answers() {
        let snapshot = read_snapshot(&|request, timeout| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            // Every command goes through the same transport with the read
            // timeout; nothing else may reach the shim.
            assert_eq!(timeout, READ_TIMEOUT_MS);
            assert!(
                argv.len() >= 2,
                "a program and at least one flag or property"
            );
            transport_by_mode(healthy_modes())(request, timeout)
        })
        .expect("a healthy system yields a snapshot");

        assert_eq!(
            snapshot
                .installed
                .iter()
                .map(|package| package.name.as_str())
                .collect::<Vec<_>>(),
            ["ark", "vim"],
            "records are sorted by name"
        );
        assert_eq!(snapshot.installed[1].description, "Editor");
        assert_eq!(snapshot.updates.len(), 1);
        assert_eq!(snapshot.updates[0].name, "vim");
        assert_eq!(snapshot.updates[0].version, "9.2.2000_1");
        assert_eq!(snapshot.explicit, vec!["vim".to_owned()]);
        assert!(snapshot.held.is_empty(), "nothing is held");
        assert!(snapshot.orphans.is_empty(), "nothing is orphaned");
        assert_eq!(snapshot.update_of("vim"), Some("9.2.2000_1"));
        assert_eq!(snapshot.update_of("ark"), None);
    }

    #[test]
    fn read_snapshot_round_trips_through_the_model_wire_format() {
        let snapshot = read_snapshot(&transport_by_mode(healthy_modes())).unwrap();
        let json = serde_json::to_string(&snapshot).expect("snapshot serialises");
        let back: Snapshot = serde_json::from_str(&json).expect("snapshot deserialises");
        assert_eq!(back, snapshot);
        assert!(json.contains("\"installed\":"), "camelCase wire keys");
        assert!(
            json.contains("\"installedSizes\":"),
            "the size takes travel in the same wire format"
        );
    }

    #[test]
    fn each_size_take_fills_its_own_list() {
        let snapshot = read_snapshot(&transport_by_mode(healthy_modes())).expect("sizes land");
        assert_eq!(
            snapshot
                .installed_sizes
                .iter()
                .map(|entry| (entry.name.as_str(), entry.bytes))
                .collect::<Vec<_>>(),
            [("ark", 5_242_880), ("vim", 2_176_000)],
            "the installed take is sorted by name and keeps xbps' version"
        );
        assert_eq!(snapshot.installed_sizes[1].version, "9.2.1031_1");
        // A swapped take in `read_snapshot` would show up here: the two
        // repository commands carry different canned values.
        assert_eq!(
            snapshot
                .repo_installed_sizes
                .iter()
                .map(|entry| (entry.name.as_str(), entry.bytes))
                .collect::<Vec<_>>(),
            [("ark", 4_608_000), ("vim", 1_843_200)],
            "the unpacked repository take is its own list"
        );
        assert_eq!(
            snapshot
                .repo_download_sizes
                .iter()
                .map(|entry| (entry.name.as_str(), entry.bytes))
                .collect::<Vec<_>>(),
            [("ark", 1_638_400), ("vim", 1_776_640)],
            "the archive take is its own list"
        );
        assert_eq!(
            snapshot.installed_size_of("vim").expect("vim is installed"),
            snapshot.installed_sizes[1].bytes
        );
        assert_eq!(snapshot.installed_size_of("nope"), None);
    }

    #[test]
    fn a_failing_size_take_keeps_the_package_lists() {
        // An unsynced repository index, an older xbps, a timeout: the sizes are
        // decoration, the package list is the answer. A failed take must not
        // take the snapshot down with it.
        let modes: Vec<(&str, String)> = healthy_modes()
            .into_iter()
            .map(|(flag, reply)| {
                let failed = match flag {
                    "-p installed_size" => Some(ok_reply("not a size line\n")),
                    "-R -p filename-size" => Some(
                        r#"{"exitCode":2,"stdout":"","stderr":"index not synced","timedOut":false}"#
                            .to_owned(),
                    ),
                    "-R -p installed_size" => Some(START_FAILURE.to_owned()),
                    _ => None,
                };
                (flag, failed.unwrap_or(reply))
            })
            .collect();

        let snapshot = read_snapshot(&transport_by_mode(modes)).expect("the list survives");
        assert_eq!(
            snapshot.installed.len(),
            2,
            "the packages the user has are still listed"
        );
        assert_eq!(snapshot.updates.len(), 1);
        assert!(
            snapshot.installed_sizes.is_empty(),
            "a rejected size line leaves this take empty, not invented"
        );
        assert!(
            snapshot.repo_installed_sizes.is_empty(),
            "a repository take that cannot start leaves its list empty"
        );
        assert!(
            snapshot.repo_download_sizes.is_empty(),
            "a repository take that exits non-zero leaves its list empty"
        );
    }

    #[test]
    fn a_size_take_with_an_empty_answer_is_not_a_repository_failure() {
        // `-s ''` over a repository index that was never synced prints nothing
        // and exits 0. That is an empty size list, not an error and not an
        // empty package list.
        let modes: Vec<(&str, String)> = healthy_modes()
            .into_iter()
            .map(|(flag, reply)| {
                if flag.contains("-p ") {
                    (flag, ok_reply(""))
                } else {
                    (flag, reply)
                }
            })
            .collect();
        let snapshot = read_snapshot(&transport_by_mode(modes)).expect("the list survives");
        assert_eq!(snapshot.installed.len(), 2);
        assert!(snapshot.installed_sizes.is_empty());
        assert!(snapshot.repo_installed_sizes.is_empty());
        assert!(snapshot.repo_download_sizes.is_empty());
        assert_eq!(snapshot.installed_size_of("vim"), None);
    }

    #[test]
    fn read_snapshot_sorts_and_deduplicates_the_list_modes() {
        let modes = vec![
            ("-l", ok_reply("ii vim-9.2.1031_1 Editor\n")),
            ("-un", ok_reply("")),
            (
                "-m",
                ok_reply("vim-9.2.1031_1\nark-25.08.0_1\nvim-9.2.1031_1\n"),
            ),
            ("-H", ok_reply("linux-6.18.63_1\nlinux-6.18.63_1\n")),
            ("-O", ok_reply("x-1_1\na-2_1\n")),
            ("-p installed_size", ok_reply("vim-9.2.1031_1: 2125KB\n")),
            ("-R -p filename-size", ok_reply("")),
            ("-R -p installed_size", ok_reply("")),
        ];
        let snapshot = read_snapshot(&transport_by_mode(modes)).unwrap();
        assert_eq!(snapshot.explicit, vec!["ark", "vim"]);
        assert_eq!(snapshot.held, vec!["linux"]);
        assert_eq!(snapshot.orphans, vec!["a", "x"]);
    }

    #[test]
    fn a_start_failure_reports_the_program_and_reason() {
        let modes = vec![("-l", START_FAILURE.to_owned())];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("xbps is missing");
        assert_eq!(error.key(), "errorXbpsStart");
        assert_eq!(
            error.args(),
            vec!["xbps-query", "No such file or directory"]
        );
    }

    #[test]
    fn a_timeout_and_a_non_zero_exit_report_distinct_keys() {
        let modes = vec![(
            "-l",
            r#"{"exitCode":-1,"stdout":"","stderr":"","timedOut":true}"#.to_owned(),
        )];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("slow xbps");
        assert_eq!(error.key(), "errorXbpsTimeout");
        assert_eq!(error.args(), vec!["xbps-query", "30"]);

        let modes = vec![(
            "-l",
            r#"{"exitCode":2,"stdout":"","stderr":"broken pkgdb","timedOut":false}"#.to_owned(),
        )];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("xbps failed");
        assert_eq!(error.key(), "errorXbpsExit");
        assert_eq!(error.args(), vec!["xbps-query", "2", "broken pkgdb"]);
    }

    #[test]
    fn unparsable_output_reports_the_offending_line() {
        let modes = vec![("-l", ok_reply("ii vim-9.2.1031_1 Editor\nbroken line\n"))];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("bad output");
        assert_eq!(error.key(), "errorXbpsOutput");
        assert_eq!(error.args(), vec!["xbps-query", "2", "broken line"]);
    }

    #[test]
    fn an_empty_installed_listing_is_its_own_error() {
        let modes = vec![("-l", ok_reply(""))];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("no packages at all");
        assert_eq!(error.key(), "errorXbpsEmpty");
        assert_eq!(error.args(), vec!["xbps-query"]);
    }

    #[test]
    fn an_undecodable_reply_is_an_internal_error() {
        let modes = vec![("-l", "not json".to_owned())];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("broken shim");
        assert_eq!(error.key(), "errorXbpsReply");
        assert_eq!(
            error.args().len(),
            1,
            "the technical reason is the only arg"
        );
    }

    #[test]
    fn a_failure_in_a_later_command_still_fails_the_whole_snapshot() {
        // `-l` answers, `-un` cannot start: the snapshot is all or nothing,
        // because a half-real list is worse than an honest empty one.
        let modes = vec![
            ("-l", ok_reply("ii vim-9.2.1031_1 Editor\n")),
            ("-un", START_FAILURE.to_owned()),
        ];
        let error = read_snapshot(&transport_by_mode(modes)).expect_err("updates failed");
        assert_eq!(error.key(), "errorXbpsStart");
        assert_eq!(error.args()[0], "xbps-install");
    }

    #[test]
    fn every_key_is_a_translation_key() {
        // QML maps these names to sentences; a rename here without one there
        // would show the key itself.
        let keys = [
            SnapshotError::Start {
                program: String::new(),
                reason: String::new(),
            }
            .key(),
            SnapshotError::Timeout {
                program: String::new(),
            }
            .key(),
            SnapshotError::Exit {
                program: String::new(),
                code: 0,
                stderr: String::new(),
            }
            .key(),
            SnapshotError::Output {
                program: String::new(),
                line: 0,
                text: String::new(),
            }
            .key(),
            SnapshotError::Empty {
                program: String::new(),
            }
            .key(),
            SnapshotError::Reply {
                reason: String::new(),
            }
            .key(),
        ];
        for key in keys {
            assert!(key.starts_with("errorXbps"), "{key} is not an error key");
        }
    }

    #[test]
    fn a_shared_transport_forwards_every_call() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let shared: SharedInvoke = std::sync::Arc::new(move |request: &str, _: u32| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            format!("echo:{request}")
        });
        let worker = shared.clone();
        let handle = std::thread::spawn(move || worker("[1]", 2));
        assert_eq!(handle.join().expect("worker ran"), "echo:[1]");
        assert_eq!(shared("[]", 1), "echo:[]");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn read_search_returns_the_hits_of_the_pattern() {
        let hits = read_search(
            &transport_by_mode(vec![(
                "-Rs",
                ok_reply("[*] vim-9.2.1031_1 Editor\n[-] bash-5.3_2 Bourne Again Shell\n"),
            )]),
            SearchSource::Repository,
            "vim",
        )
        .expect("a search answer lands");
        assert_eq!(hits.len(), 2);
        assert!(hits[0].installed, "the installed tag wins");
        assert_eq!(hits[0].name, "vim");
        assert_eq!(hits[0].version, "9.2.1031_1");
        assert_eq!(hits[1].name, "bash");
        assert!(!hits[1].installed);
    }

    #[test]
    fn read_search_asks_the_installed_database_when_the_source_says_so() {
        let hits = read_search(
            &transport_by_mode(vec![(
                "-s",
                ok_reply("[*] bash-5.3_2 Bourne Again Shell\n"),
            )]),
            SearchSource::Installed,
            "bash",
        )
        .expect("an installed search answer lands");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].installed);
    }

    #[test]
    fn read_search_accepts_an_empty_answer() {
        let hits = read_search(
            &transport_by_mode(vec![("-Rs", ok_reply(""))]),
            SearchSource::Repository,
            "vim",
        )
        .expect("no hits");
        assert!(hits.is_empty(), "an unknown pattern has no hits");
    }

    #[test]
    fn read_raw_answers_verbatim_and_names_the_command() {
        let repos = "1 https://repo.voidlinux.org/current (RSA signed)\n\
                     0 https://repo.voidlinux.org/multilib (RSA signed)\n";
        let output = read_raw(
            &transport_by_mode(vec![("-L", ok_reply(repos))]),
            &repos_argv(),
        )
        .expect("a repository list lands");
        assert_eq!(
            output.text, repos,
            "the block shows exactly what xbps printed"
        );
        assert_eq!(output.command, "xbps-query -L");

        let depends = "glibc>=2.41_1\nzlib>=1.3\n";
        let output = read_raw(
            &transport_by_mode(vec![("-x", ok_reply(depends))]),
            &depends_argv("vim"),
        )
        .expect("a dependency list lands");
        assert_eq!(output.text, depends);
        assert_eq!(output.command, "xbps-query -x vim");
    }

    #[test]
    fn a_failed_raw_take_is_a_key_not_text() {
        let modes = vec![(
            "-p",
            r#"{"exitCode":2,"stdout":"","stderr":"missing","timedOut":false}"#.to_owned(),
        )];
        let error = read_raw(&transport_by_mode(modes), &property_argv("state", "vim"))
            .expect_err("no answer");
        assert_eq!(error.key(), "errorXbpsExit");
        assert_eq!(error.args(), ["xbps-query", "2", "missing"]);
    }

    #[test]
    fn a_failed_search_names_the_program_and_the_key() {
        let modes = vec![(
            "-Rs",
            r#"{"exitCode":2,"stdout":"","stderr":"broken index","timedOut":false}"#.to_owned(),
        )];
        let error = read_search(&transport_by_mode(modes), SearchSource::Repository, "vim")
            .expect_err("a broken index fails the search");
        assert_eq!(error.key(), "errorXbpsExit");
        assert_eq!(error.args(), vec!["xbps-query", "2", "broken index"]);
    }

    #[test]
    fn read_package_reads_the_facts_of_one_installed_package() {
        let invoke = transport_by_mode(vec![
            (
                "-p",
                ok_reply(
                    "vim-9.2.1031_1\nVim editor (vi clone)\nhttps://www.vim.org\nVim\n2125KB\nhttps://repo-de.voidlinux.org/current\ninstalled\n",
                ),
            ),
            (
                "-R -p",
                ok_reply(
                    "vim-9.2.1031_1\nVim editor (vi clone)\nhttps://www.vim.org\nVim\n4851KB\n1735KB\nhttps://repo-de.voidlinux.org/current\n",
                ),
            ),
            ("-x", ok_reply("glibc>=2.41_1\n")),
            ("-X", ok_reply("base-system-0.114_2\n")),
        ]);
        // The same shape production hands a worker thread: a shared transport.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let spy: std::sync::Arc<Invoke> =
            std::sync::Arc::new(move |request: &str, timeout: u32| {
                let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
                log.lock().expect("spy lock").push(argv);
                invoke(request, timeout)
            });
        {
            let facts = read_package(&*spy, "vim", "vim-9.2.1031_1").expect("the facts land");
            assert!(facts.installed);
            assert_eq!(facts.short_desc, "Vim editor (vi clone)");
            assert_eq!(facts.pkgver, "vim-9.2.1031_1");
            assert_eq!(facts.homepage, "https://www.vim.org");
            assert_eq!(facts.license, "Vim");
            assert_eq!(facts.installed_size, "2125KB");
            assert_eq!(facts.repository, "https://repo-de.voidlinux.org/current");
            assert_eq!(facts.state, "installed");
            assert_eq!(facts.filename_size, "1735KB");
            assert_eq!(facts.depends, vec!["glibc>=2.41_1".to_owned()]);
            assert_eq!(facts.required_by, vec!["base-system".to_owned()]);
            assert_eq!(facts.version(), "9.2.1031_1");
        }
        assert_eq!(
            *seen.lock().expect("spy lock"),
            vec![
                vec![
                    "xbps-query",
                    "-p",
                    "pkgver,short_desc,homepage,license,installed_size,repository,state",
                    "vim-9.2.1031_1",
                ],
                vec![
                    "xbps-query",
                    "-R",
                    "-p",
                    "pkgver,short_desc,homepage,license,installed_size,filename-size,repository",
                    "vim-9.2.1031_1",
                ],
                vec!["xbps-query", "-x", "vim"],
                vec!["xbps-query", "-X", "vim"],
            ],
            "the four takes are sent as separate argv elements"
        );
    }

    #[test]
    fn read_package_answers_a_package_that_is_not_installed_from_the_repository_take() {
        // xbps 0.60.7 exits 2 on `-p` for a package that is not installed,
        // and on `-x`/`-X`, even when the repository knows it. "Not installed"
        // is an answer, not an error: the repository take still says what the
        // package is.
        let invoke = transport_by_mode(vec![
            (
                "-p",
                r#"{"exitCode":2,"stdout":"","stderr":"Package 'zramen-1.0.1_1' not installed.","timedOut":false}"#
                    .to_owned(),
            ),
            (
                "-R -p",
                ok_reply(
                    "zramen-1.0.1_1\nManage zram swap space\nhttps://github.com/atweiden/zramen\nUnlicense\n2307B\n5505B\nhttps://repo-de.voidlinux.org/current\n",
                ),
            ),
            (
                "-x",
                r#"{"exitCode":2,"stdout":"","stderr":"Package 'zramen' not found.","timedOut":false}"#
                    .to_owned(),
            ),
            (
                "-X",
                r#"{"exitCode":2,"stdout":"","stderr":"Package 'zramen' not found.","timedOut":false}"#
                    .to_owned(),
            ),
        ]);
        let facts =
            read_package(&invoke, "zramen", "zramen-1.0.1_1").expect("the repository answers");
        assert!(!facts.installed);
        assert_eq!(facts.pkgver, "zramen-1.0.1_1");
        assert_eq!(facts.short_desc, "Manage zram swap space");
        assert_eq!(facts.license, "Unlicense");
        assert_eq!(facts.filename_size, "5505B");
        assert_eq!(facts.state, "not installed");
        assert_eq!(
            facts.installed_size, "2307B",
            "the repository says what the package will take on disk"
        );
        assert!(
            facts.depends.is_empty(),
            "xbps cannot list dependencies of a package that is not installed"
        );
        assert!(facts.required_by.is_empty());
    }

    #[test]
    fn read_package_keeps_the_installed_record_when_the_repository_is_newer() {
        // The installed record is what the user has; the repository take only
        // adds what the installed database cannot say (the download size).
        let invoke = transport_by_mode(vec![
            (
                "-p",
                ok_reply(
                    "vim-9.2.1031_1\nVim editor (vi clone)\nhttps://www.vim.org\nVim\n2125KB\nhttps://repo-de.voidlinux.org/current\ninstalled\n",
                ),
            ),
            (
                "-R -p",
                ok_reply(
                    "vim-9.2.2000_1\nVim editor (vi clone)\nhttps://www.vim.org\nVim\n5000KB\n1800KB\nhttps://repo-de.voidlinux.org/current\n",
                ),
            ),
            ("-x", ok_reply("glibc>=2.41_1\n")),
            ("-X", ok_reply("")),
        ]);
        let facts = read_package(&invoke, "vim", "vim-9.2.1031_1").expect("both takes answer");
        assert!(facts.installed);
        assert_eq!(
            facts.pkgver, "vim-9.2.1031_1",
            "the installed record wins over a newer repository version"
        );
        assert_eq!(facts.version(), "9.2.1031_1");
        assert_eq!(
            facts.filename_size, "1800KB",
            "the download size only the repository take reports"
        );
        assert_eq!(
            facts.installed_size, "2125KB",
            "the installed record's size wins over the repository's"
        );
    }

    #[test]
    fn read_package_of_a_missing_package_asks_for_no_dependency_lists() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let invoke = transport_by_mode(vec![
            (
                "-p",
                r#"{"exitCode":2,"stdout":"","stderr":"Package 'nope-1_1' not installed.","timedOut":false}"#
                    .to_owned(),
            ),
            (
                "-R -p",
                ok_reply(
                    "nope-1_1\nNot here\nhttps://example.org\nMIT\n4321B\n1234B\nhttps://repo-de.voidlinux.org/current\n",
                ),
            ),
            // `-x`/`-X` are not in the table: a call to them panics the fake.
        ]);
        let spy: std::sync::Arc<Invoke> =
            std::sync::Arc::new(move |request: &str, timeout: u32| {
                let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
                log.lock().expect("spy lock").push(argv);
                invoke(request, timeout)
            });
        let facts = read_package(&*spy, "nope", "nope-1_1").expect("the repository answers");
        assert!(!facts.installed);
        assert_eq!(facts.state, "not installed");
        assert_eq!(facts.installed_size, "4321B");
        assert!(facts.depends.is_empty());
        assert!(facts.required_by.is_empty());
        assert_eq!(
            seen.lock().expect("spy lock").len(),
            2,
            "only the two property takes ran; no -x/-X for a package that is not installed"
        );
    }

    #[test]
    fn read_package_fails_when_neither_take_knows_the_package() {
        let not_found = r#"{"exitCode":2,"stdout":"","stderr":"Package 'nope-1_1' not found.","timedOut":false}"#;
        let invoke = transport_by_mode(vec![
            ("-p", not_found.to_owned()),
            ("-R -p", not_found.to_owned()),
            ("-x", not_found.to_owned()),
            ("-X", not_found.to_owned()),
        ]);
        let error = read_package(&invoke, "nope", "nope-1_1").expect_err("nobody knows it");
        assert_eq!(error.key(), "errorXbpsExit");
        assert_eq!(
            error.args(),
            vec!["xbps-query", "2", "Package 'nope-1_1' not found."]
        );
    }

    #[test]
    fn read_package_fails_loudly_on_a_short_property_answer() {
        let invoke = transport_by_mode(vec![
            ("-p", ok_reply("vim-9.2.1031_1\nVim editor (vi clone)\n")),
            ("-R -p", ok_reply("")),
            ("-x", ok_reply("")),
            ("-X", ok_reply("")),
        ]);
        let error = read_package(&invoke, "vim", "vim-9.2.1031_1").expect_err("short answer");
        assert_eq!(error.key(), "errorXbpsOutput");
    }
}
