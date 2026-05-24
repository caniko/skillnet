use anyhow::Result;

use super::{sync, Context};
use crate::cli::configured_scopes;

pub fn run(ctx: &Context) -> Result<()> {
    let scopes = configured_scopes(&ctx.config);
    let names = scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    println!("scopes: {} ({names})", scopes.len());

    println!();
    println!("sync:");
    for summary in sync::status_summaries(ctx, &scopes)? {
        let state = match summary.state {
            sync::ScopeState::Clean => "clean".to_string(),
            sync::ScopeState::Diverged(count) => format!("diverged ({count} files)"),
        };
        println!(
            "{}  {}  last-pulled {}",
            summary.scope,
            state,
            sync::relative_time(summary.last_pulled_at)
        );
    }

    println!();
    print_destination_health(ctx);

    println!();
    print_catalog_health(ctx);

    println!();
    print_cache_health(ctx);

    Ok(())
}

fn print_destination_health(ctx: &Context) {
    match crate::vcs::status(&ctx.mirror_root) {
        Ok(Some(status)) => {
            let state = if status.is_dirty() {
                format!("dirty ({} entries)", status.dirty_entries)
            } else {
                "clean".to_string()
            };
            let branch = status.branch.as_deref().unwrap_or("(detached)");
            let remote = status.remote.as_deref().unwrap_or("(no origin)");
            println!(
                "destination: {} git {state}, branch {branch}, origin {remote}",
                status.root
            );
        }
        Ok(None) => println!("destination: {} not a git repository", ctx.mirror_root),
        Err(err) => println!(
            "destination: {} git status unavailable: {err}",
            ctx.mirror_root
        ),
    }
}

fn print_catalog_health(ctx: &Context) {
    let skill_count = ctx
        .all_targets()
        .map(|targets| {
            targets
                .iter()
                .filter_map(|target| crate::reconcile::mirror_skill_dirs(&target.mirror_path).ok())
                .map(|skills| skills.len())
                .sum::<usize>()
        })
        .unwrap_or(0);

    match crate::catalog::lint(ctx) {
        Ok(()) => println!("catalog: {skill_count} skills, clean"),
        Err(err) => {
            let message = err.to_string();
            let issues = message
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with("catalog lint failed"))
                .count()
                .max(1);
            println!("catalog: {skill_count} skills, {issues} lint issues");
        }
    }
}

fn print_cache_health(ctx: &Context) {
    let (path, cache, modified) = sync::cache_metadata(ctx);
    if cache.stamps.is_empty() {
        println!("cache: {path} no cache yet (run `skillnet sync pull`)");
    } else {
        println!(
            "cache: {} last updated {}",
            path,
            sync::relative_time(modified)
        );
    }
}
