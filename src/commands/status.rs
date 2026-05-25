use anyhow::Result;
use serde::Serialize;
use std::io::Write;

use super::Context;
use crate::{
    cli::{args::StatusFormat, Scope},
    model::{Target, TargetScope},
};

pub fn run(ctx: &Context, scopes: &[Scope], format: StatusFormat) -> Result<()> {
    let rows = status_rows(ctx, scopes)?;

    if format == StatusFormat::Json {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        serde_json::to_writer_pretty(&mut handle, &rows)?;
        writeln!(handle)?;
        return Ok(());
    }

    let names = scopes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");

    println!("scopes: {} ({names})", scopes.len());

    println!();
    println!("views:");
    for row in &rows {
        let state = if row.drift_entries == 0 {
            "clean".to_string()
        } else {
            format!("drift ({} entries)", row.drift_entries)
        };
        println!(
            "{}  {}  canonical {}  skills {}",
            row.scope, state, row.canonical_path, row.skill_count
        );
    }

    println!();
    print_destination_health(ctx);

    println!();
    print_catalog_health(ctx);

    Ok(())
}

fn status_rows(ctx: &Context, scopes: &[Scope]) -> Result<Vec<StatusRow>> {
    ctx.targets(scopes)?
        .into_iter()
        .map(status_row)
        .collect::<Result<Vec<_>>>()
}

fn status_row(target: Target) -> Result<StatusRow> {
    let skill_count = crate::mirror::mirror_skill_dirs(&target.canonical_path)
        .map(|skills| skills.len())
        .unwrap_or(0);
    let drift_entries = match target.scope {
        TargetScope::Global => target
            .views
            .iter()
            .map(|view| crate::view::view_status(&target.canonical_path, view).map(|d| d.len()))
            .sum::<Result<usize>>()?,
        TargetScope::Project => crate::view::project_status(&target)?.len(),
    };
    Ok(StatusRow {
        scope: target.name,
        kind: match target.scope {
            TargetScope::Global => "global",
            TargetScope::Project => "project",
        },
        canonical_path: target.canonical_path.to_string(),
        skill_count,
        drift_entries,
    })
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
                .filter_map(|target| crate::mirror::mirror_skill_dirs(&target.canonical_path).ok())
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

#[derive(Serialize)]
struct StatusRow {
    scope: String,
    kind: &'static str,
    canonical_path: String,
    skill_count: usize,
    drift_entries: usize,
}
