use serde::de::DeserializeOwned;

/// A response type together with the `fields` selector that produces exactly what it decodes.
pub trait Projection: DeserializeOwned {
    const FIELDS: &'static str;
}

impl<T: Projection> Projection for Vec<T> {
    const FIELDS: &'static str = T::FIELDS;
}

/// Checks a selector's syntax; YouTrack drops unknown names silently, so syntax is all that can be checked here.
pub fn check_selector(selector: &str) -> std::result::Result<(), String> {
    let mut depth = 0usize;
    let mut name = String::new();
    let mut previous = ' ';
    for c in selector.chars() {
        match c {
            ',' | '(' => {
                if name.is_empty() && !(c == ',' && previous == ')') {
                    return Err(format!("empty name before '{c}' in '{selector}'"));
                }
                if c == '(' {
                    depth += 1;
                }
                name.clear();
            }
            ')' => {
                if name.is_empty() && previous != ')' {
                    return Err(format!("empty name before ')' in '{selector}'"));
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| format!("unbalanced ')' in '{selector}'"))?;
                name.clear();
            }
            c if c.is_ascii_alphanumeric() || c == '$' || c == '_' => name.push(c),
            other => return Err(format!("unexpected '{other}' in '{selector}'")),
        }
        previous = c;
    }
    if depth != 0 {
        return Err(format!("unbalanced '(' in '{selector}'"));
    }
    if name.is_empty() && previous != ')' {
        return Err(format!("empty name at the end of '{selector}'"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_selectors() {
        for selector in [
            "idReadable",
            "id,login,name",
            "project(shortName,name),customFields(name,$type,value(name,login))",
        ] {
            assert_eq!(check_selector(selector), Ok(()), "{selector}");
        }
    }

    #[test]
    fn invalid_selectors() {
        for selector in [
            "", "id,,name", "project(", "project)", "a(b))", "a b", "(name)", "a,",
        ] {
            assert!(check_selector(selector).is_err(), "{selector}");
        }
    }

    #[test]
    fn me_selector_is_valid() {
        assert_eq!(
            check_selector(<crate::api::users::Me as Projection>::FIELDS),
            Ok(())
        );
    }
}
