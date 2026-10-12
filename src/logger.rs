use std::io::Write as _;

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::env::Env;

pub fn level(verbose: u8, quiet: bool, env: &Env) -> LevelFilter {
    if let Some(level) = env
        .get("YT_LOG")
        .and_then(|value| value.parse::<LevelFilter>().ok())
    {
        return level;
    }
    match (quiet, verbose) {
        (true, _) => LevelFilter::Error,
        (false, 0) => LevelFilter::Warn,
        (false, 1) => LevelFilter::Debug,
        (false, _) => LevelFilter::Trace,
    }
}

pub fn init(level: LevelFilter) {
    if log::set_boxed_logger(Box::new(StderrLogger { level })).is_ok() {
        log::set_max_level(level);
    }
}

struct StderrLogger {
    level: LevelFilter,
}

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level && metadata.target().starts_with("yt")
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let _ = writeln!(
                std::io::stderr().lock(),
                "{}",
                line(record.level(), &record.args().to_string())
            );
        }
    }

    fn flush(&self) {}
}

fn line(level: Level, message: &str) -> String {
    let label = match level {
        Level::Error => "error",
        Level::Warn => "warning",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    };
    format!(
        "yt {label}: {}",
        crate::output::printable(&crate::redact::redact(message))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_map_to_levels() {
        let env = Env::default();
        assert_eq!(level(0, false, &env), LevelFilter::Warn);
        assert_eq!(level(1, false, &env), LevelFilter::Debug);
        assert_eq!(level(2, false, &env), LevelFilter::Trace);
        assert_eq!(level(0, true, &env), LevelFilter::Error);
    }

    #[test]
    fn yt_log_overrides_flags() {
        let env = Env::from_vars([("YT_LOG", "info")]);
        assert_eq!(level(2, false, &env), LevelFilter::Info);
    }

    #[test]
    fn lines_are_printable() {
        assert_eq!(line(Level::Info, "a\u{1b}[2Jb"), "yt info: a\u{fffd}[2Jb");
    }

    #[test]
    fn lines_are_redacted() {
        crate::redact::register("perm-bG9nZ2Vy.dGVzdA==.zzzz9999yyyy");
        let line = line(Level::Debug, "token perm-bG9nZ2Vy.dGVzdA==.zzzz9999yyyy");
        assert_eq!(line, "yt debug: token [REDACTED]");
    }
}
