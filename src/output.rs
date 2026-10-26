use std::borrow::Cow;
use std::io::{self, IsTerminal as _, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use miette::IntoDiagnostic as _;
use serde::Serialize;

use crate::error::Result;

/// How command results are printed on stdout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, usage_rs::ValueEnum)]
pub enum Format {
    /// Human-readable table
    #[default]
    Table,
    /// One JSON document
    Json,
    /// One JSON object per line, written as results arrive
    Ndjson,
}

pub struct Output {
    format: Format,
    tty: bool,
    closed: AtomicBool,
    out: Mutex<Box<dyn Write + Send>>,
}

impl Output {
    pub fn stdout(format: Format) -> Self {
        Self::new(format, io::stdout().is_terminal(), Box::new(io::stdout()))
    }

    pub fn new(format: Format, tty: bool, out: Box<dyn Write + Send>) -> Self {
        Self {
            format,
            tty,
            closed: AtomicBool::new(false),
            out: Mutex::new(out),
        }
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// Whether stdout is a terminal (tables then get a header; summaries may add a total line).
    pub fn is_terminal(&self) -> bool {
        self.tty
    }

    /// True once the reader of stdout has gone away (`yt … | head`); callers stop producing output.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// One JSON document: pretty-printed, or on one line in ndjson mode.
    pub fn json<T: Serialize + ?Sized>(&self, value: &T) -> Result<()> {
        if self.format == Format::Ndjson {
            return self.ndjson(value);
        }
        let text = serde_json::to_string_pretty(value).into_diagnostic()?;
        self.emit(&escape_controls(&text))
    }

    pub fn ndjson<T: Serialize + ?Sized>(&self, value: &T) -> Result<()> {
        let text = serde_json::to_string(value).into_diagnostic()?;
        self.emit(&escape_controls(&text))
    }

    /// JSON text as the server sent it, indented; keys keep their order.
    pub fn raw_json(&self, text: &str) -> Result<()> {
        self.emit(&escape_controls(&indent_json(text)))
    }

    pub fn table(&self, table: &Table) -> Result<()> {
        if table.is_empty() {
            return Ok(());
        }
        self.write(&table.render(self.tty))
    }

    /// Writes `text` followed by a newline, with terminal control characters replaced.
    pub fn write(&self, text: &str) -> Result<()> {
        self.emit(&printable(text))
    }

    /// Writes text that yt styled itself from printable parts, such as a help page, as it is.
    pub fn styled(&self, text: &str) -> Result<()> {
        self.emit(text)
    }

    fn emit(&self, text: &str) -> Result<()> {
        if self.is_closed() {
            return Ok(());
        }
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = out
            .write_all(text.as_bytes())
            .and_then(|()| out.write_all(b"\n"))
            .and_then(|()| out.flush());
        match result {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                self.closed.store(true, Ordering::Relaxed);
                Ok(())
            }
            other => other.into_diagnostic(),
        }
    }

    /// Raw bytes (a download to stdout); a closed pipe stops output as elsewhere.
    pub fn bytes(&self, data: &[u8]) -> Result<()> {
        if self.is_closed() {
            return Ok(());
        }
        let mut out = self
            .out
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match out.write_all(data).and_then(|()| out.flush()) {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                self.closed.store(true, Ordering::Relaxed);
                Ok(())
            }
            other => other.into_diagnostic(),
        }
    }
}

pub struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new<S: Into<String>>(header: impl IntoIterator<Item = S>) -> Self {
        Self {
            header: header.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row<S: Into<String>>(&mut self, cells: impl IntoIterator<Item = S>) -> &mut Self {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn render(&self, tty: bool) -> String {
        let mut table = comfy_table::Table::new();
        table.load_style(comfy_table::presets::NOTHING);
        if tty {
            table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
            table.set_header(self.header.clone());
        }
        for row in &self.rows {
            table.add_row(row.clone());
        }
        for column in table.column_iter_mut() {
            column.set_padding((0, 2));
        }
        table
            .to_string()
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `text` safe to show in a terminal: control characters other than tab and newline become U+FFFD, and `\r\n` becomes `\n`.
pub fn printable(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_unsafe) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {}
            c if is_unsafe(c) => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

fn is_unsafe(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

/// JSON text with raw control characters written as `\u` escapes, which decode to the same value.
fn escape_controls(json: &str) -> Cow<'_, str> {
    let raw = |c: char| c.is_control() && c != '\n';
    if !json.chars().any(raw) {
        return Cow::Borrowed(json);
    }
    let mut out = String::with_capacity(json.len() + 16);
    for c in json.chars() {
        if raw(c) {
            out.push_str(&format!("\\u{:04x}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Re-indents valid JSON text the way `serde_json::to_string_pretty` does, without reordering keys.
fn indent_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                while chars.next_if(|next| next.is_whitespace()).is_some() {}
                if let Some(close) = chars.next_if(|next| matches!(next, '}' | ']')) {
                    out.push(close);
                } else {
                    depth += 1;
                    new_line(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth -= 1;
                new_line(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                new_line(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

fn new_line(out: &mut String, depth: usize) {
    out.push('\n');
    out.extend(std::iter::repeat_n("  ", depth));
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Buffer {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn sample() -> Table {
        let mut table = Table::new(["ID", "SUMMARY"]);
        table.row(["WEB-1", "Login fails"]).row(["WEB-12", "Typo"]);
        table
    }

    #[test]
    fn json_is_pretty_printed() {
        let buffer = Buffer::default();
        let output = Output::new(Format::Json, false, Box::new(buffer.clone()));
        output.json(&serde_json::json!({"id": "WEB-1"})).unwrap();
        assert_eq!(buffer.text(), "{\n  \"id\": \"WEB-1\"\n}\n");
    }

    #[test]
    fn ndjson_is_one_line_per_item() {
        let buffer = Buffer::default();
        let output = Output::new(Format::Ndjson, false, Box::new(buffer.clone()));
        output.ndjson(&serde_json::json!({"id": "WEB-1"})).unwrap();
        output.ndjson(&serde_json::json!({"id": "WEB-2"})).unwrap();
        assert_eq!(buffer.text(), "{\"id\":\"WEB-1\"}\n{\"id\":\"WEB-2\"}\n");
    }

    #[test]
    fn a_single_result_is_one_line_in_ndjson_mode() {
        let buffer = Buffer::default();
        let output = Output::new(Format::Ndjson, false, Box::new(buffer.clone()));
        output
            .json(&serde_json::json!({"operations": [{"kind": "command"}]}))
            .unwrap();
        assert_eq!(buffer.text(), "{\"operations\":[{\"kind\":\"command\"}]}\n");
    }

    #[test]
    fn raw_json_is_indented_like_serde_without_reordering() {
        let text = r#"{"z":1,"a":[],"o":{ },"s":"q\"{a, b: c}\\","n":[{"k":null},2.5e3,true]}"#;
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let indented = indent_json(text);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&indented).unwrap(),
            value
        );
        assert_eq!(
            indented,
            "{\n  \"z\": 1,\n  \"a\": [],\n  \"o\": {},\n  \"s\": \"q\\\"{a, b: c}\\\\\",\n  \"n\": [\n    {\n      \"k\": null\n    },\n    2.5e3,\n    true\n  ]\n}"
        );
    }

    #[test]
    fn piped_tables_have_no_header() {
        let buffer = Buffer::default();
        Output::new(Format::Table, false, Box::new(buffer.clone()))
            .table(&sample())
            .unwrap();
        assert_eq!(buffer.text(), "WEB-1   Login fails\nWEB-12  Typo\n");
    }

    #[test]
    fn terminal_tables_have_a_header() {
        let buffer = Buffer::default();
        Output::new(Format::Table, true, Box::new(buffer.clone()))
            .table(&sample())
            .unwrap();
        assert!(
            buffer.text().starts_with("ID      SUMMARY"),
            "{}",
            buffer.text()
        );
    }

    #[test]
    fn printable_replaces_terminal_control_characters() {
        assert_eq!(
            printable("a\u{1b}[2Jb\u{7}c\u{9b}d\u{7f}e"),
            "a\u{fffd}[2Jb\u{fffd}c\u{fffd}d\u{fffd}e"
        );
        assert_eq!(
            printable("one\r\ntwo\tthree\rfour"),
            "one\ntwo\tthree\u{fffd}four"
        );
        assert!(matches!(printable("plain ā\n"), Cow::Borrowed(_)));
    }

    #[test]
    fn text_output_is_printable() {
        let buffer = Buffer::default();
        let output = Output::new(Format::Table, true, Box::new(buffer.clone()));
        output.write("a\u{1b}]0;title\u{7}b").unwrap();
        assert_eq!(buffer.text(), "a\u{fffd}]0;title\u{fffd}b\n");
    }

    #[test]
    fn json_output_escapes_control_characters() {
        let buffer = Buffer::default();
        let output = Output::new(Format::Ndjson, false, Box::new(buffer.clone()));
        output
            .json(&serde_json::json!({"s": "a\u{9b}b\u{7f}"}))
            .unwrap();
        output.raw_json("{\"s\":\"c\u{1b}d\"}").unwrap();
        assert_eq!(
            buffer.text(),
            "{\"s\":\"a\\u009bb\\u007f\"}\n{\n  \"s\": \"c\\u001bd\"\n}\n"
        );
    }

    #[test]
    fn a_closed_pipe_is_not_an_error_and_stops_output() {
        let output = Output::new(Format::Ndjson, false, Box::new(ClosedPipe));
        output.write("first").unwrap();
        assert!(output.is_closed());
        output.write("second").unwrap();
    }
}
