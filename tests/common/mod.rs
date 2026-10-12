#![allow(
    dead_code,
    reason = "each test file uses a different subset of these helpers"
)]

use std::path::PathBuf;
use std::process::{Command, Output};

pub struct TestEnv {
    root: tempfile::TempDir,
}

impl TestEnv {
    pub fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("tempdir"),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.root.path().to_path_buf()
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.path().join("config")
    }

    pub fn write_config(&self, toml: &str) {
        std::fs::create_dir_all(self.config_dir()).unwrap();
        std::fs::write(self.config_dir().join("config.toml"), toml).unwrap();
    }

    pub fn read_config(&self) -> String {
        std::fs::read_to_string(self.config_dir().join("config.toml")).unwrap_or_default()
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yt"));
        command
            .env_clear()
            .env("HOME", self.root.path().join("home"))
            .env("YT_CONFIG_DIR", self.config_dir())
            .env("YT_CACHE_DIR", self.root.path().join("cache"))
            .env("YT_STATE_DIR", self.root.path().join("state"))
            .env("NO_COLOR", "1")
            .env("YT_NO_INPUT", "1")
            .current_dir(self.root.path());
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run yt")
    }

    pub fn run_with(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        command.args(args);
        for (key, value) in vars {
            command.env(key, value);
        }
        command.output().expect("run yt")
    }

    /// For async tests that run a mock server on the same runtime.
    pub async fn run_async(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        command.args(args);
        for (key, value) in vars {
            command.env(key, value);
        }
        tokio::task::spawn_blocking(move || command.output().expect("run yt"))
            .await
            .unwrap()
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
