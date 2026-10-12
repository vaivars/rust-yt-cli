use std::sync::{LazyLock, RwLock};

static SECRETS: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Remembers a secret so that `redact` hides it wherever it appears.
pub fn register(secret: &str) {
    if secret.len() < 8 {
        return;
    }
    let mut secrets = SECRETS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !secrets.iter().any(|known| known == secret) {
        secrets.push(secret.to_string());
    }
}

pub fn redact(text: &str) -> String {
    let secrets = SECRETS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    secrets.iter().fold(text.to_string(), |text, secret| {
        text.replace(secret.as_str(), "[REDACTED]")
    })
}

/// Enough of a token to recognise it, never enough to use it.
pub fn mask(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 12 {
        return "****".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_secrets_are_redacted() {
        register("perm-c2VjcmV0.dGVzdA==.abcdef123456");
        let text = redact("Bearer perm-c2VjcmV0.dGVzdA==.abcdef123456 failed");
        assert_eq!(text, "Bearer [REDACTED] failed");
    }

    #[test]
    fn short_strings_are_not_registered() {
        register("abc");
        assert_eq!(redact("abc"), "abc");
    }

    #[test]
    fn mask_keeps_four_characters_at_each_end() {
        assert_eq!(mask("perm-c2VjcmV0.abcdef123456"), "perm…3456");
        assert_eq!(mask("short"), "****");
    }
}
