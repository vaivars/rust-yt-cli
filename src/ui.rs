use std::future::Future;
use std::io::{self, IsTerminal as _, Write as _};

use secrecy::SecretString;

use crate::env::Env;
use crate::error::{Result, YtError};

/// Writes a line to stderr with terminal control characters replaced; a closed stderr is ignored instead of panicking like `eprintln!`.
pub fn eprint_line(text: &str) {
    eprint_raw(&crate::output::printable(text));
}

/// Writes text that is already safe to show, such as a rendered error report with its colors.
pub fn eprint_raw(text: &str) {
    let _ = writeln!(io::stderr().lock(), "{text}");
}

pub struct Choice<T> {
    pub value: T,
    pub label: String,
    pub description: Option<String>,
}

/// Decides whether yt may ask questions, and asks them.
pub struct Ui {
    interactive: bool,
    assume_yes: bool,
    spinner: bool,
}

impl Ui {
    pub fn detect(env: &Env, no_input: bool, assume_yes: bool, spinner_allowed: bool) -> Self {
        let terminal = io::stdin().is_terminal() && io::stderr().is_terminal();
        let interactive = terminal && !no_input && env.get("CI").is_none();
        Self {
            interactive,
            assume_yes,
            spinner: interactive && spinner_allowed,
        }
    }

    pub fn new(interactive: bool, assume_yes: bool) -> Self {
        Self {
            interactive,
            assume_yes,
            spinner: false,
        }
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub async fn select<T: Clone + Send + 'static>(
        &self,
        title: &str,
        choices: Vec<Choice<T>>,
        flag: &str,
    ) -> Result<T> {
        self.require(title, flag)?;
        let title = title.to_string();
        blocking(move || {
            let options = choices
                .into_iter()
                .map(|choice| {
                    let option = demand::DemandOption::with_label(
                        crate::output::printable(&choice.label),
                        choice.value,
                    );
                    match &choice.description {
                        Some(description) => {
                            option.description(&crate::output::printable(description))
                        }
                        None => option,
                    }
                })
                .collect();
            demand::Select::new(title)
                .options(options)
                .filterable(true)
                .run()
        })
        .await
    }

    pub async fn input(
        &self,
        title: &str,
        description: Option<&str>,
        default: Option<&str>,
        flag: &str,
    ) -> Result<String> {
        self.require(title, flag)?;
        let title = title.to_string();
        let description = description.map(str::to_string);
        let default = default.map(str::to_string);
        let value = blocking(move || {
            let mut input = demand::Input::new(title);
            if let Some(description) = &description {
                input = input.description(description);
            }
            if let Some(default) = default {
                input = input.default_value(default);
            }
            input.run()
        })
        .await?;
        Ok(value.trim().to_string())
    }

    pub async fn secret(
        &self,
        title: &str,
        description: Option<&str>,
        flag: &str,
    ) -> Result<SecretString> {
        self.require(title, flag)?;
        let title = title.to_string();
        let description = description.map(str::to_string);
        let value = blocking(move || {
            let mut input = demand::Input::new(title).password(true);
            if let Some(description) = &description {
                input = input.description(description);
            }
            input.run()
        })
        .await?;
        Ok(SecretString::from(value.trim().to_string()))
    }

    pub async fn confirm(&self, title: &str, default: bool, flag: &str) -> Result<bool> {
        self.require(title, flag)?;
        let title = crate::output::printable(title).into_owned();
        blocking(move || demand::Confirm::new(title).selected(default).run()).await
    }

    /// Asks before something irreversible; `--yes` skips the question, and without a terminal it is required.
    pub async fn confirm_destructive(&self, action: &str) -> Result<()> {
        if self.assume_yes {
            return Ok(());
        }
        if !self.interactive {
            return Err(YtError::Usage {
                message: format!("refusing to {action} without --yes"),
            }
            .into());
        }
        if self
            .confirm(&format!("Really {action}?"), false, "--yes")
            .await?
        {
            Ok(())
        } else {
            Err(YtError::Cancelled.into())
        }
    }

    pub async fn with_spinner<F: Future>(&self, title: &str, future: F) -> F::Output {
        if !self.spinner {
            return future.await;
        }
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let title = title.to_string();
        let spinner = tokio::task::spawn_blocking(move || {
            demand::Spinner::new(title).run(move |_| {
                let _ = wait.recv();
            })
        });
        let output = future.await;
        drop(done);
        let _ = spinner.await;
        output
    }

    fn require(&self, what: &str, flag: &str) -> Result<()> {
        if self.interactive {
            Ok(())
        } else {
            Err(YtError::InputRequired {
                what: what.to_string(),
                flag: flag.to_string(),
            }
            .into())
        }
    }
}

async fn blocking<T: Send + 'static>(
    prompt: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T> {
    match tokio::task::spawn_blocking(prompt).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {
            Err(YtError::Cancelled.into())
        }
        Ok(Err(error)) => Err(miette::miette!("prompt failed: {error}")),
        Err(error) => Err(miette::miette!("prompt failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(report: &miette::Report) -> u8 {
        crate::error::exit_code(report)
    }

    #[tokio::test]
    async fn without_a_terminal_a_choice_names_its_flag() {
        let ui = Ui::new(false, false);
        let choices = vec![Choice {
            value: 1,
            label: "one".into(),
            description: None,
        }];
        let error = ui
            .select("Sign-in method", choices, "--method")
            .await
            .unwrap_err();
        assert_eq!(exit(&error), 2);
        assert!(format!("{error:?}").contains("pass --method"));
    }

    #[tokio::test]
    async fn without_a_terminal_input_and_secrets_are_refused() {
        let ui = Ui::new(false, false);
        assert_eq!(
            exit(&ui.input("URL", None, None, "<URL>").await.unwrap_err()),
            2
        );
        assert_eq!(
            exit(&ui.secret("Token", None, "--token-stdin").await.unwrap_err()),
            2
        );
    }

    #[tokio::test]
    async fn yes_skips_destructive_confirmation() {
        Ui::new(false, true)
            .confirm_destructive("remove profile acme")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn destructive_actions_need_yes_without_a_terminal() {
        let error = Ui::new(false, false)
            .confirm_destructive("remove profile acme")
            .await
            .unwrap_err();
        assert_eq!(exit(&error), 2);
        assert!(
            error
                .to_string()
                .contains("refusing to remove profile acme without --yes")
        );
    }

    #[tokio::test]
    async fn a_disabled_spinner_just_awaits() {
        assert_eq!(
            Ui::new(false, false).with_spinner("x", async { 7 }).await,
            7
        );
    }

    #[test]
    fn ci_disables_prompts() {
        let ui = Ui::detect(&Env::from_vars([("CI", "true")]), false, false, true);
        assert!(!ui.is_interactive());
    }
}
