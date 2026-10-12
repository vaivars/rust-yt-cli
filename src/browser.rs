use crate::env::Env;

/// Whether a browser can be launched here: never over SSH, and on Linux only with a display.
pub fn can_launch(env: &Env) -> bool {
    let ssh = env.get("SSH_CONNECTION").is_some() || env.get("SSH_TTY").is_some();
    let display = cfg!(target_os = "macos")
        || env.get("DISPLAY").is_some()
        || env.get("WAYLAND_DISPLAY").is_some();
    !ssh && display
}

/// Opens `url` in the default browser; false when nothing was launched. Callers always print the URL too.
pub fn open(env: &Env, url: &str) -> bool {
    can_launch(env) && webbrowser::open(url).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_over_ssh() {
        assert!(!can_launch(&Env::from_vars([
            ("SSH_CONNECTION", "1.2.3.4 22"),
            ("DISPLAY", ":0")
        ])));
        assert!(!can_launch(&Env::from_vars([
            ("SSH_TTY", "/dev/ttys001"),
            ("DISPLAY", ":0")
        ])));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_needs_a_display() {
        assert!(!can_launch(&Env::default()));
        assert!(can_launch(&Env::from_vars([(
            "WAYLAND_DISPLAY",
            "wayland-0"
        )])));
    }
}
