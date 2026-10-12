use crate::error::Result;
use crate::fields::{same, usage};

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `NAME=VALUE`, repeated for a multi-value field.
    Set(Vec<String>),
    /// `NAME+=VALUE`
    Add(String),
    /// `NAME-=VALUE`
    Remove(String),
    /// `NAME=`
    Clear,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub name: String,
    pub op: Op,
}

/// Parses `-f` arguments, merging repeated `NAME=VALUE` for one field into one set.
pub fn parse(specs: &[String]) -> Result<Vec<Change>> {
    let mut changes: Vec<Change> = Vec::new();
    for spec in specs {
        let change = parse_one(spec)?;
        let conflict = changes
            .iter()
            .filter(|other| same(&other.name, &change.name))
            .any(|other| {
                !matches!(
                    (&other.op, &change.op),
                    (Op::Set(_), Op::Set(_))
                        | (Op::Add(_) | Op::Remove(_), Op::Add(_) | Op::Remove(_))
                )
            });
        if conflict {
            return Err(usage(format!(
                "-f {}: a field is set, cleared, or changed with +=/-=, not two of these at once",
                change.name
            )));
        }
        if let Op::Set(values) = &change.op
            && let Some(Change {
                op: Op::Set(earlier),
                ..
            }) = changes
                .iter_mut()
                .find(|other| same(&other.name, &change.name))
        {
            earlier.extend(values.iter().cloned());
            continue;
        }
        changes.push(change);
    }
    Ok(changes)
}

fn parse_one(spec: &str) -> Result<Change> {
    let Some(index) = spec.find('=') else {
        return Err(usage(format!("-f {spec}: expected NAME=VALUE")));
    };
    let (head, value) = (&spec[..index], spec[index + 1..].trim());
    let (name, op) = if let Some(name) = head.strip_suffix('+') {
        (name, Op::Add(value.to_string()))
    } else if let Some(name) = head.strip_suffix('-') {
        (name, Op::Remove(value.to_string()))
    } else if value.is_empty() {
        (head, Op::Clear)
    } else {
        (head, Op::Set(vec![value.to_string()]))
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(usage(format!("-f {spec}: the field name is missing")));
    }
    if matches!(&op, Op::Add(value) | Op::Remove(value) if value.is_empty()) {
        return Err(usage(format!("-f {spec}: += and -= need a value")));
    }
    Ok(Change {
        name: name.to_string(),
        op,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn change(name: &str, op: Op) -> Change {
        Change {
            name: name.into(),
            op,
        }
    }

    #[test]
    fn each_form_parses() {
        let changes = parse(&specs(&[
            "State=In Progress",
            "Fix versions+=1.2",
            "Fix versions-=1.0",
            "Assignee=",
            "Note=a=b",
        ]))
        .unwrap();
        assert_eq!(
            changes,
            vec![
                change("State", Op::Set(vec!["In Progress".into()])),
                change("Fix versions", Op::Add("1.2".into())),
                change("Fix versions", Op::Remove("1.0".into())),
                change("Assignee", Op::Clear),
                change("Note", Op::Set(vec!["a=b".into()])),
            ]
        );
    }

    #[test]
    fn repeated_sets_of_one_field_form_one_set() {
        let changes = parse(&specs(&["Fix versions=1.0", "fix VERSIONS=1.1"])).unwrap();
        assert_eq!(
            changes,
            vec![change(
                "Fix versions",
                Op::Set(vec!["1.0".into(), "1.1".into()])
            )]
        );
    }

    #[test]
    fn set_clear_and_add_remove_do_not_mix_for_one_field() {
        assert!(parse(&specs(&["Fix versions=1.0", "Fix versions+=1.1"])).is_err());
        assert!(parse(&specs(&["Assignee=", "Assignee=bob"])).is_err());
    }

    #[test]
    fn malformed_arguments_are_usage_errors() {
        for spec in ["Priority", "=Major", "Tags+=", " +=x"] {
            let error = parse(&specs(&[spec])).unwrap_err();
            assert_eq!(crate::error::exit_code(&error), 2, "{spec}");
        }
    }
}
