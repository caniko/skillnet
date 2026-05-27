use anyhow::Result;
use serde::Serialize;

use super::Context;
use crate::{
    cli::args::StatusFormat,
    commands::status::promotion_status_counts,
    model::Target,
    view::{
        materialize_view_with_options, view_diff, view_status, DriftEntry, DriftKind,
        FileDeltaKind, ViewSyncOptions, ViewSyncSummary,
    },
};

pub fn sync(ctx: &Context, allow_delete: bool, force: bool) -> Result<()> {
    let target = ctx.config.global_target(&ctx.mirror_root)?;
    if ctx.dry_run {
        print_dry_run(&target, allow_delete, force);
        return Ok(());
    }

    for view in &target.views {
        let summary = materialize_view_with_options(
            &target.canonical_path,
            view,
            ViewSyncOptions {
                allow_delete,
                force,
                ..ViewSyncOptions::default()
            },
        )?;
        println!(
            "{}  {}",
            view.label,
            format_view_summary(&view.path, &summary)
        );
    }
    Ok(())
}

pub fn status(ctx: &Context, format: StatusFormat) -> Result<()> {
    let target = ctx.config.global_target(&ctx.mirror_root)?;
    let mut rows = Vec::new();
    for view in &target.views {
        let drift = view_status(&target.canonical_path, view)?;
        let promotion_status = promotion_status_counts(&drift);
        rows.push(ViewStatusRow {
            label: view.label.clone(),
            path: view.path.to_string(),
            would_promote: promotion_status.would_promote,
            needs_tie_break: promotion_status.needs_tie_break,
            drift,
        });
    }

    match format {
        StatusFormat::Text => {
            for row in &rows {
                if row.drift.is_empty() {
                    println!("{}  clean", row.label);
                } else {
                    println!("{}  drift ({} entries)", row.label, row.drift.len());
                    for entry in &row.drift {
                        println!("{} {}", drift_marker(entry.kind), entry.skill);
                    }
                }
            }
        }
        StatusFormat::Json => {
            serde_json::to_writer_pretty(std::io::stdout(), &rows)?;
            println!();
        }
    }
    Ok(())
}

pub fn diff(ctx: &Context) -> Result<()> {
    let target = ctx.config.global_target(&ctx.mirror_root)?;
    for view in &target.views {
        println!("# {}", view.label);
        let deltas = view_diff(&target.canonical_path, view)?;
        if deltas.is_empty() {
            println!("clean");
            continue;
        }
        for delta in deltas {
            let marker = match delta.kind {
                FileDeltaKind::Missing => '-',
                FileDeltaKind::Extra => '+',
                FileDeltaKind::Modified => '~',
            };
            println!("{marker} {}", delta.skill);
        }
    }
    Ok(())
}

pub(crate) fn format_view_summary(path: &camino::Utf8Path, summary: &ViewSyncSummary) -> String {
    format!(
        "{} (+{} ~{} ={} -{})",
        path, summary.created, summary.updated, summary.unchanged, summary.removed
    )
}

fn print_dry_run(target: &Target, allow_delete: bool, force: bool) {
    println!("# view sync global");
    println!("from: {}", target.canonical_path);
    println!("allow_delete: {allow_delete}");
    println!("force: {force}");
    for view in &target.views {
        println!("to: {}\t{}", view.label, view.path);
    }
}

fn drift_marker(kind: DriftKind) -> char {
    match kind {
        DriftKind::Missing => '-',
        DriftKind::WrongTarget | DriftKind::NonSymlink => '~',
        DriftKind::Stale => '+',
    }
}

#[derive(Serialize)]
struct ViewStatusRow {
    label: String,
    path: String,
    would_promote: usize,
    needs_tie_break: usize,
    drift: Vec<DriftEntry>,
}
