use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, IsTerminal, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use camino::Utf8PathBuf;
use regex::Regex;
use serde::Deserialize;

use super::{
    analyze::{self, AnalyzeOptions, AnalyzeReport, ThresholdProposal},
    changelog,
    db::DbParam as P,
    Db,
};
use crate::cli::args::parse_kv;

pub struct WalkthroughOptions {
    pub since: Option<String>,
    pub skill_md: Option<Utf8PathBuf>,
    pub interactive: bool,
    pub non_interactive: bool,
    pub decisions: Option<Utf8PathBuf>,
    pub dry_run: bool,
    pub filter_tags: Vec<(String, String)>,
    pub min_n: u32,
}

pub fn run(db: &mut Db, options: WalkthroughOptions) -> anyhow::Result<()> {
    Walkthrough::new(db, options)?.run()
}

struct Walkthrough<'db> {
    db: &'db mut Db,
    analysis: AnalyzeReport,
    since: Option<String>,
    interactive: bool,
    dry_run: bool,
    decisions: DecisionsFile,
}

#[derive(Default)]
struct DecisionsFile {
    by_trigger: BTreeMap<String, DecisionEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct DecisionEntry {
    trigger: String,
    action: DecisionAction,
    rationale: Option<String>,
    #[serde(default)]
    filter_tags: Vec<DecisionTag>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DecisionAction {
    Accept,
    Skip,
    Refine,
    Quit,
}

#[derive(Clone, Debug, Deserialize)]
struct DecisionTag {
    key: String,
    value: String,
}

enum ProposalDecision {
    Accept {
        rationale: String,
        filter_tags: Vec<(String, String)>,
    },
    Skip,
    Quit,
}

impl<'db> Walkthrough<'db> {
    fn new(db: &'db mut Db, options: WalkthroughOptions) -> anyhow::Result<Self> {
        let interactive = resolve_interactive(options.interactive, options.non_interactive)?;
        let since = resolve_since(options.since, options.skill_md.as_ref())?;
        let decisions = load_decisions(options.decisions.as_ref())?;
        let analysis = analyze::analyze(
            db,
            &AnalyzeOptions {
                filter_tags: options.filter_tags,
                trigger: None,
                min_n: options.min_n,
            },
        )?;

        Ok(Self {
            db,
            analysis,
            since,
            interactive,
            dry_run: options.dry_run,
            decisions,
        })
    }

    fn run(mut self) -> anyhow::Result<()> {
        self.present_summary();
        self.walk_proposals()?;
        self.emit_changelog()
    }

    fn present_summary(&self) {
        eprintln!("════════ Calibration walkthrough ════════");
        if self.dry_run {
            eprintln!("[DRY RUN] no proposals or decisions will be written");
        }
        eprintln!(
            "analyzed_at={} dataset_size={} min_n={}",
            self.analysis.analyzed_at, self.analysis.dataset_size, self.analysis.min_n
        );
        if self.analysis.filter_tags_applied.is_empty() {
            eprintln!("filter_tags=(none)");
        } else {
            let tags = self
                .analysis
                .filter_tags_applied
                .iter()
                .map(|tag| format!("{}={}", tag.key, tag.value))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!("filter_tags={tags}");
        }
        eprintln!(
            "{:<28} {:>5} {:>6} {:>7} {:>8}  VERDICT",
            "TRIGGER", "FIRES", "MISSES", "FIRE%", "SIGNAL"
        );
        for trigger in &self.analysis.triggers {
            eprintln!(
                "{:<28} {:>5} {:>6} {:>6.1}% {:>8}  {}",
                trigger.trigger,
                trigger.fires,
                trigger.misses,
                trigger.fire_rate * 100.0,
                format_signal(trigger.signal_rate),
                trigger.verdict
            );
        }
        if !self.analysis.skew_warnings.is_empty() {
            eprintln!("skew_warnings={}", self.analysis.skew_warnings.len());
            for warning in &self.analysis.skew_warnings {
                eprintln!(
                    "  - {} [{}={}]: global {:+.2}, band {:+.2}, fires {}",
                    warning.message,
                    warning.tag_key,
                    warning.tag_value,
                    warning.global_signal_rate,
                    warning.band_signal_rate,
                    warning.band_fires
                );
            }
        }
    }

    fn walk_proposals(&mut self) -> anyhow::Result<()> {
        if self.analysis.proposals.is_empty() {
            eprintln!("no candidates above min-n");
            return Ok(());
        }

        eprintln!("──── Proposals ────");
        let proposals = self.analysis.proposals.clone();
        for (index, proposal) in proposals.iter().enumerate() {
            eprintln!(
                "{}. trigger={}  {} -> {}  fire {:.1}%  signal {:+.2}",
                index + 1,
                proposal.trigger,
                analyze::pretty_threshold(proposal.current_threshold),
                analyze::pretty_threshold(proposal.proposed_threshold),
                proposal.fire_rate * 100.0,
                proposal.signal_rate
            );
            let decision = self.prompt_decision(proposal)?;
            match decision {
                ProposalDecision::Accept {
                    rationale,
                    filter_tags,
                } => self.apply_decision(proposal, rationale, filter_tags)?,
                ProposalDecision::Skip => {
                    eprintln!("skipped {}", proposal.trigger);
                }
                ProposalDecision::Quit => {
                    eprintln!("quit requested; leaving remaining proposals untouched");
                    break;
                }
            }
        }
        Ok(())
    }

    fn prompt_decision(&self, proposal: &ThresholdProposal) -> anyhow::Result<ProposalDecision> {
        if self.interactive {
            return prompt_interactive(proposal);
        }

        let Some(entry) = self.decisions.by_trigger.get(&proposal.trigger) else {
            return Ok(ProposalDecision::Skip);
        };
        match entry.action {
            DecisionAction::Accept => Ok(ProposalDecision::Accept {
                rationale: entry
                    .rationale
                    .clone()
                    .unwrap_or_else(|| "accepted by walkthrough".to_string()),
                filter_tags: decision_tags(&entry.filter_tags),
            }),
            DecisionAction::Skip => Ok(ProposalDecision::Skip),
            DecisionAction::Refine => {
                let filter_tags = decision_tags(&entry.filter_tags);
                if filter_tags.is_empty() {
                    bail!(
                        "decision for trigger {} uses action=refine but has no filter_tags",
                        entry.trigger
                    );
                }
                Ok(ProposalDecision::Accept {
                    rationale: entry
                        .rationale
                        .clone()
                        .unwrap_or_else(|| "accepted refined proposal by walkthrough".to_string()),
                    filter_tags,
                })
            }
            DecisionAction::Quit => Ok(ProposalDecision::Quit),
        }
    }

    fn apply_decision(
        &mut self,
        proposal: &ThresholdProposal,
        rationale: String,
        filter_tags: Vec<(String, String)>,
    ) -> anyhow::Result<()> {
        let proposal = if filter_tags.is_empty() {
            proposal.clone()
        } else {
            self.refined_proposal(&proposal.trigger, &filter_tags)?
        };

        if self.dry_run {
            eprintln!(
                "[DRY RUN] would accept {} {} -> {}",
                proposal.trigger,
                analyze::pretty_threshold(proposal.current_threshold),
                analyze::pretty_threshold(proposal.proposed_threshold)
            );
            return Ok(());
        }

        let supporting_plan_ids = serde_json::to_string(&proposal.supporting_plan_ids)
            .context("failed to serialize supporting plan ids")?;
        let filter_tags_json = filter_tags_json(&filter_tags)?;
        let now = unix_timestamp()?;
        let proposal_id = self.db.transaction(|tx| {
            let id = tx.query_one(
                "INSERT INTO calibration_proposals (
                    proposed_at,
                    trigger_name,
                    current_threshold,
                    proposed_threshold,
                    supporting_plan_ids,
                    fire_rate,
                    signal_rate,
                    filter_tags,
                    decision,
                    rationale
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9)
                RETURNING id",
                &[
                    P::from(now),
                    P::from(proposal.trigger.as_str()),
                    P::from(proposal.current_threshold),
                    P::from(proposal.proposed_threshold),
                    P::from(supporting_plan_ids.as_str()),
                    P::from(proposal.fire_rate),
                    P::from(proposal.signal_rate),
                    P::nullable_text(filter_tags_json.as_deref()),
                    P::from(rationale.as_str()),
                ],
                |row| row.get_i64(0),
            )?;
            let source = format!("proposal:{id}");
            tx.execute(
                "UPDATE calibration_proposals
                 SET decision = 'accepted', decided_at = $1, rationale = $2
                 WHERE id = $3 AND decision = 'pending'",
                &[P::from(now), P::from(rationale.as_str()), P::from(id)],
            )?;
            tx.execute(
                "INSERT INTO heuristic_thresholds (name, threshold, updated_by)
                 VALUES ($1, $2, $3)
                 ON CONFLICT(name) DO UPDATE SET
                    threshold = excluded.threshold,
                    updated_at = CURRENT_TIMESTAMP,
                    updated_by = excluded.updated_by",
                &[
                    P::from(proposal.trigger.as_str()),
                    P::from(proposal.proposed_threshold),
                    P::from(source.as_str()),
                ],
            )?;
            Ok(id)
        })?;
        eprintln!("proposal {proposal_id} accepted");
        Ok(())
    }

    fn refined_proposal(
        &self,
        trigger: &str,
        filter_tags: &[(String, String)],
    ) -> anyhow::Result<ThresholdProposal> {
        let report = analyze::analyze(
            self.db,
            &AnalyzeOptions {
                filter_tags: filter_tags.to_vec(),
                trigger: Some(trigger.to_string()),
                min_n: self.analysis.min_n,
            },
        )?;
        report.proposals.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!(
                "refined analysis for trigger {trigger} with filters {} produced no proposal",
                render_filter_tags(filter_tags)
            )
        })
    }

    fn emit_changelog(&self) -> anyhow::Result<()> {
        eprintln!(
            "──── Changelog block (paste into ai-skills SKILL.md \"Calibration changelog\") ────"
        );
        if self.dry_run {
            println!("[DRY RUN]");
        }
        changelog::run(self.db, self.since.as_deref())
    }
}

fn resolve_interactive(interactive: bool, non_interactive: bool) -> anyhow::Result<bool> {
    if interactive {
        if !io::stdin().is_terminal() {
            bail!("--interactive requires stdin to be a TTY");
        }
        return Ok(true);
    }
    if non_interactive {
        return Ok(false);
    }
    Ok(io::stdin().is_terminal())
}

fn load_decisions(path: Option<&Utf8PathBuf>) -> anyhow::Result<DecisionsFile> {
    let Some(path) = path else {
        return Ok(DecisionsFile::default());
    };
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read decisions file {path}"))?;
    let entries: Vec<DecisionEntry> = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse decisions file {path}"))?;
    let mut by_trigger = BTreeMap::new();
    for entry in entries {
        validate_decision_tags(&entry)?;
        by_trigger.insert(entry.trigger.clone(), entry);
    }
    Ok(DecisionsFile { by_trigger })
}

fn prompt_interactive(proposal: &ThresholdProposal) -> anyhow::Result<ProposalDecision> {
    loop {
        let choice = prompt("[a]ccept / [s]kip / [r]efine filter tags / [q]uit > ")?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "a" | "accept" => {
                let rationale = prompt_required("Rationale: ")?;
                return Ok(ProposalDecision::Accept {
                    rationale,
                    filter_tags: Vec::new(),
                });
            }
            "s" | "skip" => return Ok(ProposalDecision::Skip),
            "r" | "refine" => {
                let raw = prompt_required("Filter tags (k=v, comma-separated): ")?;
                let filter_tags = parse_filter_tags(&raw)?;
                let rationale = prompt_required("Rationale: ")?;
                return Ok(ProposalDecision::Accept {
                    rationale,
                    filter_tags,
                });
            }
            "q" | "quit" => return Ok(ProposalDecision::Quit),
            _ => eprintln!("expected a, s, r, or q for {}", proposal.trigger),
        }
    }
}

fn prompt(label: &str) -> anyhow::Result<String> {
    eprint!("{label}");
    io::stderr().flush().context("failed to flush prompt")?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("failed to read prompt response")?;
    Ok(line)
}

fn prompt_required(label: &str) -> anyhow::Result<String> {
    let value = prompt(label)?;
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("required prompt response was empty");
    }
    Ok(value)
}

fn parse_filter_tags(raw: &str) -> anyhow::Result<Vec<(String, String)>> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| parse_kv(value).map_err(anyhow::Error::msg))
        .collect()
}

fn decision_tags(tags: &[DecisionTag]) -> Vec<(String, String)> {
    tags.iter()
        .map(|tag| (tag.key.clone(), tag.value.clone()))
        .collect()
}

fn validate_decision_tags(entry: &DecisionEntry) -> anyhow::Result<()> {
    let mut seen = BTreeSet::new();
    for tag in &entry.filter_tags {
        parse_kv(&format!("{}={}", tag.key, tag.value)).map_err(anyhow::Error::msg)?;
        if !seen.insert((&tag.key, &tag.value)) {
            bail!(
                "duplicate filter tag {}={} in decision for {}",
                tag.key,
                tag.value,
                entry.trigger
            );
        }
    }
    Ok(())
}

fn resolve_since(
    explicit: Option<String>,
    skill_md: Option<&Utf8PathBuf>,
) -> anyhow::Result<Option<String>> {
    if let Some(since) = explicit {
        validate_iso_date(&since)?;
        return Ok(Some(since));
    }
    let Some(path) = skill_md else {
        return Ok(None);
    };
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read SKILL.md {path}"))?;
    // Strictly match changelog footer headings produced by export-changelog:
    // `### YYYY-MM-DD — ...`. Loose Markdown dates are intentionally ignored.
    let heading = Regex::new(r"^### (\d{4}-\d{2}-\d{2}) —").expect("valid changelog regex");
    let mut newest: Option<String> = None;
    for line in content.lines() {
        let Some(captures) = heading.captures(line) else {
            continue;
        };
        let date = captures[1].to_string();
        validate_iso_date(&date)?;
        if newest.as_ref().is_none_or(|current| date > *current) {
            newest = Some(date);
        }
    }
    Ok(newest)
}

fn validate_iso_date(value: &str) -> anyhow::Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
    {
        return Ok(());
    }
    bail!("--since must be an ISO date in YYYY-MM-DD form");
}

fn filter_tags_json(filter_tags: &[(String, String)]) -> anyhow::Result<Option<String>> {
    if filter_tags.is_empty() {
        return Ok(None);
    }

    let tags = filter_tags
        .iter()
        .cloned()
        .collect::<BTreeMap<String, String>>();
    serde_json::to_string(&tags)
        .map(Some)
        .context("failed to serialize filter tags")
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}

fn format_signal(signal: Option<f64>) -> String {
    signal.map_or_else(|| "n/a".to_string(), |signal| format!("{signal:+.2}"))
}

fn render_filter_tags(filter_tags: &[(String, String)]) -> String {
    if filter_tags.is_empty() {
        return "(none)".to_string();
    }
    filter_tags
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(", ")
}
