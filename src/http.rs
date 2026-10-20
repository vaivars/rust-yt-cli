use std::path::PathBuf;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};

use crate::config::settings::Settings;
use crate::error::{Result, YtError};

pub struct HttpConfig {
    pub timeout: Duration,
    pub retries: u32,
    pub ca_file: Option<PathBuf>,
}

impl HttpConfig {
    pub fn from_settings(settings: &Settings, ca_file: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            timeout: settings.http_timeout()?,
            retries: u32::try_from(settings.http_retries)
                .unwrap_or(u32::MAX)
                .min(10),
            ca_file,
        })
    }
}

pub fn client(config: &HttpConfig) -> Result<reqwest::Client> {
    build(builder(config)?)
}

/// For requests whose redirect is the answer, such as the sign-in preflight.
pub fn client_without_redirects(config: &HttpConfig) -> Result<reqwest::Client> {
    build(builder(config)?.redirect(reqwest::redirect::Policy::none()))
}

pub fn user_agent() -> String {
    format!(
        "yt/{} ({}-{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

/// A failed request as the user reads it; why a redirect was stopped is only in the error's source.
pub fn describe(error: &reqwest::Error) -> String {
    let message = match std::error::Error::source(error) {
        Some(reason) if error.is_redirect() => format!("{error}: {reason}"),
        _ => error.to_string(),
    };
    crate::redact::redact(&message)
}

pub fn is_transient(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429 | 500..=599)
}

pub fn retry_delay(attempt: u32, headers: Option<&HeaderMap>) -> Duration {
    let retry_after = headers
        .and_then(|headers| headers.get(RETRY_AFTER))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    match retry_after {
        Some(delay) => delay.min(Duration::from_secs(15)),
        None => match attempt {
            0 => Duration::from_millis(200),
            1 => Duration::from_secs(1),
            _ => Duration::from_secs(4),
        },
    }
}

fn builder(config: &HttpConfig) -> Result<reqwest::ClientBuilder> {
    // reqwest is built without a default crypto provider; tests reach this without going through main.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .redirect(redirect_policy());
    if !config.timeout.is_zero() {
        builder = builder.timeout(config.timeout);
    }
    if let Some(path) = &config.ca_file {
        let invalid = |message: String| YtError::Config {
            path: path.display().to_string(),
            message,
        };
        let pem = std::fs::read(path).map_err(|error| invalid(error.to_string()))?;
        let certificates = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|error| invalid(format!("not a PEM certificate bundle: {error}")))?;
        builder = builder.tls_certs_merge(certificates);
    }
    Ok(builder)
}

/// reqwest keeps `Authorization` on a redirect to the same host and port, even from https to http.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let previous = attempt.previous();
        if previous
            .last()
            .is_some_and(|from| downgrades(from, attempt.url()))
        {
            attempt.error("refused a redirect from https to http")
        } else if previous.len() > 10 {
            attempt.error("too many redirects")
        } else {
            attempt.follow()
        }
    })
}

fn downgrades(from: &reqwest::Url, to: &reqwest::Url) -> bool {
    from.scheme() == "https" && to.scheme() == "http"
}

fn build(builder: reqwest::ClientBuilder) -> Result<reqwest::Client> {
    builder
        .build()
        .map_err(|error| YtError::Network(error.to_string()).into())
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    fn config() -> HttpConfig {
        HttpConfig {
            timeout: Duration::from_secs(5),
            retries: 3,
            ca_file: None,
        }
    }

    #[test]
    fn user_agent_names_the_version_and_platform() {
        let agent = user_agent();
        assert!(
            agent.starts_with(&format!("yt/{} (", env!("CARGO_PKG_VERSION"))),
            "{agent}"
        );
        assert!(agent.contains(std::env::consts::OS), "{agent}");
    }

    #[test]
    fn transient_statuses() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert!(
                is_transient(StatusCode::from_u16(status).unwrap()),
                "{status}"
            );
        }
        for status in [400, 401, 403, 404, 409] {
            assert!(
                !is_transient(StatusCode::from_u16(status).unwrap()),
                "{status}"
            );
        }
    }

    #[test]
    fn retry_delays_back_off_and_honour_retry_after() {
        assert_eq!(retry_delay(0, None), Duration::from_millis(200));
        assert_eq!(retry_delay(1, None), Duration::from_secs(1));
        assert_eq!(retry_delay(5, None), Duration::from_secs(4));
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(retry_delay(0, Some(&headers)), Duration::from_secs(2));
        headers.insert(RETRY_AFTER, HeaderValue::from_static("120"));
        assert_eq!(retry_delay(0, Some(&headers)), Duration::from_secs(15));
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_delay(1, Some(&headers)), Duration::from_secs(1));
    }

    #[test]
    fn only_a_redirect_from_https_to_http_is_a_downgrade() {
        let url = |text: &str| reqwest::Url::parse(text).unwrap();
        assert!(downgrades(
            &url("https://acme.example/api/x"),
            &url("http://acme.example/api/x")
        ));
        for (from, to) in [
            ("https://acme.example/a", "https://acme.example/b"),
            ("http://acme.example/a", "http://acme.example/b"),
            ("http://acme.example/a", "https://acme.example/a"),
        ] {
            assert!(!downgrades(&url(from), &url(to)), "{from} -> {to}");
        }
    }

    #[test]
    fn clients_build_without_an_installed_provider() {
        client(&config()).unwrap();
        client_without_redirects(&config()).unwrap();
    }

    #[test]
    fn a_missing_ca_file_is_a_config_error() {
        let error = client(&HttpConfig {
            ca_file: Some("/nonexistent/ca.pem".into()),
            ..config()
        })
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<YtError>(),
            Some(YtError::Config { .. })
        ));
    }

    #[test]
    fn settings_map_to_config_and_retries_are_capped() {
        let settings = Settings {
            http_timeout: "5s".into(),
            http_retries: 99,
            color: "auto".into(),
            cache_ttl: "1h".into(),
        };
        let config = HttpConfig::from_settings(&settings, None).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.retries, 10);
    }
}
