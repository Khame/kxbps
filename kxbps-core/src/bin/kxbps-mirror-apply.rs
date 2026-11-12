//! `kxbps-mirror-apply`: the one privileged helper behind "Apply to system".
//!
//! polkit runs this binary as root through the `io.github.khame.kxbps.mirror`
//! action, with one base URL as its only argument. The whole pipeline lives in
//! [`kxbps::mirror_apply::Cli`]; this file only parses the argv, prints what
//! happened on stderr and exits with the documented code.
//!
//! The commands the helper runs (`xbps-install -S`, `xbps-query -L`) are
//! started as separate argv elements with stdin on `/dev/null`, so a repository
//! that asks for a new key gets an error instead of a prompt nobody can answer.

fn main() {
    let program = std::env::args_os()
        .next()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("kxbps-mirror-apply"));
    let cli = kxbps_core::mirror_apply::Cli::parse(std::env::args_os().skip(1));
    if cli.url.is_none() {
        // Not a repository question, so the usage line is the whole answer and
        // the exit code is the documented "URL refused" one.
        eprintln!("{}", kxbps_core::mirror_apply::Cli::usage(&program));
    }
    let result = cli.run();
    if !result.ok() {
        let detail = if result.detail.is_empty() {
            format!("exit {}", result.exit.code())
        } else {
            result.detail.clone()
        };
        eprintln!("kxbps-mirror-apply: {detail}");
    }
    std::process::exit(result.exit.code());
}
