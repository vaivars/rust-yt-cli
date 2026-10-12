use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use miette::IntoDiagnostic as _;

use crate::env::Env;
use crate::error::Result;

/// The editor to run: `$VISUAL`, else `$EDITOR`, else `vi`, split on whitespace (`code --wait`).
pub fn command(env: &Env) -> Vec<String> {
    let words: Vec<String> = env
        .get("VISUAL")
        .or_else(|| env.get("EDITOR"))
        .unwrap_or("vi")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        vec!["vi".to_string()]
    } else {
        words
    }
}

/// Opens `initial` in the editor and returns the saved text. The buffer is a temporary Markdown file in `dir`.
pub async fn edit(env: &Env, dir: &Path, initial: &str) -> Result<String> {
    let argv = command(env);
    let dir = dir.to_path_buf();
    let initial = initial.to_string();
    tokio::task::spawn_blocking(move || -> Result<String> {
        std::fs::create_dir_all(&dir).into_diagnostic()?;
        let path = buffer(&dir, &initial).into_diagnostic()?;
        let status = Command::new(&argv[0]).args(&argv[1..]).arg(&path).status();
        let text = std::fs::read_to_string(&path);
        let _ = std::fs::remove_file(&path);
        match status {
            Ok(status) if status.success() => text.into_diagnostic(),
            Ok(status) => Err(miette::miette!(
                "the editor exited with {status}; nothing was changed"
            )),
            Err(error) => Err(miette::miette!(
                "could not start the editor '{}': {error}",
                argv[0]
            )),
        }
    })
    .await
    .into_diagnostic()?
}

/// A new file only the user can read, under a name nobody can guess or plant a link at beforehand.
fn buffer(dir: &Path, initial: &str) -> std::io::Result<PathBuf> {
    let mut random = [0u8; 8];
    rand::fill(&mut random);
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = dir.join(format!("yt-edit-{}-{suffix}.md", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?
        .write_all(initial.as_bytes())?;
    Ok(path)
}

/// An issue as an editor buffer: the summary, a blank line, the description.
pub fn join_issue(summary: &str, description: &str) -> String {
    format!("{summary}\n\n{description}")
}

/// The summary (first line) and description (the rest, without surrounding blank lines) of a buffer.
pub fn split_issue(buffer: &str) -> (String, String) {
    let (first, rest) = buffer.split_once('\n').unwrap_or((buffer, ""));
    (
        first.trim().to_string(),
        rest.trim_start_matches(['\n', '\r']).trim_end().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn script(dir: &Path, body: &str) -> String {
        let path = dir.join("editor.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[tokio::test]
    async fn the_saved_buffer_is_returned_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let editor = script(dir.path(), r#"printf 'edited\n' >> "$1""#);
        let env = Env::from_vars([("VISUAL", editor.as_str())]);
        assert_eq!(
            edit(&env, dir.path(), "first\n").await.unwrap(),
            "first\nedited\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("yt-edit-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[tokio::test]
    async fn the_buffer_is_private_to_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let editor = script(
            dir.path(),
            r#"mode=$(stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"); printf %s "$mode" > "$1""#,
        );
        let env = Env::from_vars([("VISUAL", editor.as_str())]);
        assert_eq!(edit(&env, dir.path(), "draft").await.unwrap(), "600");
    }

    #[tokio::test]
    async fn a_failing_editor_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let editor = script(dir.path(), "exit 3");
        let env = Env::from_vars([("EDITOR", editor.as_str())]);
        let error = edit(&env, dir.path(), "x").await.unwrap_err();
        assert!(error.to_string().contains("nothing was changed"), "{error}");
    }

    #[test]
    fn visual_wins_and_arguments_are_split() {
        assert_eq!(
            command(&Env::from_vars([
                ("VISUAL", "code --wait"),
                ("EDITOR", "nano")
            ])),
            ["code", "--wait"]
        );
        assert_eq!(command(&Env::from_vars([("EDITOR", "nano")])), ["nano"]);
        assert_eq!(command(&Env::from_vars([("EDITOR", "   ")])), ["vi"]);
        assert_eq!(command(&Env::default()), ["vi"]);
    }

    #[test]
    fn issue_buffers_round_trip() {
        let buffer = join_issue("Login fails", "Steps:\n1. Open\n");
        assert_eq!(buffer, "Login fails\n\nSteps:\n1. Open\n");
        assert_eq!(
            split_issue(&buffer),
            ("Login fails".into(), "Steps:\n1. Open".into())
        );
        assert_eq!(
            split_issue("  Only a summary  \n"),
            ("Only a summary".into(), String::new())
        );
        assert_eq!(
            split_issue("Title\nno blank line\n"),
            ("Title".into(), "no blank line".into())
        );
    }
}
