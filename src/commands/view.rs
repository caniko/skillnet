use std::fs;

use anyhow::Result;
use serde::Serialize;

use super::Context;
use crate::{
    cli::args::StatusFormat,
    commands::status::promotion_status_counts,
    link::LinkStrategy,
    model::{Target, ViewTarget},
    view::{
        self, materialize_view_with_expected, materialize_view_with_options,
        materialize_view_with_promotion, DriftEntry, DriftKind, FileDeltaKind, PromotionOptions,
        PromotionSummary, ReconcileOutcome, ViewSyncOptions, ViewSyncSummary,
    },
};

pub fn sync(
    ctx: &Context,
    allow_delete: bool,
    force: bool,
    link_strategy: Option<LinkStrategy>,
) -> Result<()> {
    let target = ctx
        .config
        .global_target_with_link_override(&ctx.mirror_root, link_strategy)?;
    let bundle = prepare_bundle(ctx, &target)?;
    if ctx.dry_run {
        print_dry_run(&target, allow_delete, force, bundle.as_ref());
        return Ok(());
    }

    for view in &target.views {
        let options = ViewSyncOptions {
            allow_delete,
            force,
            link_strategy: target.link_strategy,
            ..ViewSyncOptions::default()
        };
        let summary = match &bundle {
            Some(bundle) => materialize_view_with_expected(bundle.expected_links(), view, options)?,
            None => materialize_view_with_options(&target.canonical_path, view, options)?,
        };
        println!(
            "{}  {}",
            view.label,
            format_view_summary(&view.path, &summary)
        );
    }
    Ok(())
}

/// Promote-and-adopt variant used by the user-facing `skillnet view sync`.
///
/// Unlike [`sync`], a view-only skill directory is never deleted: it is adopted
/// into the canonical store (and back-synced to a symlink on the next pass)
/// rather than wiped. `allow_delete` now only removes dangling view *symlinks*
/// whose canonical skill is gone; authored directories are always preserved.
pub fn sync_with_promotion(
    ctx: &Context,
    allow_delete: bool,
    force: bool,
    link_strategy: Option<LinkStrategy>,
) -> Result<()> {
    let target = ctx
        .config
        .global_target_with_link_override(&ctx.mirror_root, link_strategy)?;
    let bundle = prepare_bundle(ctx, &target)?;
    if ctx.dry_run {
        return print_promotion_dry_run(&target, allow_delete, force, bundle.as_ref());
    }
    if let Some(bundle) = &bundle {
        for view in &target.views {
            let summary = materialize_view_with_expected(
                bundle.expected_links(),
                view,
                ViewSyncOptions {
                    allow_delete,
                    force,
                    link_strategy: LinkStrategy::Symlink,
                    ..ViewSyncOptions::default()
                },
            )?;
            println!(
                "{}  {} (bundle; promotion disabled)",
                view.label,
                format_view_summary(&view.path, &summary)
            );
        }
        return Ok(());
    }

    let options = PromotionOptions {
        apply_promote: true,
        adopt_new: true,
        force_demote: force,
        allow_delete,
        prefer: None,
        relative_links: false,
        link_strategy: target.link_strategy,
        project_root: None,
        link_root: None,
    };

    let mut pending = 0usize;
    for view in &target.views {
        let summary = materialize_view_with_promotion(
            &target.canonical_path,
            view,
            options.clone(),
            |target_path| ctx.ensure_target_clean(target_path),
        )?;
        print_promotion_summary(view, &summary);
        pending += summary.would_promote.len()
            + summary.would_demote_destructive.len()
            + summary.needs_tie_break.len();
    }

    if pending > 0 {
        eprintln!(
            "note: {pending} entr{} need a decision; rerun `skillnet view sync` after adopted \
             skills demote, or use `skillnet sync --prefer view|canonical` / `--force`",
            if pending == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn print_promotion_summary(view: &ViewTarget, summary: &PromotionSummary) {
    println!(
        "{}  {}",
        view.label,
        format_view_summary(&view.path, &summary.view)
    );
    if !summary.adopted.is_empty() {
        println!("  adopted into canonical: {}", summary.adopted.join(", "));
    }
    if !summary.promoted.is_empty() {
        println!("  promoted to canonical: {}", summary.promoted.join(", "));
    }
    for entry in &summary.would_promote {
        println!("  would promote (view newer): {}", entry.skill);
    }
    for entry in &summary.would_demote_destructive {
        println!(
            "  canonical newer, view preserved (pass --force to discard view): {}",
            entry.skill
        );
    }
    for entry in &summary.needs_tie_break {
        println!(
            "  needs tie-break (use `skillnet sync --prefer view|canonical`): {}",
            entry.skill
        );
    }
}

fn print_promotion_dry_run(
    target: &Target,
    allow_delete: bool,
    force: bool,
    bundle: Option<&crate::bundle::BundlePlan>,
) -> Result<()> {
    println!("# view sync global (adopt + back-sync)");
    println!("from: {}", target.canonical_path);
    println!("allow_delete: {allow_delete}");
    println!("force: {force}");
    println!("apply_promote: true");
    println!("adopt_new: true");
    if let Some(bundle) = bundle {
        println!("bundle_root: {}", bundle.bundle_root);
        println!("bundle_mode: promotion disabled; generated bundles are read-only");
    }
    for view in &target.views {
        println!("to: {}\t{}", view.label, view.path);
        let drift = match bundle {
            Some(bundle) => view::view_status_with_expected(
                bundle.expected_links(),
                view,
                ViewSyncOptions {
                    link_strategy: LinkStrategy::Symlink,
                    ..ViewSyncOptions::default()
                },
            )?,
            None => view::view_status_with_options(
                &target.canonical_path,
                view,
                ViewSyncOptions {
                    link_strategy: target.link_strategy,
                    ..ViewSyncOptions::default()
                },
            )?,
        };
        for entry in drift {
            println!(
                "  {}",
                describe_dry_run_action(view, &entry, allow_delete, force)
            );
        }
    }
    Ok(())
}

fn describe_dry_run_action(
    view: &ViewTarget,
    entry: &DriftEntry,
    allow_delete: bool,
    force: bool,
) -> String {
    match entry.kind {
        DriftKind::Missing => format!("+ create symlink {}", entry.skill),
        DriftKind::WrongTarget => format!("~ update symlink {}", entry.skill),
        DriftKind::NonSymlink => match entry.reconcile_outcome.as_ref() {
            Some(ReconcileOutcome::Identical) => format!("~ back-sync to symlink {}", entry.skill),
            Some(ReconcileOutcome::ViewNewer { .. }) => {
                format!("^ promote view -> canonical {}", entry.skill)
            }
            Some(ReconcileOutcome::CanonicalNewer { .. }) if force => {
                format!("v discard view, restore symlink {}", entry.skill)
            }
            Some(ReconcileOutcome::CanonicalNewer { .. }) => {
                format!(
                    "! canonical newer; view preserved (pass --force) {}",
                    entry.skill
                )
            }
            Some(ReconcileOutcome::AdoptCandidate) => {
                format!("^ adopt into canonical {}", entry.skill)
            }
            _ => format!("? needs tie-break {}", entry.skill),
        },
        DriftKind::Stale => match fs::symlink_metadata(view.path.join(&entry.skill)) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if allow_delete {
                    format!("- remove dangling symlink {}", entry.skill)
                } else {
                    format!(
                        "= keep dangling symlink, pass --allow-delete to remove {}",
                        entry.skill
                    )
                }
            }
            Ok(_) => format!("^ adopt into canonical {}", entry.skill),
            Err(_) => format!("? {}", entry.skill),
        },
    }
}

pub fn status(ctx: &Context, format: StatusFormat) -> Result<()> {
    let target = ctx.config.global_target(&ctx.mirror_root)?;
    let bundle = ctx.bundle_plan(&target)?;
    let bundle_issues = bundle
        .as_ref()
        .map(crate::bundle::BundlePlan::materialization_issues)
        .transpose()?
        .unwrap_or_default();
    let mut rows = Vec::new();
    for view in &target.views {
        let drift = match &bundle {
            Some(bundle) => view::view_status_with_expected(
                bundle.expected_links(),
                view,
                ViewSyncOptions {
                    link_strategy: LinkStrategy::Symlink,
                    ..ViewSyncOptions::default()
                },
            )?,
            None => view::view_status_with_options(
                &target.canonical_path,
                view,
                ViewSyncOptions {
                    link_strategy: target.link_strategy,
                    ..ViewSyncOptions::default()
                },
            )?,
        };
        let promotion_status = promotion_status_counts(&drift);
        rows.push(ViewStatusRow {
            label: view.label.clone(),
            path: view.path.to_string(),
            bundle_issues: bundle_issues.clone(),
            would_promote: promotion_status.would_promote,
            needs_tie_break: promotion_status.needs_tie_break,
            drift,
        });
    }

    match format {
        StatusFormat::Text => {
            for row in &rows {
                if !row.bundle_issues.is_empty() {
                    println!(
                        "{}  bundle drift ({} issues)",
                        row.label,
                        row.bundle_issues.len()
                    );
                    for issue in &row.bundle_issues {
                        println!("{}", issue);
                    }
                } else if row.drift.is_empty() {
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
    let bundle = ctx.bundle_plan(&target)?;
    for view in &target.views {
        println!("# {}", view.label);
        let deltas = match &bundle {
            Some(bundle) => view::view_diff_with_expected(
                bundle.expected_links(),
                view,
                ViewSyncOptions {
                    link_strategy: LinkStrategy::Symlink,
                    ..ViewSyncOptions::default()
                },
            )?,
            None => view::view_diff_with_options(
                &target.canonical_path,
                view,
                ViewSyncOptions {
                    link_strategy: target.link_strategy,
                    ..ViewSyncOptions::default()
                },
            )?,
        };
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

fn print_dry_run(
    target: &Target,
    allow_delete: bool,
    force: bool,
    bundle: Option<&crate::bundle::BundlePlan>,
) {
    println!("# view sync global");
    println!("from: {}", target.canonical_path);
    println!("allow_delete: {allow_delete}");
    println!("force: {force}");
    if let Some(bundle) = bundle {
        println!("bundle_root: {}", bundle.bundle_root);
        println!("bundle_mode: generated bundles; exposed entrypoints only");
    }
    for view in &target.views {
        println!("to: {}\t{}", view.label, view.path);
    }
}

fn prepare_bundle(ctx: &Context, target: &Target) -> Result<Option<crate::bundle::BundlePlan>> {
    let plan = ctx.bundle_plan(target)?;
    let Some(plan) = plan else { return Ok(None) };
    if target.link_strategy != LinkStrategy::Symlink {
        anyhow::bail!(
            "Skillnet manifest bundles require symlink views; target `{}` is configured for {:?}",
            target.name,
            target.link_strategy
        );
    }
    if !ctx.dry_run {
        plan.materialize()?;
    }
    Ok(Some(plan))
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
    bundle_issues: Vec<String>,
    would_promote: usize,
    needs_tie_break: usize,
    drift: Vec<DriftEntry>,
}
