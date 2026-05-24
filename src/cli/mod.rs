pub(crate) mod args;
mod scope;

#[allow(unused_imports)]
pub use scope::{configured_scopes, detect_from_cwd, scope_value_parser, Scope, SkillPath};

use anyhow::{Context as AnyhowContext, Result};
use camino::Utf8PathBuf;
use clap::{CommandFactory, Parser};
use clap_complete::generate;

use crate::{
    catalog, commands,
    config::{
        default_catalog_config_path, default_config_path, expand_path, legacy_catalog_config_path,
        legacy_config_path, Config, DbOverrides, SyncOverrides,
    },
};
use crate::{commands::Context, exit::ExitError};

use args::{CatalogCommand, Cli, Command, ProjectCommand, ScopeCommand, SkillCommand, SyncCommand};
use scope::{resolve_scope, resolve_scopes};

pub fn run() -> Result<()> {
    let Cli {
        config,
        mirror_root,
        catalog_config,
        database_url,
        dry_run,
        allow_dirty_destination,
        command,
    } = Cli::parse();
    let config = resolve_config_path(config)?;
    let catalog_config = resolve_catalog_config_path(catalog_config)?;
    let command = command.unwrap_or(Command::Status {
        scope: Vec::new(),
        all: false,
        format: args::StatusFormat::Text,
    });

    if let Command::Completions { shell } = &command {
        let mut command = Cli::command();
        let name = command.get_name().to_string();
        generate(*shell, &mut command, name, &mut std::io::stdout());
        return Ok(());
    }

    if let Command::Calibration(args) = command {
        let database = Config::load_database_or_default(&config)?;
        return commands::calibration::run(
            args,
            database.resolve_db_with_overrides(&DbOverrides { database_url })?,
        );
    }

    let ctx = Context::load(
        &config,
        mirror_root.as_ref(),
        &catalog_config,
        dry_run,
        allow_dirty_destination,
    )?;

    match command {
        Command::Status { scope, all, format } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            commands::status::run(&ctx, &scopes, format)
        }
        Command::Doctor => {
            let findings = commands::doctor::run(&ctx)?;
            if findings.is_empty() {
                Ok(())
            } else {
                Err(ExitError::parity_lint("parity lint findings").into())
            }
        }
        Command::Sync { command } => run_sync_command(&ctx, command),
        Command::Skill { command } => run_skill_command(&ctx, command),
        Command::Scope { command } => run_scope_command(&ctx, command),
        Command::Project { command } => run_project_command(&ctx, command),
        Command::Catalog { command } => run_catalog_command(&ctx, command),
        Command::Calibration(_) => unreachable!("handled before config loading"),
        Command::Completions { .. } => unreachable!("handled before config loading"),
    }
}

fn resolve_config_path(flag_or_env: Option<Utf8PathBuf>) -> Result<Utf8PathBuf> {
    match flag_or_env {
        Some(path) => Ok(path),
        None => {
            let xdg = default_config_path()?;
            if xdg.exists() {
                return Ok(xdg);
            }

            let legacy = legacy_config_path();
            if legacy.exists() {
                return Ok(legacy);
            }

            Ok(xdg)
        }
    }
}

fn resolve_catalog_config_path(flag_or_env: Option<Utf8PathBuf>) -> Result<Utf8PathBuf> {
    match flag_or_env {
        Some(path) => Ok(path),
        None => {
            let xdg = default_catalog_config_path()?;
            if xdg.exists() {
                return Ok(xdg);
            }

            let legacy = legacy_catalog_config_path();
            if legacy.exists() {
                return Ok(legacy);
            }

            Ok(xdg)
        }
    }
}

pub(crate) fn resolve_mirror_root(
    config: &Config,
    flag_or_env: Option<&Utf8PathBuf>,
) -> Result<Utf8PathBuf> {
    if let Some(path) = flag_or_env {
        return Ok(path.to_path_buf());
    }

    match config
        .skills_root
        .as_deref()
        .or(config.mirror_root.as_deref())
    {
        Some(raw) if !raw.trim().is_empty() => expand_path(raw)
            .with_context(|| format!("failed to resolve configured destination root `{raw}`")),
        _ => Ok(Utf8PathBuf::from(".")),
    }
}

fn run_sync_command(ctx: &Context, command: SyncCommand) -> Result<()> {
    match command {
        SyncCommand::Pull {
            scope,
            all,
            then_push,
            auto_commit_dirty_destination,
            no_auto_commit_dirty_destination,
            codex_model,
            codex_reasoning_effort,
            allow_older,
            allow_delete,
        } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            let auto_commit_dirty_destination = if auto_commit_dirty_destination {
                Some(true)
            } else if no_auto_commit_dirty_destination {
                Some(false)
            } else {
                None
            };
            commands::sync::pull(
                ctx,
                &scopes,
                commands::sync::PullOptions {
                    then_push,
                    sync_overrides: SyncOverrides {
                        auto_commit_dirty_destination,
                        codex_model,
                        codex_reasoning_effort,
                    },
                    write_options: crate::reconcile::WriteOptions {
                        allow_older,
                        allow_delete,
                    },
                },
            )
        }
        SyncCommand::Roundtrip {
            scope,
            all,
            check,
            allow_older,
            allow_delete,
        } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            commands::sync::roundtrip(
                ctx,
                &scopes,
                check,
                crate::reconcile::WriteOptions {
                    allow_older,
                    allow_delete,
                },
            )
        }
        SyncCommand::Push {
            scope,
            all,
            allow_older,
            allow_delete,
        } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            commands::sync::push(
                ctx,
                &scopes,
                crate::reconcile::WriteOptions {
                    allow_older,
                    allow_delete,
                },
            )
            .map_err(|error| ExitError::push(error).into())
        }
        SyncCommand::Status { scope, all, format } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            commands::sync::status(ctx, &scopes, format)
        }
        SyncCommand::Diff { scope, all } => {
            let scopes = resolve_command_scopes(&ctx.config, &scope, all)?;
            commands::sync::diff(ctx, &scopes)
        }
    }
}

fn resolve_command_scopes(config: &Config, scope_args: &[String], all: bool) -> Result<Vec<Scope>> {
    if all || !scope_args.is_empty() {
        return resolve_scopes(config, scope_args, all);
    }

    if let Some(scope) = detect_from_cwd(config) {
        eprintln!("note: defaulting to --scope {scope} --scope global (detected from cwd)");
        return Ok(vec![scope, Scope::Global]);
    }

    anyhow::bail!("must pass --scope or --all");
}

fn run_skill_command(ctx: &Context, command: SkillCommand) -> Result<()> {
    match command {
        SkillCommand::List { scope, all } => {
            let scopes = resolve_scopes(&ctx.config, &scope, all)?;
            commands::list(ctx, &scopes)
        }
        SkillCommand::Show { path } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::show(ctx, &skill_path)
        }
        SkillCommand::Delete { path } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::delete(ctx, &skill_path)
        }
        SkillCommand::Rename { path, new } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::rename(ctx, &skill_path, &new, false)
        }
        SkillCommand::Move { from, to } => {
            let from_path = parse_skill_path(ctx, &from)?;
            let (to_scope, as_name) = parse_move_destination(ctx, &to)?;
            commands::move_skill(ctx, &from_path, &to_scope, as_name.as_deref(), false, false)
        }
    }
}

fn run_scope_command(ctx: &Context, command: ScopeCommand) -> Result<()> {
    match command {
        ScopeCommand::List => commands::targets(ctx),
        ScopeCommand::Sources { scope } => {
            let scopes = match scope {
                Some(scope) => vec![resolve_scope(&ctx.config, &scope)?],
                None => configured_scopes(&ctx.config),
            };
            commands::sources(ctx, &scopes)
        }
    }
}

fn run_project_command(ctx: &Context, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::List => {
            commands::project_list(ctx);
            Ok(())
        }
        ProjectCommand::Add {
            name,
            path,
            allow_missing,
        } => commands::project_add(ctx, &name, &path, allow_missing),
        ProjectCommand::Remove { name, prune_mirror } => {
            commands::project_remove(ctx, &name, prune_mirror)
        }
    }
}

fn run_catalog_command(ctx: &Context, command: CatalogCommand) -> Result<()> {
    match command {
        CatalogCommand::Generate => catalog::generate(ctx),
        CatalogCommand::Lint => catalog::lint(ctx),
        CatalogCommand::Search { query } => catalog::search(ctx, &query),
    }
}

fn parse_skill_path(ctx: &Context, raw: &str) -> Result<SkillPath> {
    SkillPath::parse(raw, &configured_scopes(&ctx.config))
}

fn parse_move_destination(ctx: &Context, raw: &str) -> Result<(Scope, Option<String>)> {
    if raw.contains('/') {
        let skill_path = parse_skill_path(ctx, raw)?;
        Ok((skill_path.scope, Some(skill_path.skill)))
    } else {
        Ok((resolve_scope(&ctx.config, raw)?, None))
    }
}
