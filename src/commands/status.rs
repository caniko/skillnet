use anyhow::Result;
use serde::Serialize;
use std::io::Write;

use super::Context;
use crate::{
    cli::{args::StatusFormat, Scope},
    config::legacy_project_canonical_warning,
    model::{Target, TargetScope},
    view::{DriftEntry, ReconcileOutcome},
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
    warn_legacy_project_layout(&target);
    let skill_count = crate::mirror::mirror_skill_dirs(&target.canonical_path)
        .map(|skills| skills.len())
        .unwrap_or(0);
    let drift = match target.scope {
        TargetScope::Global => target
            .views
            .iter()
            .map(|view| crate::view::view_status(&target.canonical_path, view))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        TargetScope::Project => crate::view::project_status(&target)?,
    };
    let promotion_status = promotion_status_counts(&drift);
    Ok(StatusRow {
        scope: target.name,
        kind: match target.scope {
            TargetScope::Global => "global",
            TargetScope::Project => "project",
        },
        canonical_path: target.canonical_path.to_string(),
        skill_count,
        drift_entries: drift.len(),
        would_promote: promotion_status.would_promote,
        needs_tie_break: promotion_status.needs_tie_break,
        drift,
    })
}

fn warn_legacy_project_layout(target: &Target) {
    if let Some(message) = legacy_project_canonical_warning(target) {
        eprintln!("warning: {message}");
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PromotionStatusCounts {
    pub would_promote: usize,
    pub needs_tie_break: usize,
}

pub(crate) fn promotion_status_counts(drift: &[DriftEntry]) -> PromotionStatusCounts {
    let mut counts = PromotionStatusCounts::default();
    for entry in drift {
        match entry.reconcile_outcome.as_ref() {
            Some(ReconcileOutcome::ViewNewer { .. }) => counts.would_promote += 1,
            Some(
                ReconcileOutcome::EqualMtimeDifferentContent { .. }
                | ReconcileOutcome::BothAdvanced { .. },
            ) => counts.needs_tie_break += 1,
            Some(
                ReconcileOutcome::CanonicalNewer { .. }
                | ReconcileOutcome::Identical
                | ReconcileOutcome::AdoptCandidate,
            )
            | None => {}
        }
    }
    counts
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
    would_promote: usize,
    needs_tie_break: usize,
    drift: Vec<DriftEntry>,
}
