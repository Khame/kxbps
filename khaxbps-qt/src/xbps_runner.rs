//! Synchronous runner for the xbps commands the read layer calls.
//!
//! `src/xbps_shim.cpp` starts one process with an explicit argv list (never a
//! shell string), waits with a hard timeout and answers with a JSON object.
//! That file is not part of the compiled bridge yet — Task 17b adds it to
//! `build.rs` — so [`run`] takes the shim call as a closure instead of linking
//! it. What this module owns today is the testable half: building the argv and
//! decoding the reply. Wiring the shim in 17b only means passing a closure
//! that calls `khaxbps_run_xbps`.
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

/// `xbps-query -L`: configured repositories.
pub fn repos_argv() -> Vec<String> {
    argv(XPBS_QUERY, &["-L"])
}

/// `xbps-query -p <property> <package>`: one package property.
pub fn property_argv(property: &str, package: &str) -> Vec<String> {
    argv(XPBS_QUERY, &["-p", property, package])
}

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
/// shim's JSON. Task 17b passes `khaxbps_run_xbps` here.
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
    }

    #[test]
    fn a_search_pattern_with_spaces_stays_one_argument() {
        assert_eq!(
            search_argv("vim plugin"),
            ["xbps-query", "-Rs", "vim plugin"]
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
