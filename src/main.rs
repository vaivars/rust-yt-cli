use std::ffi::OsString;
use std::process::ExitCode;

mod cli;

fn main() -> ExitCode {
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    if argv.first().is_some_and(|word| word == "__complete_word__") {
        return cli::completion::answer(&argv);
    }
    cli::run(cli::Yt::parse())
}
