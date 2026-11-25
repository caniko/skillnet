use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    diff::FileDiff,
    expr::fingerprint_fields,
    findings::{extract_tool_value, Finding, TOOL_TEXT_FIELDS},
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
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl Payload {
    pub(crate) fn extract_findings(&mut self, event: Event) {
        if event != Event::PostTool || !self.findings.is_empty() {
            return;
        }
        for key in ["tool_response", "result", "error"]
            .into_iter()
            .chain(TOOL_TEXT_FIELDS.iter().copied())
        {
            if let Some(value) = self.extra.get(key) {
                extract_tool_value(value, &mut self.findings);
            }
        }
    }

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

type DiagnosticMatch<'a> = (&'a str, Option<&'a Utf8Path>);

struct ChangedInput<'a> {
    file: &'a Utf8Path,
    added: Option<&'a str>,
    findings: Vec<DiagnosticMatch<'a>>,
}

pub(crate) struct MatchInput<'a> {
    event: Event,
    file: Option<&'a Utf8Path>,
    findings: Vec<DiagnosticMatch<'a>>,
    finding_files: Vec<&'a Utf8Path>,
    changes: Vec<ChangedInput<'a>>,
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
            changes: Vec::new(),
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

    pub(crate) fn with_changes(mut self, changes: &'a [FileDiff]) -> Self {
        if changes.is_empty() {
            return self;
        }
        self.audit_packs = None;
        self.changes = changes
            .iter()
            .map(|change| ChangedInput {
                file: &change.file,
                added: (!change.added.is_empty()).then_some(change.added.as_str()),
                findings: Vec::new(),
            })
            .collect();
        if self.findings.is_empty() {
            return self;
        }
        let by_path: HashMap<_, _> = self
            .changes
            .iter()
            .enumerate()
            .map(|(index, change)| (change.file, index))
            .collect();
        self.findings.retain(|&(code, file)| {
            let Some(&index) = file.or(self.file).and_then(|path| by_path.get(path)) else {
                return true;
            };
            self.changes[index].findings.push((code, file));
            false
        });
        self
    }

    pub(super) fn matches(&self, card: &Card) -> bool {
        self.match_count(card) != 0
    }

    fn match_count(&self, card: &Card) -> usize {
        let meta = &card.metadata;
        if let Some(packs) = self.audit_packs {
            return usize::from(
                meta.tier == Tier::Advise
                    && meta.events.contains(&Event::Audit)
                    && packs.contains(&meta.pack)
                    && card
                        .repo_expr
                        .as_ref()
                        .is_none_or(|expr| expr.matches(&self.fields)),
            );
        }
        if !meta.events.is_empty() && !meta.events.contains(&self.event) {
            return 0;
        }
        if self.changes.is_empty() {
            return usize::from(self.matches_scope(card, self.file, self.diff, &self.findings));
        }
        let files = self
            .changes
            .iter()
            .filter(|change| {
                self.matches_scope(card, Some(change.file), change.added, &change.findings)
            })
            .count();
        // Findings outside the diff should still fire, but not once per unrelated file.
        files + usize::from(matches_diagnostic(card, self.file, &self.findings))
    }

    fn matches_scope(
        &self,
        card: &Card,
        file: Option<&Utf8Path>,
        diff: Option<&str>,
        findings: &[DiagnosticMatch<'_>],
    ) -> bool {
        let on = &card.metadata.on;
        if matches_diagnostic(card, file, findings) {
            return true;
        }
        let eligible_context = if file.is_some() || self.finding_files.is_empty() {
            eligible(card, file)
        } else {
            self.finding_files
                .iter()
                .any(|file| eligible(card, Some(file)))
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
            || diff.is_some_and(|diff| card.diff.is_match(diff))
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

fn eligible(card: &Card, file: Option<&Utf8Path>) -> bool {
    (card.path.is_empty() || file.is_some_and(|p| card.path.is_match(p)))
        && !file.is_some_and(|p| card.not_path.is_match(p))
}

fn matches_diagnostic(
    card: &Card,
    context_file: Option<&Utf8Path>,
    findings: &[DiagnosticMatch<'_>],
) -> bool {
    findings.iter().any(|(code, file)| {
        eligible(card, file.or(context_file))
            && card.metadata.on.diagnostic.iter().any(|pattern| {
                pattern
                    .strip_suffix('*')
                    .map_or(code == pattern, |prefix| code.starts_with(prefix))
            })
    })
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
    limits: &Limits,
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
    for card in cards {
        let matches = input.match_count(card);
        if matches == 0 {
            continue;
        }
        out.matched_count += matches;
        match card.metadata.tier {
            Tier::Fix => {}
            Tier::Gate => {
                out.deny.get_or_insert(Denial {
                    card: &card.metadata.id,
                    reason: &card.body,
                });
            }
            Tier::Advise => advice.push((card, matches)),
        }
    }
    if advice.is_empty() {
        return Ok(out);
    }
    let needs_repo = advice
        .iter()
        .any(|(card, _)| card.metadata.cooldown == Cooldown::Repo30d);
    with_state(state_dir, root, harness, session, needs_repo, |state| {
        for card in advice
            .into_iter()
            .flat_map(|(card, count)| std::iter::repeat_n(card, count))
        {
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
