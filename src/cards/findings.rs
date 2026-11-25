use std::sync::LazyLock;

use anyhow::{Context, Result};
use clap::ValueEnum;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum FindingFormat {
    Auto,
    CargoJson,
    CargoText,
    Jscpd,
}

// Older trigger clients may send only a code (and optionally a file).
#[derive(Deserialize, Serialize)]
pub(super) struct Finding {
    v: Option<u64>,
    source: Option<String>,
    pub(super) code: String,
    pub(super) file: Option<String>,
    line: Option<u64>,
    severity: Option<String>,
}

impl Finding {
    fn new(source: &str, code: String, file: Option<&str>, line: u64, severity: &str) -> Self {
        Self {
            v: Some(1),
            source: Some(source.to_owned()),
            code,
            file: file.map(str::to_owned),
            line: Some(line),
            severity: Some(severity.to_owned()),
        }
    }

    fn cargo(code: &str, file: Option<&str>, line: u64, severity: &str) -> Self {
        let (source, code) = if code.starts_with("clippy::") {
            ("clippy", code.to_owned())
        } else {
            (
                "rustc",
                format!("rustc::{}", code.strip_prefix("rustc::").unwrap_or(code)),
            )
        };
        Self::new(source, code, file, line, severity)
    }
}

#[derive(Serialize)]
pub(crate) struct Findings {
    findings: Vec<Finding>,
}

pub(crate) fn extract(text: &str, format: FindingFormat) -> Result<Findings> {
    let findings = match format {
        FindingFormat::Auto => {
            if let Ok(report) = serde_json::from_str::<Value>(text) {
                if report.get("duplicates").is_some() || report.get("findings").is_some() {
                    return Ok(Findings {
                        findings: jscpd(&report),
                    });
                }
            }
            let (is_cargo_json, findings) = cargo_json(text);
            if is_cargo_json {
                findings
            } else {
                cargo_text(text)
            }
        }
        FindingFormat::CargoJson => cargo_json(text).1,
        FindingFormat::CargoText => cargo_text(text),
        FindingFormat::Jscpd => {
            jscpd(&serde_json::from_str(text).context("invalid jscpd report JSON")?)
        }
    };
    Ok(Findings { findings })
}

pub(super) const TOOL_TEXT_FIELDS: &[&str] =
    &["stdout", "stderr", "output", "content", "message", "text"];

pub(super) fn extract_tool_value(value: &Value, out: &mut Vec<Finding>) {
    match value {
        Value::String(text) => {
            // OpenCode and Claude may wrap both Cargo stdout and stderr in one string.
            out.extend(cargo_json(text).1);
            out.extend(cargo_text(text));
        }
        Value::Array(values) => {
            for value in values {
                extract_tool_value(value, out);
            }
        }
        Value::Object(fields) => {
            for field in TOOL_TEXT_FIELDS {
                if let Some(value) = fields.get(*field) {
                    extract_tool_value(value, out);
                }
            }
        }
        _ => {}
    }
}

fn cargo_json(text: &str) -> (bool, Vec<Finding>) {
    let mut seen = false;
    let findings = text
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('{'))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|row| {
            if row.get("reason")?.as_str()? != "compiler-message" {
                return None;
            }
            seen = true;
            let message = row.get("message")?;
            let code = message.pointer("/code/code")?.as_str()?;
            let severity = severity(message.get("level")?.as_str()?)?;
            let spans = message.get("spans")?.as_array()?;
            let span = spans
                .iter()
                .find(|span| span["is_primary"] == true)
                .or_else(|| spans.first())?;
            Some(Finding::cargo(
                code,
                Some(span.get("file_name")?.as_str()?),
                span.get("line_start")?.as_u64()?,
                severity,
            ))
        })
        .collect();
    (seen, findings)
}

fn severity(level: &str) -> Option<&'static str> {
    match level {
        "error" | "ERROR" => Some("error"),
        "warning" | "WARN" | "WARNING" => Some("warn"),
        _ => None,
    }
}

#[derive(Default)]
struct Diagnostic<'a> {
    code: Option<String>,
    span: Option<(&'a str, u64)>,
    severity: Option<&'static str>,
    lint_definition: bool,
}

impl Diagnostic<'_> {
    fn finish(self) -> Option<Finding> {
        let (file, line) = self.span?;
        Some(Finding::cargo(
            &self.code?,
            Some(file),
            line,
            self.severity?,
        ))
    }
}

fn cargo_text(text: &str) -> Vec<Finding> {
    static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    static HEADER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*(error|warning)(?:\[([\w:]+)\])?:").unwrap());
    static SPAN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*-->\s+(.+):(\d+):\d+\s*$").unwrap());
    static LINT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"#!?\[(?:warn|deny|forbid)\(\s*([\w:]+)\s*\)\]|requested on the command line with `-[DWF] ([\w:-]+)`").unwrap()
    });
    static LSP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s*(ERROR|WARN|WARNING)\s+\[(\d+):\d+\]\s+\[([\w:]+)\]").unwrap()
    });

    let text = COLOR.replace_all(text, "");
    let mut out = Vec::new();
    let mut diagnostic = Diagnostic::default();
    let mut lsp_file = None;
    for line in text.lines() {
        if let Some(path) = line
            .trim()
            .strip_prefix(r#"<diagnostics file=""#)
            .and_then(|line| line.strip_suffix(r#"">"#))
        {
            lsp_file = Some(path);
        } else if line.trim() == "</diagnostics>" {
            lsp_file = None;
        }
        if let Some(c) = LSP.captures(line) {
            out.extend(std::mem::take(&mut diagnostic).finish());
            if let (Ok(line), Some(severity)) = (c[2].parse(), severity(&c[1])) {
                out.push(Finding::cargo(&c[3], lsp_file, line, severity));
            }
            continue;
        }
        let header = HEADER.captures(line);
        if let Some(c) = &header {
            out.extend(std::mem::take(&mut diagnostic).finish());
            diagnostic.severity = severity(&c[1]);
            diagnostic.code = c.get(2).map(|code| code.as_str().to_owned());
        }
        if diagnostic.severity.is_none() {
            continue;
        }
        if diagnostic.span.is_none() {
            if let Some(c) = SPAN.captures(line) {
                let (_, [file, line]) = c.extract();
                diagnostic.span = line.parse().ok().map(|line| (file, line));
            }
        }
        let trimmed = line.trim_start();
        let note = trimmed.strip_prefix("= ").unwrap_or(trimmed);
        diagnostic.lint_definition |= note.starts_with("note: the lint level is defined here");
        if diagnostic.code.is_none()
            && (header.is_some()
                || note.starts_with("note:")
                || trimmed.starts_with("#[")
                || trimmed.starts_with("#![")
                || diagnostic.lint_definition)
        {
            if let Some(c) = LINT.captures(line) {
                diagnostic.code = c
                    .get(1)
                    .or_else(|| c.get(2))
                    .map(|code| code.as_str().replace('-', "_"));
            }
        }
    }
    out.extend(diagnostic.finish());
    out
}

fn jscpd(report: &Value) -> Vec<Finding> {
    fn position(file: &Value) -> Option<(&str, u64)> {
        Some((file.get("name")?.as_str()?, file.get("start")?.as_u64()?))
    }

    let rows = |key: &str| {
        report
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let make = |category: &str, file: &str, line| {
        Finding::new(
            "jscpd",
            format!("jscpd::{category}"),
            Some(file),
            line,
            "warn",
        )
    };
    let mut out = Vec::new();
    for row in rows("duplicates") {
        let first = position(&row["firstFile"]);
        let second = position(&row["secondFile"]);
        let other = second.filter(|second| Some(second) != first.as_ref());
        for (file, line) in first.into_iter().chain(other) {
            out.push(make("clone", file, line));
        }
    }
    for row in rows("findings") {
        if let (Some(category), Some(path), Some(line)) = (
            row["category"].as_str(),
            row["path"].as_str(),
            row["start"]["line"].as_u64(),
        ) {
            out.push(make(category, path, line));
        }
    }
    out
}

#[test]
fn denied_source_lint_uses_the_original_span_not_the_lint_definition() {
    let text = "\x1b[1merror\x1b[0m: unused\n --> src/lib.rs:2:4\n\
        note: the lint level is defined here\n --> src/lib.rs:1:9\n\
        1 | #![deny(dead_code)]\n";
    let out = cargo_text(text);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].code, "rustc::dead_code");
    assert_eq!(out[0].line, Some(2));
    assert_eq!(out[0].severity.as_deref(), Some("error"));
}

#[test]
fn auto_recognizes_cargo_json_even_when_the_diagnostic_has_no_code() {
    let text = "error[E0382]: moved\n --> src/lib.rs:1:3\n\
        {\"reason\":\"compiler-message\",\"message\":{\"code\":null}}";
    assert!(extract(text, FindingFormat::Auto)
        .unwrap()
        .findings
        .is_empty());
}
