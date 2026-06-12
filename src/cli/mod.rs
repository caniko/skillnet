pub(crate) mod args;
mod scope;

#[allow(unused_imports)]
pub use scope::{configured_scopes, detect_from_cwd, scope_value_parser, Scope, SkillPath};

use anyhow::{Context as AnyhowContext, Result};
use camino::Utf8PathBuf;
use clap::{CommandFactory, Parser};
use clap_complete::generate;

use crate::commands::Context;
use crate::{
    catalog, commands,
    config::{
        default_catalog_config_path, default_config_path, expand_path, legacy_catalog_config_path,
        legacy_config_path, Config, DbOverrides,
    },
    link::LinkStrategy,
};

use args::{
    CatalogCommand, Cli, Command, ProjectCommand, ScopeCommand, SkillCommand, SubscriptionCommand,
    ViewCommand,
};
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
    if let Some(Command::Config { command }) = command {
        return commands::config::run(command, dry_run);
    }

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

    if let Command::Hook(args) = command {
        let target = if matches!(args.command, args::HookCommand::Ingest { .. }) {
            let database = Config::load_database_or_default(&config)?;
            Some(database.resolve_db_with_overrides(&DbOverrides { database_url })?)
        } else {
            None
        };
        return commands::hook::run(args, target);
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
            commands::doctor::run(&ctx)?;
            Ok(())
        }
        Command::View { command } => run_view_command(&ctx, command),
        Command::Sync {
            scope,
            all,
            apply_promote,
            no_promote,
            force,
            prefer,
            adopt_new,
            allow_delete,
            link,
        } => {
            let scopes = resolve_scopes(&ctx.config, &scope, all)?;
            let preference = prefer.map(|preference| match preference {
                args::PreferenceArg::View => crate::view::Preference::View,
                args::PreferenceArg::Canonical => crate::view::Preference::Canonical,
            });
            let link_strategy = link.map(link_arg_to_strategy);
            let options = crate::view::PromotionOptions {
                apply_promote,
                force_demote: force,
                prefer: preference,
                adopt_new,
                allow_delete,
                relative_links: false,
                link_strategy: link_strategy.unwrap_or(LinkStrategy::Symlink),
                project_root: None,
                link_root: None,
            };
            let exit_code = commands::sync::run(&ctx, &scopes, options, no_promote, link_strategy)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Command::Skill { command } => run_skill_command(&ctx, command),
        Command::Scope { command } => run_scope_command(&ctx, command),
        Command::Project { command } => run_project_command(&ctx, command),
        Command::Subscription { command } => run_subscription_command(&ctx, command),
        Command::Catalog { command } => run_catalog_command(&ctx, command),
        Command::Config { .. } => unreachable!("handled before config loading"),
        Command::Calibration(_) => unreachable!("handled before config loading"),
        Command::Hook(_) => unreachable!("handled before config loading"),
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
                warn_legacy_config_path(&legacy);
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
                warn_legacy_config_path(&legacy);
                return Ok(legacy);
            }

            Ok(xdg)
        }
    }
}

fn warn_legacy_config_path(path: &Utf8PathBuf) {
    eprintln!(
        "warning: using legacy working-directory config at {path};\n\
         this discovery path is deprecated and will be removed in skillnet 0.7.0.\n\
         Run `skillnet config migrate` to move it to $XDG_CONFIG_HOME/skillnet/."
    );
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

fn run_view_command(ctx: &Context, command: ViewCommand) -> Result<()> {
    match command {
        ViewCommand::Sync {
            scope,
            all,
            allow_delete,
            force,
            link,
        } => {
            resolve_view_scope(&scope, all)?;
            commands::view::sync_with_promotion(
                ctx,
                allow_delete,
                force,
                link.map(link_arg_to_strategy),
            )
        }
        ViewCommand::Status { scope, all, format } => {
            resolve_view_scope(&scope, all)?;
            commands::view::status(ctx, format)
        }
        ViewCommand::Diff { scope, all } => {
            resolve_view_scope(&scope, all)?;
            commands::view::diff(ctx)
        }
    }
}

fn resolve_view_scope(scope_args: &[String], all: bool) -> Result<()> {
    if all && !scope_args.is_empty() {
        anyhow::bail!("use either --all or --scope, not both");
    }
    if !all && scope_args.is_empty() {
        anyhow::bail!("must pass --scope global or --all");
    }
    if scope_args.iter().any(|scope| scope != "global") {
        anyhow::bail!(
            "`skillnet view` currently manages only the `global` scope; \
             selector scopes `projects` and `all` are valid for `skillnet sync`, \
             `skillnet project sync`, and `skillnet status`"
        );
    }
    Ok(())
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
        SkillCommand::New { path, no_view_sync } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::new(ctx, &skill_path, !no_view_sync)
        }
        SkillCommand::List { scope, all } => {
            let scopes = resolve_scopes(&ctx.config, &scope, all)?;
            commands::list(ctx, &scopes)
        }
        SkillCommand::Show { path } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::show(ctx, &skill_path)
        }
        SkillCommand::Delete { path, no_view_sync } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::delete(ctx, &skill_path, !no_view_sync)
        }
        SkillCommand::Rename {
            path,
            new,
            no_view_sync,
        } => {
            let skill_path = parse_skill_path(ctx, &path)?;
            commands::rename(ctx, &skill_path, &new, false, !no_view_sync)
        }
        SkillCommand::Move {
            from,
            to,
            no_view_sync,
        } => {
            let from_path = parse_skill_path(ctx, &from)?;
            let (to_scope, as_name) = parse_move_destination(ctx, &to)?;
            commands::move_skill(
                ctx,
                &from_path,
                &to_scope,
                as_name.as_deref(),
                false,
                false,
                !no_view_sync,
            )
        }
    }
}

fn run_scope_command(ctx: &Context, command: ScopeCommand) -> Result<()> {
    match command {
        ScopeCommand::List => commands::targets(ctx),
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
        ProjectCommand::Sync {
            name,
            all,
            allow_delete,
            force,
            link,
        } => commands::project_sync(
            ctx,
            &name,
            all,
            allow_delete,
            force,
            link.map(link_arg_to_strategy),
        ),
        ProjectCommand::Status { name, all, format } => {
            commands::project_status_command(ctx, &name, all, format)
        }
        ProjectCommand::Diff { name, all } => commands::project_diff_command(ctx, &name, all),
        ProjectCommand::Clone {
            all,
            dry_run,
            ssh_strict,
        } => commands::project_clone_all(ctx, all, dry_run, ssh_strict),
    }
}

fn run_subscription_command(ctx: &Context, command: SubscriptionCommand) -> Result<()> {
    match command {
        SubscriptionCommand::Sync { name, all } => commands::subscription::sync(ctx, &name, all),
    }
}

fn run_catalog_command(ctx: &Context, command: CatalogCommand) -> Result<()> {
    match command {
        CatalogCommand::Generate => catalog::generate(ctx),
        CatalogCommand::Lint => catalog::lint(ctx),
        CatalogCommand::Search { query } => catalog::search(ctx, &query),
    }
}

fn link_arg_to_strategy(link: args::LinkArg) -> LinkStrategy {
    match link {
        args::LinkArg::Symlink => LinkStrategy::Symlink,
        args::LinkArg::Hardlink => LinkStrategy::Hardlink,
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
