use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use super::Context;
use crate::{
    model::{Target, ViewTarget},
    view::{self, DriftKind, PromotionOptions, PromotionSummary, ReconcileOutcome, WouldEntry},
};

pub fn run(ctx: &Context, options: PromotionOptions, no_promote: bool) -> Result<i32> {
    if no_promote {
        return run_no_promote(ctx, &options);
    }
    run_with_promotion(ctx, options)
}

fn run_no_promote(ctx: &Context, options: &PromotionOptions) -> Result<i32> {
    super::view::sync(ctx, options.allow_delete, options.force_demote)?;
    super::project_sync(ctx, &[], true, options.allow_delete, options.force_demote)?;
    Ok(0)
}

fn run_with_promotion(ctx: &Context, options: PromotionOptions) -> Result<i32> {
    let mut report = OverallReport::default();
    let global = ctx.config.global_target(&ctx.mirror_root)?;

    if ctx.dry_run {
        dry_run_target(ctx, &global, &options, &mut report)?;
    } else {
        for view in &global.views {
            let summary = view::materialize_view_with_promotion(
                &global.canonical_path,
                view,
                PromotionOptions {
                    relative_links: false,
                    project_root: None,
                    ..options.clone()
                },
                |target_path| ctx.ensure_target_clean(target_path),
            )?;
            report.add_summary(&global, view, &summary);
        }
    }

    for target in ctx.config.targets(&ctx.mirror_root)?.into_iter().skip(1) {
        if let Some(project_root) = &target.project_root {
            if !project_root.is_dir() {
                eprintln!(
                    "warn: [{}] project repository path {} does not exist; skipping",
                    target.name, project_root
                );
                continue;
            }
        }

        if ctx.dry_run {
            dry_run_target(ctx, &target, &options, &mut report)?;
            continue;
        }

        let summary = view::materialize_project_with_promotion(
            &target,
            PromotionOptions {
                relative_links: true,
                project_root: target.project_root.clone(),
                ..options.clone()
            },
            |target_path| ctx.ensure_target_clean(target_path),
        )?;
        for view in &summary.views {
            report.add_summary_by_path(
                &target.name,
                &target.canonical_path,
                &view.label,
                &view.path,
                &view.summary,
            );
        }
    }

    report.print();

    if ctx.dry_run || report.totals.pending() == 0 {
        Ok(0)
    } else {
        Ok(2)
    }
}

fn dry_run_target(
    _ctx: &Context,
    target: &Target,
    options: &PromotionOptions,
    report: &mut OverallReport,
) -> Result<()> {
    println!("# {} sync {}", target_kind(target), target.name);
    println!("from: {}", target.canonical_path);
    println!("allow_delete: {}", options.allow_delete);
    println!("force: {}", options.force_demote);
    println!("apply_promote: {}", options.apply_promote);
    for view in &target.views {
        println!("to: {}\t{}", view.label, view.path);
        let mut summary = PromotionSummary::default();
        for entry in view::view_status(&target.canonical_path, view)? {
            if entry.kind != DriftKind::NonSymlink {
                continue;
            }
            let Some(outcome) = entry.reconcile_outcome.clone() else {
                continue;
            };
            match outcome {
                ReconcileOutcome::ViewNewer { .. } => summary.would_promote.push(WouldEntry {
                    skill: entry.skill,
                    outcome,
                }),
                ReconcileOutcome::CanonicalNewer { .. } => {
                    summary.would_demote_destructive.push(WouldEntry {
                        skill: entry.skill,
                        outcome,
                    });
                }
                ReconcileOutcome::EqualMtimeDifferentContent { .. }
                | ReconcileOutcome::BothAdvanced { .. } => {
                    summary.needs_tie_break.push(WouldEntry {
                        skill: entry.skill,
                        outcome,
                    });
                }
                ReconcileOutcome::Identical | ReconcileOutcome::AdoptCandidate => {}
            }
        }
        report.add_summary(target, view, &summary);
    }
    Ok(())
}

fn target_kind(target: &Target) -> &'static str {
    match target.scope {
        crate::model::TargetScope::Global => "view",
        crate::model::TargetScope::Project => "project",
    }
}

#[derive(Default)]
struct OverallReport {
    totals: Totals,
    per_target: Vec<TargetReport>,
}

impl OverallReport {
    fn add_summary(&mut self, target: &Target, view: &ViewTarget, summary: &PromotionSummary) {
        self.add_summary_by_path(
            &target.name,
            &target.canonical_path,
            &view.label,
            &view.path,
            summary,
        );
    }

    fn add_summary_by_path(
        &mut self,
        target_name: &str,
        canonical_path: &Utf8Path,
        view_label: &str,
        view_path: &Utf8Path,
        summary: &PromotionSummary,
    ) {
        self.totals.add(summary);
        self.per_target.push(TargetReport {
            target_name: target_name.to_string(),
            canonical_path: canonical_path.to_path_buf(),
            view_label: view_label.to_string(),
            view_path: view_path.to_path_buf(),
            summary: summary.clone(),
        });
    }

    fn print(&self) {
        for target in &self.per_target {
            println!(
                "{}:{}  {} (+{} ~{} ={} -{})",
                target.target_name,
                target.view_label,
                target.view_path,
                target.summary.view.created,
                target.summary.view.updated,
                target.summary.view.unchanged,
                target.summary.view.removed
            );
            print_would_entries(
                "would promote",
                &target.view_path,
                &target.canonical_path,
                &target.summary.would_promote,
            );
            print_would_entries(
                "would destructively demote",
                &target.view_path,
                &target.canonical_path,
                &target.summary.would_demote_destructive,
            );
            print_would_entries(
                "needs tie-break",
                &target.view_path,
                &target.canonical_path,
                &target.summary.needs_tie_break,
            );
        }
    }
}

#[derive(Default)]
struct Totals {
    created: usize,
    updated: usize,
    unchanged: usize,
    removed: usize,
    promoted: usize,
    demoted_destructive: usize,
    adopted: usize,
    would_promote: usize,
    would_demote_destructive: usize,
    needs_tie_break: usize,
}

impl Totals {
    fn add(&mut self, summary: &PromotionSummary) {
        self.created += summary.view.created;
        self.updated += summary.view.updated;
        self.unchanged += summary.view.unchanged;
        self.removed += summary.view.removed;
        self.promoted += summary.promoted.len();
        self.demoted_destructive += summary.demoted_destructive.len();
        self.adopted += summary.adopted.len();
        self.would_promote += summary.would_promote.len();
        self.would_demote_destructive += summary.would_demote_destructive.len();
        self.needs_tie_break += summary.needs_tie_break.len();
    }

    fn pending(&self) -> usize {
        self.would_promote + self.would_demote_destructive + self.needs_tie_break
    }
}

struct TargetReport {
    target_name: String,
    canonical_path: Utf8PathBuf,
    view_label: String,
    view_path: Utf8PathBuf,
    summary: PromotionSummary,
}

fn print_would_entries(
    label: &str,
    view_path: &Utf8Path,
    canonical_path: &Utf8Path,
    entries: &[WouldEntry],
) {
    for entry in entries {
        let view_skill = view_path.join(&entry.skill);
        let canonical_skill = canonical_path.join(&entry.skill);
        match (&entry.outcome, label) {
            (
                ReconcileOutcome::ViewNewer {
                    view_mtime,
                    canonical_mtime,
                },
                "would promote",
            ) => println!(
                "{label} {view_skill} -> {canonical_skill} (view_mtime={view_mtime}, canonical_mtime={canonical_mtime})"
            ),
            (ReconcileOutcome::CanonicalNewer { .. }, "would destructively demote") => println!(
                "{label} {view_skill} -> {canonical_skill} (view newer mtime; pass --force to discard view)"
            ),
            (
                ReconcileOutcome::EqualMtimeDifferentContent {
                    view_sha,
                    canonical_sha,
                    ..
                },
                "needs tie-break",
            ) => println!(
                "{label} {view_skill} vs {canonical_skill} (view_sha={}, canonical_sha={}); pass --prefer view|canonical",
                short_sha(view_sha),
                short_sha(canonical_sha)
            ),
            (ReconcileOutcome::BothAdvanced { .. }, "needs tie-break") => println!(
                "{label} {view_skill} vs {canonical_skill}; pass --prefer view|canonical"
            ),
            _ => {}
        }
    }
}

fn short_sha(sha: &str) -> &str {
    sha.get(..8).unwrap_or(sha)
}
