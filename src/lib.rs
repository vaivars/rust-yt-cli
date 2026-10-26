#![deny(unreachable_pub)]

pub mod api;
pub mod apps;
pub mod auth;
pub mod browser;
pub mod config;
pub mod credentials;
pub mod dirs;
pub mod editor;
pub mod env;
pub mod error;
pub mod fields;
pub mod http;
pub mod instance;
pub mod logger;
pub mod output;
pub mod redact;
pub mod time;
pub mod ui;

pub use error::{Result, YtError};
