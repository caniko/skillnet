use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    expr::fingerprint_fields,
    schema::{Card, Cooldown, Event, Tier},
    state::with_state,
    RepoFingerprint,
};

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct Payload {
    findings: Vec<Finding>,
    baseline_findings: Vec<Finding>,
    file: Option<String>,
    file_path: Option<String>,
    diff: Option<String>,
    content: Option<String>,
    new_string: Option<String>,
    tool_name: Option<String>,
    command: Option<String>,
    tool_input: Option<Value>,
    signal: Option<String>,
    signals: Vec<String>,
    fingerprint: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
struct Finding {
    #[serde(rename = "v")]
    _version: Option<u64>,
    #[serde(rename = "source")]
    _source: Option<String>,
    code: String,
    file: Option<String>,
    #[serde(rename = "line")]
    _line: Option<u64>,
    #[serde(rename = "severity")]
    _severity: Option<String>,
}

impl Payload {
    pub(crate) fn fingerprint(&self, base: RepoFingerprint) -> Result<RepoFingerprint> {
        if let Some(overrides) = &self.fingerprint {
            let mut fields = fingerprint_fields(&base)?;
            fields.extend(overrides.clone());
            serde_json::from_value(Value::Object(fields)).context("invalid fingerprint override")
        } else {
            Ok(base)
        }
    }

    fn command(&self) -> Option<Cow<'_, str>> {
        self.command.as_deref().map(Cow::Borrowed).or_else(|| {
            let input = self.tool_input.as_ref()?;
            Some(
                input
                    .get("command")
                    .and_then(Value::as_str)
                    .map_or_else(|| Cow::Owned(input.to_string()), Cow::Borrowed),
            )
        })
    }

    fn tool_text(&self, key: &str) -> Option<&str> {
        self.tool_input.as_ref()?.get(key)?.as_str()
    }
}

pub(crate) struct MatchInput<'a> {
    event: Event,
    file: Option<&'a Utf8Path>,
    findings: Vec<(&'a str, Option<&'a Utf8Path>)>,
    finding_files: Vec<&'a Utf8Path>,
    diff: Option<&'a str>,
    tool: Option<&'a str>,
    command: Option<Cow<'a, str>>,
    signals: Vec<&'a str>,
    fields: Map<String, Value>,
    audit_packs: Option<&'a [String]>,
}

impl<'a> MatchInput<'a> {
    pub(crate) fn new(
        event: Event,
        payload: &'a Payload,
        root: &Utf8Path,
        fp: &'a RepoFingerprint,
    ) -> Result<Self> {
        let relative = |path: &'a str| {
            let path = Utf8Path::new(path);
            path.strip_prefix(root)
                .or_else(|_| path.strip_prefix("."))
                .unwrap_or(path)
        };
        let file = payload
            .file
            .as_deref()
            .or(payload.file_path.as_deref())
            .or_else(|| payload.tool_text("file_path"))
            .or_else(|| payload.tool_text("path"))
            .map(relative);
        let diff = payload
            .diff
            .as_deref()
            .or(payload.content.as_deref())
            .or(payload.new_string.as_deref())
            .or_else(|| payload.tool_text("new_string"))
            .or_else(|| payload.tool_text("content"))
            .or_else(|| payload.tool_text("old_string"));
        let audit_sweep = event == Event::Audit
            && file.is_none()
            && payload.findings.is_empty()
            && diff.is_none()
            && payload.tool_name.is_none()
            && payload.signal.is_none()
            && payload.signals.is_empty();
        let key = |f: &'a Finding| (f.code.as_str(), f.file.as_deref().map(relative));
        let mut baseline = HashMap::<_, usize>::new();
        for finding in &payload.baseline_findings {
            *baseline.entry(key(finding)).or_default() += 1;
        }
        let findings = payload
            .findings
            .iter()
            .map(key)
            .filter(|f| match baseline.get_mut(f) {
                Some(count) if *count > 0 => {
                    *count -= 1;
                    false
                }
                _ => true,
            })
            .collect();
        Ok(Self {
            event,
            file,
            findings,
            finding_files: payload
                .findings
                .iter()
                .filter_map(|f| f.file.as_deref().map(relative))
                .collect(),
            diff,
            tool: payload.tool_name.as_deref(),
            command: payload.command(),
            signals: payload
                .signal
                .as_deref()
                .into_iter()
                .chain(payload.signals.iter().map(String::as_str))
                .collect(),
            fields: fingerprint_fields(fp)?,
            audit_packs: audit_sweep.then_some(fp.recommended_packs.as_slice()),
        })
    }

    pub(super) fn matches(&self, card: &Card) -> bool {
        let meta = &card.metadata;
        let on = &meta.on;
        if let Some(packs) = self.audit_packs {
            return meta.tier == Tier::Advise
                && meta.events.contains(&Event::Audit)
                && packs.contains(&meta.pack)
                && card
                    .repo_expr
                    .as_ref()
                    .is_none_or(|expr| expr.matches(&self.fields));
        }
        if !meta.events.is_empty() && !meta.events.contains(&self.event) {
            return false;
        }
        let eligible = |file: Option<&Utf8Path>| {
            (card.path.is_empty() || file.is_some_and(|p| card.path.is_match(p)))
                && !file.is_some_and(|p| card.not_path.is_match(p))
        };
        if self.findings.iter().any(|(code, file)| {
            eligible(file.or(self.file))
                && on.diagnostic.iter().any(|pattern| {
                    pattern
                        .strip_suffix('*')
                        .map_or(code == pattern, |prefix| code.starts_with(prefix))
                })
        }) {
            return true;
        }
        let eligible_context = if self.file.is_some() || self.finding_files.is_empty() {
            eligible(self.file)
        } else {
            self.finding_files.iter().any(|file| eligible(Some(file)))
        };
        if !eligible_context {
            return false;
        }
        let command = self.command.as_deref();
        let tool_matches = (!on.tool.is_empty() || !card.command.is_empty())
            && (on.tool.is_empty()
                || self.tool.is_some_and(|tool| {
                    let bare = tool
                        .rsplit_once("__")
                        .filter(|_| {
                            tool.get(..5)
                                .is_some_and(|p| p.eq_ignore_ascii_case("mcp__"))
                        })
                        .map_or(tool, |(_, bare)| bare);
                    on.tool.iter().any(|name| {
                        name.eq_ignore_ascii_case(tool) || name.eq_ignore_ascii_case(bare)
                    })
                }))
            && (card.command.is_empty() || command.is_some_and(|cmd| card.command.is_match(cmd)))
            && !command.is_some_and(|cmd| card.not_command.is_match(cmd));
        tool_matches
            || self.diff.is_some_and(|diff| card.diff.is_match(diff))
            || card
                .repo_expr
                .as_ref()
                .is_some_and(|expr| expr.matches(&self.fields))
            || self
                .signals
                .iter()
                .any(|signal| on.signal.iter().any(|value| value == signal))
    }
}

#[derive(Serialize)]
pub(crate) struct Resolution<'a> {
    pub event: Event,
    pub inject: Vec<Injection<'a>>,
    pub deny: Option<Denial<'a>>,
    pub collapsed: BTreeMap<&'a str, usize>,
    pub matched_count: usize,
    pub budget_dropped: usize,
}

#[derive(Serialize)]
pub(crate) struct Injection<'a> {
    pub card: &'a str,
    pub pack: &'a str,
    pub tier: Tier,
    pub text: &'a str,
}

#[derive(Serialize)]
pub(crate) struct Denial<'a> {
    pub card: &'a str,
    pub reason: &'a str,
}

pub(crate) struct Limits {
    pub max_cards: usize,
    pub max_tokens: usize,
}

pub(crate) fn resolve<'a>(
    cards: &'a [Card],
    input: &MatchInput<'_>,
    state_dir: Option<&Utf8Path>,
    root: &Utf8Path,
    harness: &str,
    session: Option<&str>,
    limits: Limits,
) -> Result<Resolution<'a>> {
    let mut out = Resolution {
        event: input.event,
        inject: Vec::new(),
        deny: None,
        collapsed: BTreeMap::new(),
        matched_count: 0,
        budget_dropped: 0,
    };
    let mut advice = Vec::new();
    for card in cards.iter().filter(|card| input.matches(card)) {
        out.matched_count += 1;
        match card.metadata.tier {
            Tier::Fix => {}
            Tier::Gate => {
                out.deny.get_or_insert(Denial {
                    card: &card.metadata.id,
                    reason: &card.body,
                });
            }
            Tier::Advise => advice.push(card),
        }
    }
    if advice.is_empty() {
        return Ok(out);
    }
    let needs_repo = advice
        .iter()
        .any(|card| card.metadata.cooldown == Cooldown::Repo30d);
    with_state(state_dir, root, harness, session, needs_repo, |state| {
        for card in advice {
            let meta = &card.metadata;
            let id = meta.id.as_str();
            if state.seen(id, meta.cooldown) {
                *out.collapsed.entry(id).or_default() += 1;
                continue;
            }
            let tokens = card.tokens();
            if out.inject.len() >= limits.max_cards
                || tokens > limits.max_tokens.saturating_sub(state.tokens())
            {
                out.budget_dropped += 1;
                continue;
            }
            state.record(id, meta.cooldown, tokens);
            out.inject.push(Injection {
                card: id,
                pack: &meta.pack,
                tier: meta.tier,
                text: &card.body,
            });
        }
    })?;
    Ok(out)
}
