use usage_rs::Args;
use yt::output::Format;

/// Flags every command accepts. Their environment variables are read in `Ctx::new`, through `Env`.
#[derive(Clone, Debug, Args)]
pub(crate) struct Globals {
    /// Profile to use [env: YT_PROFILE]
    #[usage(long, global, value_name = "PROFILE")]
    pub(crate) profile: Option<String>,
    /// Output format (default: table) [env: YT_FORMAT]
    #[usage(long, global, value_enum)]
    pub(crate) format: Option<Format>,
    /// More log output: -v for debug, -vv for trace
    #[usage(short, long, global, count)]
    pub(crate) verbose: u8,
    /// Only print errors
    #[usage(short, long, global)]
    pub(crate) quiet: bool,
    /// Disable color
    #[usage(long, global)]
    pub(crate) no_color: bool,
    /// Fail instead of prompting [env: YT_NO_INPUT]
    #[usage(long, global)]
    pub(crate) no_input: bool,
    /// Confirm destructive actions without asking
    #[usage(short, long, global)]
    pub(crate) yes: bool,
}
