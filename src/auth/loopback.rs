use std::collections::HashMap;
use std::time::Duration;

use miette::IntoDiagnostic as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use url::Url;

use crate::error::{Result, YtError};

pub const CALLBACK_PATH: &str = "/callback";
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A one-shot listener on 127.0.0.1 for the browser's redirect after sign-in.
pub struct Callback {
    listener: TcpListener,
    port: u16,
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

impl Callback {
    pub async fn bind() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .into_diagnostic()?;
        let port = listener.local_addr().into_diagnostic()?.port();
        Ok(Self { listener, port })
    }

    pub fn redirect_uri(&self) -> String {
        redirect_uri(self.port)
    }

    /// Waits for the redirect and returns the authorization code.
    pub async fn wait(self, state: &str, timeout: Duration) -> Result<String> {
        tokio::time::timeout(timeout, self.accept(state))
            .await
            .map_err(|_| {
                miette::Report::from(YtError::OAuth {
                    message: "the browser did not come back".into(),
                    help: Some(
                        "run `yt auth login --no-browser` to finish by pasting the address instead"
                            .into(),
                    ),
                })
            })?
    }

    async fn accept(&self, state: &str) -> Result<String> {
        // Each connection is read on its own task, so an idle one (a browser preconnect) cannot hold up the callback.
        let (requests, mut callbacks) = mpsc::channel(8);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted.into_diagnostic()?;
                    tokio::spawn(read_callback(stream, requests.clone()));
                }
                Some((mut stream, params)) = callbacks.recv() => {
                    if !state_matches(&params, state) {
                        respond(&mut stream, 400, "This address is not for the running sign-in.").await;
                        continue;
                    }
                    let result = code_from_params(&params, state);
                    match &result {
                        Ok(_) => {
                            respond(
                                &mut stream,
                                200,
                                "Signed in. You can close this tab and return to the terminal.",
                            )
                            .await
                        }
                        Err(_) => {
                            respond(
                                &mut stream,
                                400,
                                "Sign-in failed. Return to the terminal for details.",
                            )
                            .await
                        }
                    }
                    return result;
                }
            }
        }
    }
}

type Request = (TcpStream, HashMap<String, String>);

async fn read_callback(mut stream: TcpStream, callbacks: mpsc::Sender<Request>) {
    let Ok(Some(target)) = tokio::time::timeout(READ_TIMEOUT, read_target(&mut stream)).await
    else {
        return;
    };
    match callback_params(&target) {
        Some(params) => {
            let _ = callbacks.send((stream, params)).await;
        }
        None => respond(&mut stream, 404, "Not found.").await,
    }
}

/// The code from the address of the page the browser landed on, pasted by the user (SSH or headless sign-in).
pub fn code_from_pasted(pasted: &str, state: &str) -> Result<String> {
    let url = Url::parse(pasted.trim()).map_err(|_| YtError::Usage {
        message: "that is not the address of the page the browser landed on".into(),
    })?;
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    code_from_params(&params, state)
}

fn state_matches(params: &HashMap<String, String>, state: &str) -> bool {
    params.get("state").map(String::as_str) == Some(state)
}

/// Checks `state` before anything else, so a response that is not for this sign-in cannot show its text.
fn code_from_params(params: &HashMap<String, String>, state: &str) -> Result<String> {
    if !state_matches(params, state) {
        return Err(YtError::OAuth {
            message: "the sign-in response did not match this request".into(),
            help: Some("start `yt auth login` again".into()),
        }
        .into());
    }
    if let Some(error) = params.get("error") {
        let message = params
            .get("error_description")
            .filter(|text| !text.is_empty())
            .cloned()
            .unwrap_or_else(|| error.clone());
        return Err(YtError::OAuth {
            message,
            help: crate::auth::oauth::help_for(error),
        }
        .into());
    }
    params.get("code").cloned().ok_or_else(|| {
        YtError::OAuth {
            message: "the sign-in response had no code".into(),
            help: None,
        }
        .into()
    })
}

fn callback_params(target: &str) -> Option<HashMap<String, String>> {
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    (url.path() == CALLBACK_PATH).then(|| url.query_pairs().into_owned().collect())
}

async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 16 * 1024 {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut parts = head.lines().next()?.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    parts.next().map(str::to_string)
}

async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let body = format!("<!doctype html><meta charset=\"utf-8\"><title>yt</title><p>{message}</p>");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn send(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    async fn bound() -> (Callback, u16) {
        let callback = Callback::bind().await.unwrap();
        let port = callback.port;
        (callback, port)
    }

    #[tokio::test]
    async fn returns_the_code_and_ignores_other_paths() {
        let (callback, port) = bound().await;
        assert!(callback.redirect_uri().starts_with("http://127.0.0.1:"));
        let browser = tokio::spawn(async move {
            let favicon = send(port, "/favicon.ico").await;
            let page = send(port, "/callback?code=c0de&state=s1").await;
            (favicon, page)
        });
        let code = callback.wait("s1", Duration::from_secs(5)).await.unwrap();
        assert_eq!(code, "c0de");
        let (favicon, page) = browser.await.unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("You can close this tab"));
    }

    #[tokio::test]
    async fn requests_for_another_state_are_refused_and_the_wait_goes_on() {
        let (callback, port) = bound().await;
        let browser = tokio::spawn(async move {
            let forged = send(port, "/callback?code=f0rged&state=forged").await;
            let injected = send(port, "/callback?error=x&error_description=Injected").await;
            let page = send(port, "/callback?code=c0de&state=s1").await;
            (forged, injected, page)
        });
        let code = callback.wait("s1", Duration::from_secs(5)).await.unwrap();
        assert_eq!(code, "c0de");
        let (forged, injected, page) = browser.await.unwrap();
        assert!(forged.starts_with("HTTP/1.1 400"), "{forged}");
        assert!(injected.starts_with("HTTP/1.1 400"), "{injected}");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    }

    #[tokio::test]
    async fn an_oauth_error_in_the_redirect_is_reported() {
        let (callback, port) = bound().await;
        tokio::spawn(async move {
            send(
                port,
                "/callback?error=access_denied&error_description=User%20denied&state=s1",
            )
            .await
        });
        let error = callback
            .wait("s1", Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(format!("{error:?}").contains("User denied"));
    }

    #[tokio::test]
    async fn waiting_times_out_with_a_hint() {
        let (callback, _) = bound().await;
        let error = callback
            .wait("s1", Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(format!("{error:?}").contains("--no-browser"));
    }

    #[test]
    fn pasted_addresses() {
        assert_eq!(
            code_from_pasted("http://127.0.0.1:8765/callback?code=c0de&state=s1", "s1").unwrap(),
            "c0de"
        );
        assert!(
            code_from_pasted("http://127.0.0.1:8765/callback?code=c0de&state=s2", "s1").is_err()
        );
        assert!(code_from_pasted("not an address", "s1").is_err());
        let error = code_from_pasted(
            "http://127.0.0.1:8765/callback?error=x&error_description=Injected",
            "s1",
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not match"), "{error}");
    }

    #[tokio::test]
    async fn an_idle_connection_does_not_block_the_callback() {
        let (callback, port) = bound().await;
        let idle = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        tokio::spawn(async move { send(port, "/callback?code=c0de&state=s1").await });
        let code = callback.wait("s1", Duration::from_secs(2)).await.unwrap();
        assert_eq!(code, "c0de");
        drop(idle);
    }
}
