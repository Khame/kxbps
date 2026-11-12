//! Synchronous runner for the xbps commands the read layer calls.
//!
//! `src/xbps_shim.cpp` starts one process with an explicit argv list (never a
//! shell string), waits with a hard timeout and answers with a JSON object.
//! [`run`] takes that call as a closure instead of linking the shim, so the
//! argv builders and the reply decoder are testable without a GUI; production
//! passes the closure from [`crate::production_invoke`] and every test passes a
//! fake, so the suite never shells out to xbps.
//!
//! Timeouts and non-zero exit codes are data, not errors: a read command that
//! exits 2 has a stderr line the caller wants to show. [`RunnerError`] is
//! reserved for a request that cannot be formed and a reply that cannot be
//! decoded.

use serde::Deserialize;

/// Program name of the query tool.
pub const XPBS_QUERY: &str = "xbps-query";

/// Program name of the transaction tool; `-un` and `-n` do not change anything.
pub const XPBS_INSTALL: &str = "xbps-install";

/// Program name of the removal tool; `-n` only prints the plan.
pub const XPBS_REMOVE: &str = "xbps-remove";

/// Timeout for the read-only commands, which only touch the local package
/// database and the already-synced repository indexes.
pub const READ_TIMEOUT_MS: u32 = 30_000;

/// What the C++ shim returns for one process.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XbpsOutput {
    /// Process exit code, or `-1` when it could not be started or died from a
    /// signal.
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// The timeout killed the process before it exited.
    pub timed_out: bool,
}

impl XbpsOutput {
    /// Exited on its own with code 0.
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }
}

/// A request or a reply that cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnerError {
    /// The argv list has no program to run.
    EmptyArgv,
    /// The shim answered with something that is not the documented object.
    InvalidReply(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyArgv => f.write_str("xbps command has no program"),
            Self::InvalidReply(reason) => {
                write!(f, "xbps runner reply is not understood: {reason}")
            }
        }
    }
}

impl std::error::Error for RunnerError {}

/// Builds an argv list: the program first, then its arguments untouched.
pub fn argv(program: &str, args: &[&str]) -> Vec<String> {
    let mut list = Vec::with_capacity(args.len() + 1);
    list.push(program.to_owned());
    list.extend(args.iter().map(|arg| (*arg).to_owned()));
    list
}

/// `xbps-query -l`: every installed package.
pub fn installed_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-l"])
}

/// `xbps-install -un`: pending updates in the dry-run plan format.
pub fn updates_argv() -> Vec<String> {
    argv(XPBS_INSTALL, &["-un"])
}

/// `xbps-query -Rs <pattern>`: repository search.
///
/// The pattern stays one argument even when it contains spaces; xbps never
/// sees a shell.
pub fn search_argv(pattern: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-Rs", pattern])
}

/// `xbps-query -s <pattern>`: search the installed packages only.
pub fn installed_search_argv(pattern: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-s", pattern])
}

/// `xbps-query -L`: configured repositories.
pub fn repos_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-L"])
}

/// `xbps-query -p <property> <package>`: one package property.
pub fn property_argv(property: &str, package: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-p", property, package])
}

/// `xbps-query -R -p <property-list> <package>`: the repository-mode property
/// take, which also answers for facts the plain take cannot (`filename-size`).
pub fn property_repo_argv(property: &str, package: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-R", "-p", property, package])
}

/// `xbps-query -x <package>`: the dependencies of one package.
pub fn depends_argv(package: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-x", package])
}

/// `xbps-query -X <package>`: the reverse dependencies of one package.
pub fn required_by_argv(package: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-X", package])
}

/// `xbps-query -m`: the packages installed on purpose.
pub fn manual_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-m"])
}

/// `xbps-query -H`: the packages pinned against updates.
pub fn held_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-H"])
}

/// `xbps-query -O`: the installed packages nothing depends on.
pub fn orphans_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-O"])
}

/// `xbps-query -p installed_size -s ''`: the installed size of every installed
/// package in one take.
///
/// The empty pattern is one argv element; it is the documented way to make
/// `-s` match every package without a shell expanding anything.
pub fn installed_sizes_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-p", "installed_size", "-s", ""])
}

/// `xbps-query -R -p filename-size -s ''`: the archive size of every package
/// the repository indexes offer.
pub fn repo_filename_sizes_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-R", "-p", "filename-size", "-s", ""])
}

/// `xbps-query -R -p installed_size -s ''`: the unpacked size of every package
/// the repository indexes offer.
pub fn repo_installed_sizes_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-R", "-p", "installed_size", "-s", ""])
}

/// Timeout for a privileged transaction: polkit waits for a password and xbps
/// downloads and unpacks, so the read path's 30 s is far too short. A
/// transaction that outlives this is treated as failed and never applied.
pub const TRANSACTION_TIMEOUT_MS: u32 = 30 * 60 * 1000;

/// Encodes an argv list for the shim.
pub fn argv_json(argv: &[String]) -> Result<String, RunnerError> {
    if argv.is_empty() || argv[0].is_empty() {
        return Err(RunnerError::EmptyArgv);
    }
    Ok(serde_json::to_string(argv).expect("a list of strings always serialises to a JSON array"))
}

/// Decodes the shim's reply.
pub fn decode_reply(reply: &str) -> Result<XbpsOutput, RunnerError> {
    serde_json::from_str(reply).map_err(|err| RunnerError::InvalidReply(err.to_string()))
}

/// Runs one command through `invoke`, which stands in for the C++ shim.
///
/// `invoke` receives the encoded argv and the timeout and must return the
/// shim's JSON. Task 17b passes `kxbps_run_xbps` here.
pub fn run(
    argv: &[String],
    timeout_ms: u32,
    invoke: impl FnOnce(&str, u32) -> String,
) -> Result<XbpsOutput, RunnerError> {
    let request = argv_json(argv)?;
    decode_reply(&invoke(&request, timeout_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_commands_have_the_documented_argv() {
        assert_eq!(installed_argv(), ["xbps-query", "-l"]);
        assert_eq!(updates_argv(), ["xbps-install", "-un"]);
        assert_eq!(repos_argv(), ["xbps-query", "-L"]);
        assert_eq!(search_argv("vim"), ["xbps-query", "-Rs", "vim"]);
        assert_eq!(
            property_argv("pkgver", "vim"),
            ["xbps-query", "-p", "pkgver", "vim"]
        );
        assert_eq!(
            property_repo_argv("pkgver,short_desc", "vim-9.2.1031_1"),
            [
                "xbps-query",
                "-R",
                "-p",
                "pkgver,short_desc",
                "vim-9.2.1031_1"
            ]
        );
        assert_eq!(installed_search_argv("lib"), ["xbps-query", "-s", "lib"]);
        assert_eq!(depends_argv("bash"), ["xbps-query", "-x", "bash"]);
        assert_eq!(required_by_argv("bash"), ["xbps-query", "-X", "bash"]);
        assert_eq!(manual_argv(), ["xbps-query", "-m"]);
        assert_eq!(held_argv(), ["xbps-query", "-H"]);
        assert_eq!(orphans_argv(), ["xbps-query", "-O"]);
    }

    #[test]
    fn a_search_pattern_with_spaces_stays_one_argument() {
        assert_eq!(
            search_argv("vim plugin"),
            ["xbps-query", "-Rs", "vim plugin"]
        );
    }

    #[test]
    fn the_bulk_size_takes_use_an_empty_pattern_as_one_argument() {
        // `-s ''` is what makes xbps print every package; the empty string is
        // one argv element and never a shell artifact.
        assert_eq!(
            installed_sizes_argv(),
            ["xbps-query", "-p", "installed_size", "-s", ""]
        );
        assert_eq!(
            repo_filename_sizes_argv(),
            ["xbps-query", "-R", "-p", "filename-size", "-s", ""]
        );
        assert_eq!(
            repo_installed_sizes_argv(),
            ["xbps-query", "-R", "-p", "installed_size", "-s", ""]
        );
        let encoded = argv_json(&repo_filename_sizes_argv()).expect("argv encodes");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&encoded).expect("decodes"),
            ["xbps-query", "-R", "-p", "filename-size", "-s", ""],
            "the empty pattern survives the round trip as its own element"
        );
    }

    #[test]
    fn argv_json_encodes_a_json_array() {
        let encoded = argv_json(&installed_argv()).expect("argv encodes");
        assert_eq!(encoded, r#"["xbps-query","-l"]"#);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&encoded).expect("decodes"),
            installed_argv()
        );
    }

    #[test]
    fn argv_json_rejects_an_empty_list() {
        assert_eq!(argv_json(&[]), Err(RunnerError::EmptyArgv));
        assert_eq!(
            argv_json(&[String::new(), "-l".to_owned()]),
            Err(RunnerError::EmptyArgv)
        );
    }

    #[test]
    fn decode_reply_reads_every_field() {
        let reply =
            r#"{"exitCode":0,"stdout":"ii vim-9.2.1031_1 Editor\n","stderr":"","timedOut":false}"#;
        let output = decode_reply(reply).expect("reply decodes");
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, "ii vim-9.2.1031_1 Editor\n");
        assert_eq!(output.stderr, "");
        assert!(!output.timed_out);
        assert!(output.success());
    }

    #[test]
    fn a_failed_or_timed_out_process_is_not_success() {
        let failed = XbpsOutput {
            exit_code: 2,
            stdout: String::new(),
            stderr: "Package 'x' not found".to_owned(),
            timed_out: false,
        };
        assert!(!failed.success());

        let killed = XbpsOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
        };
        assert!(!killed.success());
    }

    #[test]
    fn decode_reply_rejects_a_missing_field() {
        let error = decode_reply(r#"{"exitCode":0,"stdout":"","stderr":""}"#)
            .expect_err("timedOut is required");
        assert!(matches!(error, RunnerError::InvalidReply(_)));
    }

    #[test]
    fn decode_reply_rejects_text_that_is_not_json() {
        let error = decode_reply("not json").expect_err("text does not decode");
        assert!(matches!(error, RunnerError::InvalidReply(_)));
        assert!(error.to_string().contains("not understood"));
    }

    #[test]
    fn run_passes_the_encoded_argv_and_timeout_to_the_shim() {
        let reply = r#"{"exitCode":0,"stdout":"ok","stderr":"","timedOut":false}"#;
        let mut seen_request = String::new();
        let mut seen_timeout = 0;
        let output = run(&installed_argv(), 1234, |request, timeout| {
            seen_request = request.to_owned();
            seen_timeout = timeout;
            reply.to_owned()
        })
        .expect("run decodes the reply");
        assert_eq!(seen_request, r#"["xbps-query","-l"]"#);
        assert_eq!(seen_timeout, 1234);
        assert_eq!(output.stdout, "ok");
        assert!(output.success());
    }

    #[test]
    fn run_does_not_call_the_shim_for_an_empty_argv() {
        let output = run(&[], READ_TIMEOUT_MS, |_, _| {
            panic!("the shim must not be called");
        });
        assert_eq!(output, Err(RunnerError::EmptyArgv));
    }

    #[test]
    fn run_reports_a_reply_it_cannot_decode() {
        let output = run(&installed_argv(), READ_TIMEOUT_MS, |_, _| "{}".to_owned());
        assert!(matches!(output, Err(RunnerError::InvalidReply(_))));
    }
}
