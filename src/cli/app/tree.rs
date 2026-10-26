use std::ffi::{OsStr, OsString};

use usage_rs::help::{self, Page, Style};
use usage_rs::spec::{ArgMeta, CommandArgs, CommandExtra, CommandMeta, FlagMeta, Spec};
use usage_rs::{Arg, ArgAction, Command, Error, Event, Flag, Parser, UnknownFlags};
use yt::YtError;
use yt::apps::openapi::{AppApi, Entity, INPUT_FLAG};
use yt::apps::request::Inputs;

use crate::cli::Globals;

pub(crate) enum Parsed {
    Help(String),
    Call(Call),
    /// The first word names no command of the app; the error is ready to report.
    UnknownCommand(miette::Report),
}

pub(crate) struct Call {
    /// Index into the app's operations.
    pub(crate) operation: usize,
    pub(crate) inputs: Inputs,
    /// The `--input` source: a file, or `-` for stdin.
    pub(crate) input: Option<String>,
}

/// Long names of the flags every yt command has; an app's flags may not reuse them.
pub(crate) fn global_longs() -> Vec<&'static str> {
    <Globals as CommandArgs>::COMMAND
        .flags
        .iter()
        .flat_map(|flag| flag.longs.iter().copied())
        .collect()
}

pub(crate) struct Tree {
    pub(crate) spec: &'static Spec<'static>,
    pub(crate) root: &'static Command<'static>,
    commands: Vec<Built>,
}

struct Built {
    command: &'static Command<'static>,
    flags: Vec<(&'static Flag<'static>, Target)>,
    args: Vec<&'static Arg<'static>>,
}

#[derive(Clone, Copy)]
enum Target {
    Query(usize),
    Body(usize),
    Input,
}

// The tables are built once per process and live until it exits.
fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

fn leak_slice<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}

fn leak_str(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

fn leak_opt(text: Option<&str>) -> Option<&'static str> {
    text.map(leak_str)
}

fn value_name(flag: &str) -> &'static str {
    leak_str(&flag.to_uppercase().replace('-', "_"))
}

pub(crate) fn build(api: &AppApi) -> Tree {
    let mut keys = 0x5954_4150_0000_0000_u64..;
    let mut key = move || keys.next().unwrap_or_default();
    let mut commands = Vec::new();
    let mut metas = Vec::new();
    for operation in &api.operations {
        let mut flags = Vec::new();
        let mut flag_metas = Vec::new();
        let mut targets = Vec::new();
        let mut add = |flag: Flag<'static>, meta: FlagMeta<'static>, target: Target| {
            let flag = leak(flag);
            flags.push(flag);
            flag_metas.push(FlagMeta { flag, ..meta });
            targets.push((flag, target));
        };
        for (index, query) in operation.flags.iter().enumerate() {
            let base = if query.switch {
                Flag::BOOL
            } else {
                Flag::VALUE
            };
            add(
                Flag {
                    key: key(),
                    name: leak_str(&query.flag),
                    longs: leak_slice(vec![leak_str(&query.flag)]),
                    allow_negative_numbers: !query.switch,
                    ..base
                },
                FlagMeta {
                    help: leak_opt(query.description.as_deref().map(first_sentence)),
                    long_help: leak_opt(query.description.as_deref()),
                    value_name: (!query.switch).then(|| value_name(&query.flag)),
                    choices: leak_slice(
                        query
                            .choices
                            .iter()
                            .map(|choice| leak_str(choice))
                            .collect(),
                    ),
                    allow_unknown_choices: true,
                    required: query.required,
                    ..FlagMeta::EMPTY
                },
                Target::Query(index),
            );
        }
        if let Some(body) = &operation.body {
            for (index, field) in body.fields.iter().enumerate() {
                let json = if field.text {
                    ""
                } else {
                    "\n\nParsed as JSON; text that is not JSON is sent as a string."
                };
                let help = field
                    .description
                    .as_deref()
                    .unwrap_or("A field of the request body");
                add(
                    Flag {
                        key: key(),
                        name: leak_str(&field.flag),
                        longs: leak_slice(vec![leak_str(&field.flag)]),
                        allow_negative_numbers: true,
                        ..Flag::VALUE
                    },
                    FlagMeta {
                        help: Some(leak_str(first_sentence(help))),
                        long_help: Some(leak_str(&format!("{help}{json}"))),
                        value_name: Some(if field.text {
                            value_name(&field.flag)
                        } else {
                            "JSON"
                        }),
                        ..FlagMeta::EMPTY
                    },
                    Target::Body(index),
                );
            }
            add(
                Flag {
                    key: key(),
                    name: INPUT_FLAG,
                    longs: &[INPUT_FLAG],
                    ..Flag::VALUE
                },
                FlagMeta {
                    help: Some("The whole request body as JSON, from a file or `-` for stdin"),
                    value_name: Some("FILE"),
                    ..FlagMeta::EMPTY
                },
                Target::Input,
            );
        }
        let mut args = Vec::new();
        let mut arg_metas = Vec::new();
        for positional in &operation.positionals {
            let arg = leak(Arg {
                key: key(),
                name: leak_str(&positional.name),
                ..Arg::REQUIRED
            });
            args.push(arg);
            // yt reads issues and articles the same way everywhere, whatever else the app accepts.
            let help = match positional.entity {
                Some(Entity::Issue) => Some("Issue ID (WEB-42) or issue URL"),
                Some(Entity::Article) => Some("Article ID (KB-A-12) or article URL"),
                _ => positional.description.as_deref().map(first_sentence),
            };
            arg_metas.push(ArgMeta {
                arg,
                help: leak_opt(help),
                ..ArgMeta::EMPTY
            });
        }
        let command = leak(Command {
            name: leak_str(&operation.command),
            flags: leak_slice(flags),
            args: leak_slice(args.clone()),
            unknown_flags: Some(UnknownFlags::Error),
            key: key(),
            ..Command::EMPTY
        });
        metas.push(leak(CommandMeta {
            cmd: command,
            about: leak_opt(operation.summary.as_deref()),
            long_about: leak_opt(
                long_about(
                    operation.summary.as_deref(),
                    operation.description.as_deref(),
                )
                .as_deref(),
            ),
            flags: leak_slice(flag_metas),
            args: leak_slice(arg_metas),
            ..CommandMeta::EMPTY
        }));
        commands.push(Built {
            command,
            flags: targets,
            args,
        });
    }
    let bin = leak_str(&format!("yt app {}", api.id));
    let root = leak(Command {
        name: bin,
        flags: <Globals as CommandArgs>::COMMAND.flags,
        subcommands: leak_slice(commands.iter().map(|built| built.command).collect()),
        unknown_flags: Some(UnknownFlags::Error),
        key: key(),
        ..Command::EMPTY
    });
    let about = match &api.title {
        Some(title) => format!("{title} {}", api.version),
        None => format!("{} {}", api.id, api.version),
    };
    let about = Some(leak_str(&about));
    let long = leak_opt(long_about(about, api.description.as_deref()).as_deref());
    let root_meta = leak(CommandMeta {
        cmd: root,
        about,
        long_about: long,
        flags: <Globals as CommandArgs>::META.flags,
        subcommands: leak_slice(metas),
        extra: leak(CommandExtra {
            subcommand_required: true,
            ..CommandExtra::EMPTY
        }),
        ..CommandMeta::EMPTY
    });
    let spec = leak(Spec {
        name: bin,
        bin: Some(bin),
        about,
        long_about: long,
        root: root_meta,
        ..Spec::EMPTY
    });
    Tree {
        spec,
        root,
        commands,
    }
}

/// The first sentence of the first line, for a one-line description.
fn first_sentence(text: &str) -> &str {
    let line = text.lines().next().unwrap_or_default();
    match line.find(". ") {
        Some(end) => &line[..=end],
        None => line,
    }
}

fn long_about(summary: Option<&str>, description: Option<&str>) -> Option<String> {
    match (summary, description) {
        (Some(summary), Some(description)) if description != summary => {
            Some(format!("{summary}\n\n{description}"))
        }
        (summary, description) => description.or(summary).map(str::to_string),
    }
}

/// Parses the words after `yt app <id>` against the app's commands.
pub(crate) fn parse(api: &AppApi, words: &[OsString]) -> yt::Result<Parsed> {
    let tree = build(api);
    let argv: Vec<&OsStr> = words.iter().map(OsString::as_os_str).collect();
    let mut parser = Parser::new(tree.root, &argv);
    let mut selected: Option<usize> = None;
    let mut inputs = Inputs::default();
    let mut input = None;
    let mut body_flag: Option<&'static str> = None;
    while let Some(event) = parser.next_event() {
        match event {
            Ok(Event::Command(command)) => {
                selected = tree
                    .commands
                    .iter()
                    .position(|built| std::ptr::eq(built.command, command));
            }
            Ok(Event::Flag { flag, .. }) if usage_rs::is_help_flag(flag) => {
                let cmd = parser.command();
                let error = match flag.action {
                    ArgAction::HelpAll => Error::HelpAll { cmd },
                    ArgAction::HelpShort => Error::Help { cmd, long: false },
                    _ => Error::Help {
                        cmd,
                        long: flag.action == ArgAction::HelpLong || !flag.longs.is_empty(),
                    },
                };
                return failure(&tree, &argv, error);
            }
            Ok(Event::Flag { flag, value, .. }) => {
                let Some(index) = selected else {
                    continue;
                };
                let Some((_, target)) = tree.commands[index]
                    .flags
                    .iter()
                    .find(|(candidate, _)| std::ptr::eq(*candidate, flag))
                else {
                    continue;
                };
                let value = match value {
                    Some(bytes) => utf8(&tree, &argv, flag.name, bytes)?,
                    None => "true".to_string(),
                };
                let operation = &api.operations[index];
                match *target {
                    Target::Query(field) => {
                        inputs
                            .query
                            .insert(operation.flags[field].flag.clone(), value);
                    }
                    Target::Body(field) => {
                        body_flag.get_or_insert(flag.name);
                        let fields = &operation.body.as_ref().map(|body| &body.fields);
                        if let Some(fields) = fields {
                            inputs.body.insert(fields[field].flag.clone(), value);
                        }
                    }
                    Target::Input => input = Some(value),
                }
            }
            Ok(Event::Arg { arg, value, .. }) => {
                inputs
                    .positionals
                    .push(utf8(&tree, &argv, arg.name, value)?);
            }
            Ok(_) => {}
            Err(error @ Error::UnexpectedArg { token })
                if selected.is_none() && !token.starts_with(b"-") =>
            {
                return Ok(Parsed::UnknownCommand(rendered(&tree, &argv, &error)));
            }
            Err(error) => return failure(&tree, &argv, error),
        }
    }
    let Some(index) = selected else {
        return failure(&tree, &argv, Error::MissingSubcommand);
    };
    let built = &tree.commands[index];
    let operation = &api.operations[index];
    let missing = |name: &'static str| failure(&tree, &argv, Error::MissingRequired { name });
    if let Some(arg) = built.args.get(inputs.positionals.len()) {
        return missing(arg.name);
    }
    let flag_named = |name: &str| {
        built
            .flags
            .iter()
            .map(|(flag, _)| flag.name)
            .find(|candidate| *candidate == name)
            .unwrap_or(INPUT_FLAG)
    };
    for query in &operation.flags {
        if query.required && !inputs.query.contains_key(&query.flag) {
            return missing(flag_named(&query.flag));
        }
    }
    if let Some(body) = &operation.body {
        match (&input, body_flag) {
            (Some(_), Some(other)) => {
                return failure(
                    &tree,
                    &argv,
                    Error::ConflictingFlags {
                        name: INPUT_FLAG,
                        other,
                    },
                );
            }
            (Some(_), None) => {}
            (None, _) => {
                for field in &body.fields {
                    if field.required && !inputs.body.contains_key(&field.flag) {
                        return missing(flag_named(&field.flag));
                    }
                }
                if body.required && body.fields.is_empty() {
                    return missing(INPUT_FLAG);
                }
            }
        }
    }
    Ok(Parsed::Call(Call {
        operation: index,
        inputs,
        input,
    }))
}

fn utf8(tree: &Tree, argv: &[&OsStr], name: &'static str, bytes: &[u8]) -> yt::Result<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => Err(rendered(
            tree,
            argv,
            &Error::InvalidValue(Box::new(usage_rs::InvalidValue {
                name,
                value: String::from_utf8_lossy(bytes).into_owned(),
                reason: "it is not valid UTF-8".into(),
            })),
        )),
    }
}

fn failure(tree: &Tree, argv: &[&OsStr], error: Error<'static, '_>) -> yt::Result<Parsed> {
    let page = match &error {
        Error::Help { cmd, long: true } => Some((*cmd, Page::Long)),
        Error::Help { cmd, long: false } => Some((*cmd, Page::Short)),
        Error::HelpAll { cmd } => Some((*cmd, Page::All)),
        _ => None,
    };
    // yt decides colour (`--no-color`, `color`, `NO_COLOR`), not usage-rs.
    let style = if console::colors_enabled() {
        Style::COLOURED
    } else {
        Style::PLAIN
    };
    match page.and_then(|(cmd, page)| help::page(tree.spec, tree.root, argv, cmd, page, style)) {
        Some(text) => Ok(Parsed::Help(text)),
        None => Err(rendered(tree, argv, &error)),
    }
}

fn rendered(tree: &Tree, argv: &[&OsStr], error: &Error<'_, '_>) -> miette::Report {
    let text = if console::colors_enabled_stderr() {
        usage_rs::render_failure(tree.spec, argv, error)
    } else {
        usage_rs::render_failure_plain(tree.spec, argv, error)
    };
    YtError::CommandLine(text).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> AppApi {
        let text = include_str!("../../../tests/fixtures/apps/hidden-fields.openapi.json");
        AppApi::parse("hidden-fields", text, &global_longs()).unwrap()
    }

    fn words(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    fn call(words_: &[&str]) -> Call {
        match parse(&api(), &words(words_)).unwrap() {
            Parsed::Call(call) => call,
            Parsed::Help(page) => panic!("{page}"),
            Parsed::UnknownCommand(error) => panic!("{error:?}"),
        }
    }

    fn error(words_: &[&str]) -> String {
        let error = match parse(&api(), &words(words_)) {
            Err(error) | Ok(Parsed::UnknownCommand(error)) => error,
            Ok(_) => panic!("{words_:?} parsed"),
        };
        match error.downcast_ref::<YtError>() {
            Some(YtError::CommandLine(text)) => console::strip_ansi_codes(text).into_owned(),
            _ => panic!("{error:?}"),
        }
    }

    fn help(words_: &[&str]) -> String {
        match parse(&api(), &words(words_)).unwrap() {
            Parsed::Help(page) => console::strip_ansi_codes(&page).into_owned(),
            _ => panic!("{words_:?} is not help"),
        }
    }

    #[test]
    fn arguments_and_flags_bind_to_the_operation() {
        let api = api();
        let parsed = call(&["get-issue", "WEB-1", "--fields", "A,B", "--full"]);
        assert_eq!(api.operations[parsed.operation].command, "get-issue");
        assert_eq!(parsed.inputs.positionals, ["WEB-1"]);
        assert_eq!(parsed.inputs.query["fields"], "A,B");
        assert_eq!(parsed.inputs.query["full"], "true");
        let parsed = call(&[
            "update-issue",
            "WEB-1",
            "--field",
            "Estimate",
            "--value",
            "-5",
        ]);
        assert_eq!(parsed.inputs.body["field"], "Estimate");
        assert_eq!(parsed.inputs.body["value"], "-5");
        let parsed = call(&["update-issue", "WEB-1", "--input", "-"]);
        assert_eq!(parsed.input.as_deref(), Some("-"));
    }

    #[test]
    fn global_flags_are_accepted_and_ignored() {
        let parsed = call(&["--format", "json", "get-issue", "-v", "WEB-1", "--quiet"]);
        assert_eq!(parsed.inputs.positionals, ["WEB-1"]);
        assert!(parsed.inputs.query.is_empty());
    }

    #[test]
    fn mistakes_are_usage_errors_in_usage_rs_style() {
        let text = error(&["get-issue", "WEB-1", "--feilds", "x"]);
        assert!(text.contains("unexpected argument '--feilds'"), "{text}");
        let text = error(&["get-issue"]);
        assert!(
            text.contains("the following required arguments were not provided")
                && text.contains("<ISSUE>"),
            "{text}"
        );
        let text = error(&["update-issue", "WEB-1", "--field", "x"]);
        assert!(text.contains("--value"), "{text}");
        let text = error(&["update-issue", "WEB-1", "--field", "x", "--input", "-"]);
        assert!(text.contains("cannot be used with"), "{text}");
        assert!(matches!(
            parse(&api(), &words(&["get-isue", "WEB-1"])),
            Ok(Parsed::UnknownCommand(_))
        ));
        let text = error(&["get-isue"]);
        assert!(
            text.contains("unrecognized subcommand 'get-isue'"),
            "{text}"
        );
        assert!(text.contains("get-issue"), "{text}");
        assert!(text.contains("Usage: yt app hidden-fields"), "{text}");
    }

    #[test]
    fn a_bare_app_shows_its_commands() {
        let text = error(&[]);
        assert!(
            text.contains("get-issue") && text.contains("update-issue"),
            "{text}"
        );
    }

    #[test]
    fn help_describes_the_commands() {
        let page = help(&["--help"]);
        assert!(page.contains("Hidden Fields API 0.0.1"), "{page}");
        assert!(
            page.contains("get-issue") && page.contains("Set one exposed field"),
            "{page}"
        );
        let page = help(&["get-issue", "--help"]);
        assert!(
            page.contains("Usage: yt app hidden-fields get-issue"),
            "{page}"
        );
        assert!(
            page.contains("<ISSUE>") && page.contains("--fields <FIELDS>"),
            "{page}"
        );
        let page = help(&["help", "update-issue"]);
        assert!(
            page.contains("--value <JSON>") && page.contains("--input <FILE>"),
            "{page}"
        );
        let short = help(&["update-issue", "-h"]);
        assert!(short.contains("--field <FIELD>"), "{short}");
    }
}
