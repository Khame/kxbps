//! The command line, and the help it prints.
//!
//! `Main.qml` reads `--tray`, `--filter` and `--section` from
//! `Qt.application.arguments`, so the window keeps its own flags. What this
//! module adds is what a terminal user expects from any program: `--help` and
//! `--version`. They are answered before `QApplication` exists, so they work
//! without a display.
//!
//! Nothing here is translated. The text goes to stdout, next to the command
//! tokens the rest of the program keeps untranslated too.

/// What the command line asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cli {
    /// Start the window, which reads its own flags from `Qt.application.arguments`.
    Run,
    /// Print [`usage`] and exit without a display.
    Help,
    /// Print [`version_line`] and exit without a display.
    Version,
}

/// Reads the two flags this module answers.
///
/// `args` is `std::env::args()`, so the first item is the program path and is
/// skipped. `--help` and `--version` win wherever they appear; every other
/// argument belongs to the window and leaves the verdict at [`Cli::Run`],
/// including the ones this module does not know.
pub fn parse(args: &[String]) -> Cli {
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--help" | "-h" => return Cli::Help,
            "--version" | "-V" => return Cli::Version,
            _ => {}
        }
    }
    Cli::Run
}

/// The text `--help` prints.
///
/// It names every flag the window reads, because a flag that cannot be
/// discovered from the terminal is a flag only its author knows.
pub fn usage() -> String {
    [
        "KXbps — a native KDE package manager for Void Linux.",
        "",
        "Usage: kxbps [OPTION]…",
        "",
        "Options:",
        "  --tray           start hidden in the tray",
        "  --filter TOKEN   open the package table on one filter:",
        "                   installed, updates, all, queued, explicit, orphans, held",
        "  --section NAME   open one page: packages, query, mirrors, repositories,",
        "                   settings, help",
        "  --help, -h       print this text and exit",
        "  --version, -V    print the version and exit",
        "",
        "State lives in ~/.local/share/kxbps/ (queue.json, prefs.json, mirror.json).",
        "Reading needs no root; every change goes through polkit.",
    ]
    .join("\n")
}

/// The line `--version` prints.
pub fn version_line() -> String {
    format!("kxbps {}", crate::VERSION)
}

#[cfg(test)]
mod tests {
    use super::{Cli, parse, usage, version_line};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn help_wins_wherever_it_appears() {
        assert_eq!(parse(&args(&["kxbps", "--help"])), Cli::Help);
        assert_eq!(parse(&args(&["kxbps", "-h"])), Cli::Help);
        assert_eq!(parse(&args(&["kxbps", "--tray", "--help"])), Cli::Help);
    }

    #[test]
    fn version_wins_wherever_it_appears() {
        assert_eq!(parse(&args(&["kxbps", "--version"])), Cli::Version);
        assert_eq!(parse(&args(&["kxbps", "-V"])), Cli::Version);
    }

    #[test]
    fn the_window_flags_still_start_the_window() {
        assert_eq!(parse(&args(&["kxbps"])), Cli::Run);
        assert_eq!(parse(&args(&["kxbps", "--tray"])), Cli::Run);
        assert_eq!(parse(&args(&["kxbps", "--section", "settings"])), Cli::Run);
        assert_eq!(parse(&args(&["kxbps", "--filter", "updates"])), Cli::Run);
    }

    #[test]
    fn usage_names_every_flag_and_every_section() {
        let text = usage();
        for flag in ["--tray", "--filter", "--section", "--help", "--version"] {
            assert!(text.contains(flag), "usage does not mention {flag}");
        }
        for section in [
            "packages",
            "query",
            "mirrors",
            "repositories",
            "settings",
            "help",
        ] {
            assert!(
                text.contains(section),
                "usage does not mention the {section} page"
            );
        }
        for filter in [
            "installed",
            "updates",
            "all",
            "queued",
            "explicit",
            "orphans",
            "held",
        ] {
            assert!(
                text.contains(filter),
                "usage does not mention the {filter} filter"
            );
        }
    }

    #[test]
    fn the_version_line_carries_the_package_version() {
        assert_eq!(version_line(), format!("kxbps {}", crate::VERSION));
    }
}
