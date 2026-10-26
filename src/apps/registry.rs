/// A YouTrack app yt can use: it serves `meta/openapi.json` under its extension endpoints.
#[derive(Debug, PartialEq, Eq)]
pub struct AppEntry {
    /// The app's id in YouTrack, which is also its `yt app` command.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub homepage: &'static str,
}

pub const APPS: &[AppEntry] = &[AppEntry {
    id: "hidden-fields",
    name: "Hidden Fields API",
    description: "Read and update custom fields that are hidden from the YouTrack UI",
    homepage: "https://github.com/vaivars/yt-app-expose-fields-api",
}];

/// Words `yt app` uses itself, so no app can be named after them.
pub const RESERVED: &[&str] = &["list", "ls", "add", "remove", "help"];

pub fn find(id: &str) -> Option<&'static AppEntry> {
    APPS.iter().find(|app| app.id == id)
}

pub fn ids() -> impl Iterator<Item = &'static str> {
    APPS.iter().map(|app| app.id)
}

/// Lowercase words joined by single hyphens, like `hidden-fields`.
pub fn is_kebab(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_kebab_case_and_not_reserved() {
        for (index, app) in APPS.iter().enumerate() {
            assert!(is_kebab(app.id), "{}", app.id);
            assert!(!RESERVED.contains(&app.id), "{}", app.id);
            assert!(
                APPS[..index].iter().all(|other| other.id != app.id),
                "{}",
                app.id
            );
            assert!(app.homepage.starts_with("https://"), "{}", app.homepage);
        }
    }

    #[test]
    fn kebab_case() {
        for name in ["hidden-fields", "a", "get-issue2", "v2-api"] {
            assert!(is_kebab(name), "{name}");
        }
        for name in ["", "Hidden", "-a", "a-", "a--b", "2a", "a_b", "a b", "a.b"] {
            assert!(!is_kebab(name), "{name}");
        }
    }

    #[test]
    fn find_knows_registered_apps_only() {
        assert_eq!(
            find("hidden-fields").map(|app| app.id),
            Some("hidden-fields")
        );
        assert_eq!(find("Hidden-Fields"), None);
    }
}
