//! One consistent take of the system state, read through the xbps runner.
//!
//! Five read-only commands answer in one go — `xbps-query -l` (installed
//! packages), `xbps-install -un` (pending updates), and `xbps-query -m`, `-H`
//! and `-O` (explicit, held and orphaned names) — and every line is parsed by
//! `kxbps-core`. Nothing here touches Qt: the transport is the `Invoke`
//! closure, so production installs the C++ shim and the tests install a fake.
//!
//! A snapshot is all or nothing. A list that is half real would be the one lie
//! this layer must never tell, so one failing command fails the whole take and
//! the caller shows why instead of an empty window.

use kxbps_core::{
    InstalledPackage, UpdateEntry, XbpsError, parse_installed, parse_pkgver_list, parse_updates,
};

use crate::xbps_runner::{
    READ_TIMEOUT_MS, held_argv, installed_argv, manual_argv, orphans_argv, run, updates_argv,
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
}

impl Snapshot {
    /// The version the pending update of `name` would install, if any.
    pub fn update_of(&self, name: &str) -> Option<&str> {
        self.updates
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.version.as_str())
    }
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
fn run_parsed<T>(
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

/// Reads the whole system snapshot through `invoke`:
/// `xbps-query -l`, `xbps-install -un`, `xbps-query -m`, `-H` and `-O`.
///
/// All or nothing: any failing command fails the snapshot, because a list
/// that is half real is the one lie this module must never tell. The result
/// is sorted, so the wire format is stable and the model can diff it.
pub fn read_snapshot(invoke: &Invoke) -> Result<Snapshot, SnapshotError> {
    let mut installed = run_parsed(&installed_argv(), parse_installed, invoke)?;
    let mut updates = run_parsed(&updates_argv(), parse_updates, invoke)?;
    let explicit = run_parsed(&manual_argv(), parse_pkgver_list, invoke)?;
    let held = run_parsed(&held_argv(), parse_pkgver_list, invoke)?;
    let orphans = run_parsed(&orphans_argv(), parse_pkgver_list, invoke)?;

    installed.sort_by(|left, right| left.name.cmp(&right.name));
    updates.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Snapshot {
        installed,
        updates,
        explicit: sorted_names(explicit),
        held: sorted_names(held),
        orphans: sorted_names(orphans),
    })
}

/// Sorted and deduplicated, so a repeated answer cannot double a count.
fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
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

    /// A transport that answers each request by its second argv element (the
    /// mode flag), so one fake covers the whole five-command snapshot.
    fn transport_by_mode(modes: Vec<(&str, String)>) -> impl Fn(&str, u32) -> String {
        move |request, _| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            let mode = argv.get(1).map(String::as_str).unwrap_or_default();
            modes
                .iter()
                .find(|(flag, _)| *flag == mode)
                .map(|(_, reply)| reply.clone())
                .unwrap_or_else(|| panic!("unexpected command: {argv:?}"))
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
        ]
    }

    #[test]
    fn read_snapshot_assembles_and_sorts_the_five_answers() {
        let snapshot = read_snapshot(&|request, timeout| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            // Every command goes through the same transport with the read
            // timeout; nothing else may reach the shim.
            assert_eq!(timeout, READ_TIMEOUT_MS);
            assert_eq!(argv.len(), 2, "one mode flag per read command");
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
}
