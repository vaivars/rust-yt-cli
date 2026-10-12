use miette::Diagnostic;
use thiserror::Error;

pub type Result<T, E = miette::Report> = std::result::Result<T, E>;

#[derive(Debug, Error, Diagnostic)]
pub enum YtError {
    #[error("{message}")]
    #[diagnostic(code(yt::usage))]
    Usage { message: String },

    #[error("{what} is required")]
    #[diagnostic(code(yt::input_required), help("pass {flag}"))]
    InputRequired { what: String, flag: String },

    #[error("cancelled")]
    #[diagnostic(code(yt::cancelled))]
    Cancelled,

    #[error("HOME is not set")]
    #[diagnostic(
        code(yt::no_home),
        help("set HOME, or set YT_CONFIG_DIR, YT_CACHE_DIR and YT_STATE_DIR")
    )]
    NoHome,

    #[error("{path}: {message}")]
    #[diagnostic(code(yt::config))]
    Config { path: String, message: String },

    #[error("no profile is configured")]
    #[diagnostic(
        code(yt::no_profile),
        help("run `yt auth login`, or set YOUTRACK_URL and YOUTRACK_TOKEN")
    )]
    NoProfile,

    #[error("no profile uses {url}")]
    #[diagnostic(
        code(yt::no_profile_for_url),
        help("{path} names this instance; if you trust it, run `yt auth login {url}`")
    )]
    NoProfileForUrl { url: String, path: String },

    #[error("profile '{name}' does not exist")]
    #[diagnostic(code(yt::unknown_profile))]
    UnknownProfile {
        name: String,
        #[help]
        help: Option<String>,
    },

    #[error("{count} profiles use {url}")]
    #[diagnostic(
        code(yt::ambiguous_profile),
        help("pick one with --profile, or run `yt profile use <name>`")
    )]
    AmbiguousProfile { url: String, count: usize },

    #[error("not signed in to {profile}")]
    #[diagnostic(
        code(yt::not_signed_in),
        help("run `yt auth login --profile {profile}`")
    )]
    NotSignedIn { profile: String },

    #[error("{profile}: the server rejected the credential")]
    #[diagnostic(
        code(yt::credential_rejected),
        help("run `yt auth login --profile {profile}`, or check YOUTRACK_TOKEN")
    )]
    CredentialRejected { profile: String },

    #[error("the server rejected the token")]
    #[diagnostic(
        code(yt::token_rejected),
        help("the token must include the YouTrack scope")
    )]
    TokenRejected,

    #[error("your sign-in to {profile} has expired")]
    #[diagnostic(
        code(yt::session_expired),
        help("run `yt auth login --profile {profile}`")
    )]
    SessionExpired { profile: String },

    #[error("no system keyring is available ({reason})")]
    #[diagnostic(
        code(yt::no_keyring),
        help("set YOUTRACK_URL and YOUTRACK_TOKEN instead")
    )]
    NoKeyring { reason: String },

    #[error("keyring: {0}")]
    #[diagnostic(code(yt::keyring))]
    Keyring(String),

    #[error("{url} is not a reachable YouTrack instance: {reason}")]
    #[diagnostic(
        code(yt::discovery),
        help("use the address you open YouTrack at, e.g. https://acme.youtrack.cloud")
    )]
    Discovery { url: String, reason: String },

    #[error("sign-in failed: {message}")]
    #[diagnostic(code(yt::oauth))]
    OAuth {
        message: String,
        #[help]
        help: Option<String>,
    },

    #[error("{method} {path}: {message}")]
    #[diagnostic(code(yt::api))]
    Api {
        method: String,
        path: String,
        status: u16,
        message: String,
        #[help]
        help: Option<String>,
    },

    #[error("network error: {0}")]
    #[diagnostic(
        code(yt::network),
        help("check your network connection and the instance URL")
    )]
    Network(String),
}

/// A copy of `report` for display, with terminal control characters replaced in every text.
pub fn printable(report: &miette::Report) -> miette::Report {
    miette::Report::new(Printable::of(report.as_ref()))
}

#[derive(Debug)]
struct Printable {
    message: String,
    code: Option<String>,
    help: Option<String>,
    url: Option<String>,
    severity: Option<miette::Severity>,
    source: Option<Box<Printable>>,
}

impl Printable {
    fn of(diagnostic: &dyn miette::Diagnostic) -> Self {
        Self {
            message: text(&diagnostic),
            code: diagnostic.code().map(|code| text(&code)),
            help: diagnostic.help().map(|help| text(&help)),
            url: diagnostic.url().map(|url| text(&url)),
            severity: diagnostic.severity(),
            source: diagnostic.source().map(Self::cause),
        }
    }

    fn cause(error: &dyn std::error::Error) -> Box<Self> {
        Box::new(Self {
            message: text(&error),
            code: None,
            help: None,
            url: None,
            severity: None,
            source: error.source().map(Self::cause),
        })
    }
}

fn text(value: &dyn std::fmt::Display) -> String {
    crate::output::printable(&value.to_string()).into_owned()
}

impl std::fmt::Display for Printable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Printable {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl miette::Diagnostic for Printable {
    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        boxed(self.code.as_ref())
    }

    fn help<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        boxed(self.help.as_ref())
    }

    fn url<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        boxed(self.url.as_ref())
    }

    fn severity(&self) -> Option<miette::Severity> {
        self.severity
    }
}

fn boxed<'a>(text: Option<&'a String>) -> Option<Box<dyn std::fmt::Display + 'a>> {
    text.map(|text| Box::new(text) as Box<dyn std::fmt::Display + 'a>)
}

pub fn exit_code(report: &miette::Report) -> u8 {
    match report.downcast_ref::<YtError>() {
        Some(YtError::Usage { .. } | YtError::InputRequired { .. }) => 2,
        Some(
            YtError::NotSignedIn { .. }
            | YtError::CredentialRejected { .. }
            | YtError::TokenRejected
            | YtError::SessionExpired { .. },
        ) => 4,
        Some(YtError::Cancelled) => 130,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(error: YtError) -> u8 {
        exit_code(&miette::Report::new(error))
    }

    #[test]
    fn exit_codes_follow_the_spec() {
        assert_eq!(
            code(YtError::Usage {
                message: "x".into()
            }),
            2
        );
        assert_eq!(
            code(YtError::InputRequired {
                what: "URL".into(),
                flag: "<URL>".into()
            }),
            2
        );
        assert_eq!(
            code(YtError::NotSignedIn {
                profile: "acme".into()
            }),
            4
        );
        assert_eq!(
            code(YtError::CredentialRejected {
                profile: "acme".into()
            }),
            4
        );
        assert_eq!(code(YtError::TokenRejected), 4);
        assert_eq!(
            code(YtError::SessionExpired {
                profile: "acme".into()
            }),
            4
        );
        assert_eq!(code(YtError::Cancelled), 130);
        assert_eq!(code(YtError::NoProfile), 1);
    }

    fn plain(diagnostic: &dyn miette::Diagnostic) -> String {
        let mut text = String::new();
        miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
            .render_report(&mut text, diagnostic)
            .unwrap();
        text
    }

    #[test]
    fn printable_reports_keep_the_text_without_control_characters() {
        let report = miette::Report::new(YtError::Api {
            method: "GET".into(),
            path: "/api/issues".into(),
            status: 400,
            message: "bad\u{1b}[2J query".into(),
            help: Some("try\u{7} again".into()),
        });
        let wrapped = miette::Report::new(YtError::Api {
            method: "GET".into(),
            path: "/api/issues".into(),
            status: 400,
            message: "inner\u{9b}1A".into(),
            help: None,
        })
        .wrap_err("while \u{1b}]0;t\u{7}reading");
        for report in [report, wrapped] {
            let text = plain(printable(&report).as_ref());
            assert_eq!(text, crate::output::printable(&plain(report.as_ref())));
            assert!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                "{text:?}"
            );
        }
    }

    #[test]
    fn exit_code_survives_added_context() {
        let report = miette::Report::new(YtError::Cancelled).wrap_err("while signing in");
        assert_eq!(exit_code(&report), 130);
    }

    #[test]
    fn help_names_the_profile() {
        let report = miette::Report::new(YtError::NotSignedIn {
            profile: "acme".into(),
        });
        assert!(format!("{report:?}").contains("yt auth login --profile acme"));
    }
}
