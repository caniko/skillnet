use camino::Utf8PathBuf;
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "skillnet",
    version,
    about = "Manage canonical AI skill stores and derived views",
    long_about = "Manage canonical AI skill stores, materialise configured skill view symlinks, and record calibration data for multi-phase-plan.",
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
    /// Materialise and inspect global view symlinks.
    View {
        #[command(subcommand)]
        command: ViewCommand,
    },
    /// Materialise every configured global view and project view in one shot.
    Sync {
        /// Mirror scope to sync. Use `projects` for every project or `all` for global plus projects.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Sync every configured scope. Alias for `--scope all`.
        #[arg(long)]
        all: bool,
        /// Promote view-side edits back into canonical skills.
        #[arg(long)]
        apply_promote: bool,
        /// Disable promotion handling and keep legacy non-symlink errors.
        #[arg(long, conflicts_with = "apply_promote", conflicts_with = "force")]
        no_promote: bool,
        /// Destructively demote view-side content when canonical is newer.
        #[arg(long)]
        force: bool,
        /// Tie-breaker for equal-mtime or both-advanced conflicts.
        #[arg(long, value_enum)]
        prefer: Option<PreferenceArg>,
        /// Adopt view-only skills into canonical when promotion is enabled.
        #[arg(long)]
        adopt_new: bool,
        /// Remove view entries that no longer correspond to canonical skills.
        #[arg(long)]
        allow_delete: bool,
        /// Link strategy for this sync run.
        #[arg(long, value_enum)]
        link: Option<LinkArg>,
    },
    /// Manage skillnet's configuration files.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage configured project roots.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// List, inspect, and edit mirrored skill directories.
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// Inspect configured canonical scopes.
    Scope {
        #[command(subcommand)]
        command: ScopeCommand,
    },
    /// Generate and validate skill catalog metadata.
    Catalog {
        #[command(subcommand)]
        command: CatalogCommand,
    },
    /// Record and verify multi-phase-plan calibration data.
    Calibration(CalibrationArgs),
    /// Ingest shell hook payloads.
    Hook(HookArgs),
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

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(crate) enum ConfigCommand {
    /// Move cwd skillnet.toml and skillnet.catalog.toml to $XDG_CONFIG_HOME/skillnet/.
    ///
    /// This command operates on the current working directory and ignores --config
    /// and --catalog-config overrides.
    Migrate {
        /// Print decisions without touching the filesystem.
        #[arg(long)]
        dry_run: bool,
        /// Overwrite XDG when both locations exist and differ.
        #[arg(long)]
        force: bool,
        /// Delete .skillnet.toml.moved-to-xdg breadcrumb files at the rank-4 discovery location, if present, then exit.
        #[arg(long, conflicts_with_all = ["dry_run", "force"])]
        remove_breadcrumbs: bool,
    },
}

#[derive(Debug, Args)]
pub(crate) struct HookArgs {
    #[command(subcommand)]
    pub command: HookCommand,
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(crate) enum HookCommand {
    /// Read one Claude Code hook payload and record it.
    Ingest {
        /// Hook event name as Claude Code passes it, such as PostToolUse or SessionEnd.
        #[arg(long, env = "CLAUDE_HOOK_EVENT")]
        event: String,
        /// Read payload from this path instead of stdin.
        #[arg(long)]
        payload_file: Option<Utf8PathBuf>,
        /// Return a non-zero exit code on ingest errors.
        #[arg(long)]
        strict: bool,
    },
    /// Install skillnet-managed Claude Code hook entries.
    Install {
        /// Claude Code user settings file to update.
        #[arg(long, default_value = "$HOME/.claude/settings.json")]
        settings: Utf8PathBuf,
        /// Hook event names to install entries for.
        #[arg(long, value_delimiter = ',', default_value = "PostToolUse,SessionEnd")]
        events: Vec<String>,
        /// Claude Code hook matchers to install.
        #[arg(long, value_delimiter = ',', default_value = "Skill")]
        matchers: Vec<String>,
        /// Print the intended change without writing the settings file.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove only skillnet-managed Claude Code hook entries.
    Uninstall {
        /// Claude Code user settings file to update.
        #[arg(long, default_value = "$HOME/.claude/settings.json")]
        settings: Utf8PathBuf,
    },
    /// Check whether any skillnet-managed Claude Code hook entry is installed.
    Status {
        /// Claude Code user settings file to inspect.
        #[arg(long, default_value = "$HOME/.claude/settings.json")]
        settings: Utf8PathBuf,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum PreferenceArg {
    View,
    Canonical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LinkArg {
    Symlink,
    Hardlink,
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
pub(super) enum ViewCommand {
    /// Materialise global view symlinks, adopting view-only skill directories
    /// into the canonical store and back-syncing them as symlinks. Authored
    /// view content is never deleted.
    Sync {
        /// Global scope to sync. Only `global` is currently valid.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Sync every configured global view.
        #[arg(long)]
        all: bool,
        /// Remove dangling view symlinks whose canonical skill is gone. Authored
        /// non-symlink directories are adopted into canonical, never deleted.
        #[arg(long)]
        allow_delete: bool,
        /// When canonical is newer than a view directory, discard the view copy
        /// and restore the symlink (destructive demote).
        #[arg(long)]
        force: bool,
        /// Link strategy for this sync run.
        #[arg(long, value_enum)]
        link: Option<LinkArg>,
    },
    /// Show read-only global view drift.
    Status {
        /// Global scope to inspect. Only `global` is currently valid.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Inspect every configured global view.
        #[arg(long)]
        all: bool,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: StatusFormat,
    },
    /// Show global view symlink deltas.
    Diff {
        /// Global scope to diff. Only `global` is currently valid.
        #[arg(long, value_name = "SCOPE", action = ArgAction::Append)]
        scope: Vec<String>,
        /// Diff every configured global view.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum SkillCommand {
    /// Create one mirrored skill.
    New {
        /// Skill path as <scope>/<skill>.
        path: String,
        /// Do not materialise affected view symlinks after creation.
        #[arg(long)]
        no_view_sync: bool,
    },
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
        /// Do not materialise affected view symlinks after deletion.
        #[arg(long)]
        no_view_sync: bool,
    },
    /// Rename one mirrored skill within its current scope.
    Rename {
        /// Skill path as <scope>/<old>.
        path: String,
        /// New skill directory name.
        new: String,
        /// Do not materialise affected view symlinks after renaming.
        #[arg(long)]
        no_view_sync: bool,
    },
    /// Move one mirrored skill to another scope or scope/name destination.
    Move {
        /// Original skill path as <scope>/<skill>.
        from: String,
        /// Destination scope, optionally with a new name as <scope>/<name>.
        to: String,
        /// Do not materialise affected view symlinks after moving.
        #[arg(long)]
        no_view_sync: bool,
    },
}

#[derive(Debug, Subcommand)]
#[command(disable_help_subcommand = true)]
pub(super) enum ScopeCommand {
    /// List configured mirror scopes.
    List,
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
    /// Materialise configured project views and aggregators.
    Sync {
        /// Project name to sync. May be repeated.
        #[arg(long, value_name = "NAME", action = ArgAction::Append)]
        name: Vec<String>,
        /// Sync every configured project.
        #[arg(long)]
        all: bool,
        /// Remove view entries that no longer correspond to canonical skills.
        #[arg(long)]
        allow_delete: bool,
        /// Replace existing non-symlink entries in project views.
        #[arg(long)]
        force: bool,
        /// Link strategy for this sync run.
        #[arg(long, value_enum)]
        link: Option<LinkArg>,
    },
    /// Show read-only project view and aggregator drift.
    Status {
        /// Project name to inspect. May be repeated.
        #[arg(long, value_name = "NAME", action = ArgAction::Append)]
        name: Vec<String>,
        /// Inspect every configured project.
        #[arg(long)]
        all: bool,
        /// Output format.
        #[arg(long, default_value = "text")]
        format: StatusFormat,
    },
    /// Show project view and aggregator deltas.
    Diff {
        /// Project name to diff. May be repeated.
        #[arg(long, value_name = "NAME", action = ArgAction::Append)]
        name: Vec<String>,
        /// Diff every configured project.
        #[arg(long)]
        all: bool,
    },
    /// Clone every missing configured project repository.
    Clone {
        /// Clone every configured project whose path does not exist.
        #[arg(long)]
        all: bool,
        /// Print planned clones without invoking git or syncing views.
        #[arg(long)]
        dry_run: bool,
        /// Refuse HTTP(S) origins. Use --ssh-strict=false to allow them.
        #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true")]
        ssh_strict: bool,
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
    use clap::CommandFactory;

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

    #[test]
    fn sync_command_defaults_record_promotion_off() {
        let cli = Cli::parse_from(["skillnet", "sync"]);

        assert!(matches!(
            cli.command,
            Some(Command::Sync {
                scope,
                all: false,
                apply_promote: false,
                no_promote: false,
                force: false,
                prefer: None,
                adopt_new: false,
                allow_delete: false,
                link: None,
            }) if scope.is_empty()
        ));
    }

    #[test]
    fn sync_command_parses_apply_promote_and_prefer() {
        let cli = Cli::parse_from([
            "skillnet",
            "sync",
            "--apply-promote",
            "--force",
            "--prefer",
            "view",
            "--adopt-new",
            "--allow-delete",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Sync {
                scope,
                all: false,
                apply_promote: true,
                no_promote: false,
                force: true,
                prefer: Some(PreferenceArg::View),
                adopt_new: true,
                allow_delete: true,
                link: None,
            }) if scope.is_empty()
        ));
    }

    #[test]
    fn materializing_commands_parse_link_strategy() {
        let sync = Cli::parse_from(["skillnet", "sync", "--link", "hardlink"]);
        assert!(matches!(
            sync.command,
            Some(Command::Sync {
                link: Some(LinkArg::Hardlink),
                ..
            })
        ));

        let view = Cli::parse_from(["skillnet", "view", "sync", "--all", "--link", "symlink"]);
        assert!(matches!(
            view.command,
            Some(Command::View {
                command: ViewCommand::Sync {
                    link: Some(LinkArg::Symlink),
                    ..
                }
            })
        ));

        let project = Cli::parse_from([
            "skillnet", "project", "sync", "--name", "x", "--link", "hardlink",
        ]);
        assert!(matches!(
            project.command,
            Some(Command::Project {
                command: ProjectCommand::Sync {
                    link: Some(LinkArg::Hardlink),
                    ..
                }
            })
        ));
    }

    #[test]
    fn sync_command_rejects_apply_promote_with_no_promote() {
        let err = Cli::try_parse_from(["skillnet", "sync", "--apply-promote", "--no-promote"])
            .unwrap_err();
        let message = err.to_string();

        assert!(!message.is_empty());
        assert!(message.contains("--apply-promote"));
        assert!(message.contains("--no-promote"));
    }

    #[test]
    fn sync_command_rejects_no_promote_with_force() {
        let err = Cli::try_parse_from(["skillnet", "sync", "--force", "--no-promote"]).unwrap_err();
        let message = err.to_string();

        assert!(!message.is_empty());
        assert!(message.contains("--force"));
        assert!(message.contains("--no-promote"));
    }

    #[test]
    fn sync_command_accepts_global_dry_run_flag() {
        let cli = Cli::parse_from(["skillnet", "--dry-run", "sync"]);

        assert!(cli.dry_run);
        assert!(matches!(cli.command, Some(Command::Sync { .. })));
    }

    #[test]
    fn long_help_lists_sync_command() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("sync"));
        assert!(help.contains("Materialise every configured global view and project view"));
    }
}
