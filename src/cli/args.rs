use camino::Utf8PathBuf;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "skillnet",
    version,
    about = "Reconcile and manage AI skills",
    long_about = "Reconcile AI skill directories from live agent sources into a mirror, edit mirrored skills, and sync selected mirror state back to live .agents/skills and .claude/skills directories.",
    subcommand_required = false,
    arg_required_else_help = false,
    disable_help_subcommand = true
)]
pub(super) struct Cli {
    /// Path to the skillnet TOML configuration file.
    #[arg(long, env = "SKILLNET_CONFIG", global = true)]
    pub(super) config: Option<Utf8PathBuf>,

    /// Root directory containing the global/ and projects/ mirror directories.
    #[arg(long, env = "SKILLNET_MIRROR_ROOT", global = true)]
    pub(super) mirror_root: Option<Utf8PathBuf>,

    /// Path to the skill catalog metadata configuration file.
    #[arg(long, env = "SKILLNET_CATALOG_CONFIG", global = true)]
    pub(super) catalog_config: Option<Utf8PathBuf>,

    /// Postgres URL for the calibration database.
    #[arg(long, value_name = "URL", global = true)]
    pub(super) database_url: Option<String>,

    /// Print planned filesystem changes without mutating files.
    #[arg(long, global = true)]
    pub(super) dry_run: bool,

    /// Allow mutating the mirror destination even when its Git working tree is dirty.
    #[arg(long, global = true)]
    pub(super) allow_dirty_destination: bool,

    #[command(subcommand)]
    pub(super) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum Command {
    /// Show scope divergence and catalog health.
    Status {
        /// Mirror scope to inspect. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Inspect every configured scope.
        #[arg(long)]
        all: bool,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: StatusFormat,
    },
    /// Check configured scopes for invariant violations.
    Doctor,
    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
    /// Pull from live sources, push to live targets, and inspect divergence.
    Sync {
        #[command(subcommand)]
        command: SyncCommand,
    },
    /// List, inspect, and edit mirrored skill directories.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Inspect configured mirror scopes and their live sources.
    Scope {
        #[command(subcommand)]
        command: ScopeCommand,
    },
    /// Manage configured project roots.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Generate and validate skill catalog metadata.
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    /// Record and verify multi-phase-plan calibration data.
    Calibration(CalibrationArgs),
}

#[derive(Debug, Args)]
pub(crate) struct CalibrationArgs {
    #[command(subcommand)]
    pub command: CalibrationCommand,
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(crate) enum CalibrationCommand {
    /// Read a plan sidecar and record plan metadata.
    Record { plan_dir: Utf8PathBuf },
    /// Read a plan sidecar verify section and record verification outcome.
    Verify { plan_dir: Utf8PathBuf },
    /// Parse a plan directory and create its calibration sidecar.
    Init {
        plan_dir: Utf8PathBuf,
        /// Print the generated sidecar to stdout instead of writing it.
        #[arg(long)]
        stdout: bool,
        /// Overwrite an existing sidecar while preserving id, tags, and verify data.
        #[arg(long)]
        force: bool,
    },
    /// Evaluate every catalog heuristic against a plan directory.
    Eval {
        plan_dir: Utf8PathBuf,
        /// Output format.
        #[arg(long, default_value = "json")]
        format: EvalFormat,
    },
    /// Evaluate meta-heuristics against a plan directory.
    MetaHeuristics {
        plan_dir: Utf8PathBuf,
        /// Optional sidecar path containing verify-time data.
        #[arg(long)]
        sidecar: Option<Utf8PathBuf>,
    },
    /// Print the deterministic shape hash for a plan directory.
    ShapeHash { plan_dir: Utf8PathBuf },
    /// Browse the heuristic catalog.
    Heuristics {
        #[command(subcommand)]
        command: HeuristicsCommand,
    },
    /// Run the full analyze -> propose -> decide -> changelog calibration flow.
    Walkthrough {
        /// Explicit changelog lower bound, in YYYY-MM-DD form.
        #[arg(long)]
        since: Option<String>,
        /// SKILL.md path used to auto-detect the latest changelog date.
        #[arg(long)]
        skill_md: Option<Utf8PathBuf>,
        /// Force TTY prompts.
        #[arg(long, conflicts_with = "non_interactive")]
        interactive: bool,
        /// Disable prompts and read choices from --decisions.
        #[arg(long, conflicts_with = "interactive")]
        non_interactive: bool,
        /// JSON decisions file for --non-interactive mode.
        #[arg(long, requires = "non_interactive")]
        decisions: Option<Utf8PathBuf>,
        /// Walk the flow without writing proposals or decisions.
        #[arg(long)]
        dry_run: bool,
        /// Restrict analysis to plans with this tag, as key=value. May be repeated.
        #[arg(long, value_parser = parse_kv)]
        filter_tag: Vec<(String, String)>,
        /// Minimum fired rows required before a threshold proposal is trusted.
        #[arg(long, default_value = "10")]
        min_n: u32,
    },
    /// Add user tags to a recorded plan.
    Tag {
        plan_id: String,
        /// Tags to add, as key=value. May be repeated.
        #[arg(required = true, value_parser = parse_kv)]
        tags: Vec<(String, String)>,
    },
    /// Remove user tags from a recorded plan.
    Untag {
        plan_id: String,
        /// Tags to remove, as key=value. May be repeated.
        #[arg(required = true, value_parser = parse_kv)]
        tags: Vec<(String, String)>,
    },
    /// Dump one recorded plan as JSON.
    Show { plan_id: String },
    /// Query recorded plans.
    Query {
        /// Restrict results to plans with this tag, as key=value. May be repeated.
        #[arg(long, value_parser = parse_kv)]
        tag: Vec<(String, String)>,
        /// Restrict results to plans with this trigger.
        #[arg(long)]
        trigger: Option<String>,
        /// Restrict trigger matches to fired rows.
        #[arg(long, conflicts_with = "missed")]
        fired: bool,
        /// Restrict trigger matches to missed rows.
        #[arg(long, conflicts_with = "fired")]
        missed: bool,
        /// Maximum number of plans to return.
        #[arg(long, default_value = "100")]
        limit: u32,
        /// Output format.
        #[arg(long, default_value = "table")]
        format: QueryFormat,
    },
    /// Apply pending schema migrations.
    Migrate,
    /// Vacuum the calibration database.
    Vacuum,
    /// Export all recorded plans.
    Export {
        /// Output format.
        #[arg(long, default_value = "jsonl")]
        format: ExportFormat,
        /// Write output to this path instead of stdout.
        #[arg(long)]
        out: Option<Utf8PathBuf>,
    },
    /// Analyze verified calibration rows and propose threshold changes.
    Analyze {
        /// Restrict analysis to plans with this tag, as key=value. May be repeated.
        #[arg(long, value_parser = parse_kv)]
        filter_tag: Vec<(String, String)>,
        /// Restrict analysis to one trigger name.
        #[arg(long)]
        trigger: Option<String>,
        /// Minimum fired rows required before a threshold proposal is trusted.
        #[arg(long, default_value = "10")]
        min_n: u32,
        /// Output format.
        #[arg(long, default_value = "table")]
        format: AnalyzeFormat,
    },
    /// Persist a calibration threshold proposal.
    Propose {
        /// Trigger name to adjust.
        #[arg(long)]
        trigger: String,
        /// Proposed replacement threshold.
        #[arg(long)]
        new_threshold: f64,
        /// Restrict supporting analysis to plans with this tag, as key=value.
        #[arg(long, value_parser = parse_kv)]
        filter_tag: Vec<(String, String)>,
        /// Human rationale for opening the proposal.
        #[arg(long)]
        rationale: String,
        /// Comma-separated supporting plan ids.
        #[arg(long, value_delimiter = ',', required = true)]
        supporting_plan_ids: Vec<String>,
    },
    /// List persisted threshold proposals.
    Proposals {
        /// Show pending proposals.
        #[arg(long, conflicts_with_all = ["accepted", "rejected"])]
        pending: bool,
        /// Show accepted proposals.
        #[arg(long, conflicts_with_all = ["pending", "rejected"])]
        accepted: bool,
        /// Show rejected proposals.
        #[arg(long, conflicts_with_all = ["pending", "accepted"])]
        rejected: bool,
    },
    /// Accept or reject a pending proposal.
    Decide {
        /// Proposal id.
        proposal_id: i64,
        /// Decision to record.
        decision: Decision,
        /// Human rationale for the decision.
        #[arg(long)]
        rationale: String,
    },
    /// Emit a SKILL.md changelog block from accepted proposals.
    ExportChangelog {
        /// Include proposals decided on or after YYYY-MM-DD.
        #[arg(long)]
        since: Option<String>,
    },
    // PHASE 04 commands here
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum EvalFormat {
    Json,
    Table,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum HeuristicsFormat {
    Json,
    Table,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum HeuristicCategoryArg {
    Coordination,
    Risk,
    PlanShape,
    QualityLint,
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(crate) enum HeuristicsCommand {
    /// List catalog heuristics.
    List {
        /// Output format.
        #[arg(long, default_value = "table")]
        format: HeuristicsFormat,
        /// Restrict to a category.
        #[arg(long)]
        category: Option<HeuristicCategoryArg>,
    },
    /// Show one catalog heuristic.
    Show { name: String },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum AnalyzeFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum QueryFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ExportFormat {
    Jsonl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum StatusFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Decision {
    Accept,
    Reject,
}

pub(crate) fn parse_kv(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| "expected key=value".to_string())?;
    if key.is_empty() {
        return Err("tag key must not be empty".to_string());
    }
    if value.is_empty() {
        return Err("tag value must not be empty".to_string());
    }
    if !valid_tag_key(key) {
        return Err(
            "tag key must match ^[a-z][a-z0-9_-]*$ (lowercase letters, numbers, '_' and '-')"
                .to_string(),
        );
    }
    Ok((key.to_string(), value.to_string()))
}

fn valid_tag_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_' | '-'))
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum SyncCommand {
    /// Read live sources and rebuild selected mirror scopes.
    Pull {
        /// Mirror scope to pull. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Pull every configured scope.
        #[arg(long)]
        all: bool,
        /// Push selected mirror scopes after a successful pull.
        #[arg(long)]
        then_push: bool,
        /// Auto-commit selected-scope dirty mirror paths with Codex before pulling.
        #[arg(long, conflicts_with = "no_auto_commit_dirty_destination")]
        auto_commit_dirty_destination: bool,
        /// Disable Codex auto-commit for this run even if config enables it.
        #[arg(long, conflicts_with = "auto_commit_dirty_destination")]
        no_auto_commit_dirty_destination: bool,
        /// Codex model to use when auto-committing a dirty destination.
        #[arg(long, value_name = "MODEL")]
        codex_model: Option<String>,
        /// Codex reasoning effort to use when auto-committing a dirty destination.
        #[arg(long, value_name = "EFFORT")]
        codex_reasoning_effort: Option<String>,
        /// Allow incoming older or equal-mtime skill content to overwrite existing skills.
        #[arg(long)]
        allow_older: bool,
        /// Allow pruning skills that are missing from the incoming side.
        #[arg(long)]
        allow_delete: bool,
    },
    /// Pull selected scopes and then push them back to live destinations.
    Roundtrip {
        /// Mirror scope to roundtrip. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Roundtrip every configured scope.
        #[arg(long)]
        all: bool,
        /// Compute would-push diffs without mutating any destination.
        /// Exits non-zero if any destination would change.
        #[arg(long)]
        check: bool,
        /// Allow incoming older or equal-mtime skill content to overwrite existing skills.
        #[arg(long)]
        allow_older: bool,
        /// Allow pruning skills that are missing from the incoming side.
        #[arg(long)]
        allow_delete: bool,
    },
    /// Write selected mirror scopes back to live .agents and .claude directories.
    Push {
        /// Mirror scope to push. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Push every configured scope.
        #[arg(long)]
        all: bool,
        /// Allow incoming older or equal-mtime skill content to overwrite existing skills.
        #[arg(long)]
        allow_older: bool,
        /// Allow pruning skills that are missing from the incoming side.
        #[arg(long)]
        allow_delete: bool,
    },
    /// Show read-only divergence for selected scopes.
    Status {
        /// Mirror scope to inspect. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Inspect every configured scope.
        #[arg(long)]
        all: bool,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: StatusFormat,
    },
    /// Show file-level mirror/live diffs for selected scopes.
    Diff {
        /// Mirror scope to diff. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Diff every configured scope.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum SkillCommand {
    /// List mirrored skills for selected scopes.
    List {
        /// Mirror scope to list. May be repeated.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// List every configured scope.
        #[arg(long)]
        all: bool,
    },
    /// Show metadata for one mirrored skill.
    Show {
        /// Skill path as <scope>/<skill>.
        path: String,
    },
    /// Delete one mirrored skill.
    Delete {
        /// Skill path as <scope>/<skill>.
        path: String,
    },
    /// Rename one mirrored skill within its current scope.
    Rename {
        /// Skill path as <scope>/<old>.
        path: String,
        /// New skill directory name.
        new: String,
    },
    /// Move one mirrored skill to another scope or scope/name destination.
    Move {
        /// Source skill path as <scope>/<skill>.
        from: String,
        /// Destination scope, optionally with a new name as <scope>/<name>.
        to: String,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum ScopeCommand {
    /// List configured mirror scopes.
    List,
    /// Print configured live source directories for one scope or all scopes.
    Sources {
        /// Mirror scope to inspect.
        #[arg(long, value_name = "SCOPE")]
        scope: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum ProjectCommand {
    /// List configured projects and their root paths.
    List,
    /// Add a configured project root.
    Add {
        /// Project name used as the mirror scope.
        name: String,
        /// Project repository root path. Relative paths are expanded before writing.
        path: Utf8PathBuf,
        /// Allow adding a project path that does not exist yet.
        #[arg(long)]
        allow_missing: bool,
    },
    /// Remove a configured project root.
    Remove {
        /// Project name to remove.
        name: String,
        /// Also delete projects/<name> from the mirror if it exists.
        #[arg(long)]
        prune_mirror: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum CatalogCommand {
    /// Rebuild CATALOG.md, ROUTING.md, SKILL_CONFLICTS.md, and project INDEX.md files.
    Generate,
    /// Validate effective catalog metadata and routing hygiene.
    Lint,
    /// Search skill names, descriptions, tags, categories, and projects.
    Search {
        /// Case-insensitive search query.
        query: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_args_read_environment_fallbacks() {
        temp_env::with_var("SKILLNET_CONFIG", Some("/tmp/foo.toml"), || {
            temp_env::with_var("SKILLNET_CATALOG_CONFIG", Some("/tmp/catalog.toml"), || {
                temp_env::with_var("SKILLNET_MIRROR_ROOT", Some("/tmp/mirror"), || {
                    let cli = Cli::parse_from(["skillnet", "status"]);

                    assert_eq!(cli.config, Some(Utf8PathBuf::from("/tmp/foo.toml")));
                    assert_eq!(
                        cli.catalog_config,
                        Some(Utf8PathBuf::from("/tmp/catalog.toml"))
                    );
                    assert_eq!(cli.mirror_root, Some(Utf8PathBuf::from("/tmp/mirror")));
                });
            });
        });
    }

    #[test]
    fn path_args_prefer_flags_over_environment() {
        temp_env::with_var("SKILLNET_CONFIG", Some("/tmp/foo.toml"), || {
            temp_env::with_var("SKILLNET_CATALOG_CONFIG", Some("/tmp/catalog.toml"), || {
                temp_env::with_var("SKILLNET_MIRROR_ROOT", Some("/tmp/mirror"), || {
                    let cli = Cli::parse_from([
                        "skillnet",
                        "--config",
                        "/tmp/bar.toml",
                        "--catalog-config",
                        "/tmp/flag-catalog.toml",
                        "--mirror-root",
                        "/tmp/flag-mirror",
                        "status",
                    ]);

                    assert_eq!(cli.config, Some(Utf8PathBuf::from("/tmp/bar.toml")));
                    assert_eq!(
                        cli.catalog_config,
                        Some(Utf8PathBuf::from("/tmp/flag-catalog.toml"))
                    );
                    assert_eq!(cli.mirror_root, Some(Utf8PathBuf::from("/tmp/flag-mirror")));
                });
            });
        });
    }

    #[test]
    fn path_args_have_no_literal_defaults_without_flags_or_environment() {
        temp_env::with_var("SKILLNET_CONFIG", None::<&str>, || {
            temp_env::with_var("SKILLNET_CATALOG_CONFIG", None::<&str>, || {
                temp_env::with_var("SKILLNET_MIRROR_ROOT", None::<&str>, || {
                    let cli = Cli::parse_from(["skillnet", "status"]);

                    assert_eq!(cli.config, None);
                    assert_eq!(cli.catalog_config, None);
                    assert_eq!(cli.mirror_root, None);
                });
            });
        });
    }
}
