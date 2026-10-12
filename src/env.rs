use std::collections::BTreeMap;
use std::path::PathBuf;

/// The process environment, read once at startup.
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    #[allow(
        clippy::disallowed_methods,
        reason = "the one place that reads the process environment"
    )]
    pub fn from_process() -> Self {
        Self::from_vars(
            std::env::vars_os().filter_map(|(key, value)| {
                Some((key.into_string().ok()?, value.into_string().ok()?))
            }),
        )
    }

    pub fn from_vars<K: Into<String>, V: Into<String>>(
        vars: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// The value of `key`; an empty value counts as unset.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    pub fn is_true(&self, key: &str) -> bool {
        self.get(key).is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "y" | "on"
            )
        })
    }

    /// An absolute path from `key`; a relative path is ignored.
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.get(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_values_count_as_unset() {
        let env = Env::from_vars([("A", ""), ("B", "x")]);
        assert_eq!(env.get("A"), None);
        assert_eq!(env.get("B"), Some("x"));
        assert_eq!(env.get("C"), None);
    }

    #[test]
    fn truthy_values() {
        let env = Env::from_vars([
            ("A", "1"),
            ("B", "Yes"),
            ("C", "on"),
            ("D", "0"),
            ("E", "false"),
        ]);
        assert!(env.is_true("A") && env.is_true("B") && env.is_true("C"));
        assert!(!env.is_true("D") && !env.is_true("E") && !env.is_true("MISSING"));
    }

    #[test]
    fn relative_paths_are_ignored() {
        let env = Env::from_vars([("ABS", "/tmp/x"), ("REL", "tmp/x")]);
        assert_eq!(env.path("ABS"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(env.path("REL"), None);
    }
}
