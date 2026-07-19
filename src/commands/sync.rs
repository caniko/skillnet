use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

use super::Context;
use crate::{
    cli::Scope,
    config::legacy_project_canonical_warning,
    link::LinkStrategy,
    model::{Target, TargetScope, ViewTarget},
    view::{
        self, AggregatorPending, AggregatorPendingKind, AggregatorPlanAction, AggregatorStatus,
        DriftKind, PromotionOptions, PromotionSummary, ReconcileOutcome, ViewSyncOptions,
        WouldEntry,
    },
};

pub fn run(
    ctx: &Context,
    scopes: &[Scope],
    options: PromotionOptions,
    no_promote: bool,
    link_strategy: Option<LinkStrategy>,
) -> Result<i32> {
    if no_promote {
        return run_no_promote(ctx, scopes, &options, link_strategy);
    }
    run_with_promotion(ctx, scopes, options, link_strategy)
}

fn run_no_promote(
    ctx: &Context,
    scopes: &[Scope],
    options: &PromotionOptions,
    link_strategy: Option<LinkStrategy>,
) -> Result<i32> {
    for target in scoped_targets(ctx, scopes, link_strategy)? {
        match target.scope {
            TargetScope::Global => super::view::sync(
                ctx,
                options.allow_delete,
                options.force_demote,
                link_strategy,
            )?,
            TargetScope::Project => {
                super::project_sync(
                    ctx,
                    std::slice::from_ref(&target.name),
                    false,
                    options.allow_delete,
                    options.force_demote,
                    link_strategy,
                )?;
            }
        }
    }
    Ok(0)
}

fn run_with_promotion(
    ctx: &Context,
    scopes: &[Scope],
    options: PromotionOptions,
    link_strategy: Option<LinkStrategy>,
) -> Result<i32> {
    let mut report = OverallReport::default();

    for target in scoped_targets(ctx, scopes, link_strategy)? {
        warn_legacy_project_layout(&target);
        if ctx.dry_run {
            dry_run_target(ctx, &target, &options, &mut report)?;
            continue;
        }

        match target.scope {
            TargetScope::Global => {
                if let Some(bundle) = ctx.bundle_plan(&target)? {
                    if target.link_strategy != LinkStrategy::Symlink {
                        anyhow::bail!(
                            "Skillnet manifest bundles require symlink views; target `{}` is configured for {:?}",
                            target.name,
                            target.link_strategy
                        );
                    }
                    bundle.materialize()?;
                    for view in &target.views {
                        let view_summary = view::materialize_view_with_expected(
                            bundle.expected_links(),
                            view,
                            ViewSyncOptions {
                                allow_delete: options.allow_delete,
                                force: options.force_demote,
                                link_strategy: LinkStrategy::Symlink,
                                ..ViewSyncOptions::default()
                            },
                        )?;
                        report.add_summary(
                            &target,
                            view,
                            &PromotionSummary {
                                view: view_summary,
                                ..PromotionSummary::default()
                            },
                        );
                    }
                } else {
                    for view in &target.views {
                        let summary = view::materialize_view_with_promotion(
                            &target.canonical_path,
                            view,
                            PromotionOptions {
                                relative_links: false,
                                link_strategy: target.link_strategy,
                                project_root: None,
                                ..options.clone()
                            },
                            |target_path| ctx.ensure_target_clean(target_path),
                        )?;
                        report.add_summary(&target, view, &summary);
                    }
                }
            }
            TargetScope::Project => {
                ctx.ensure_target_clean(&target.canonical_path)?;
                let summary = view::materialize_project_with_promotion(
                    &target,
                    PromotionOptions {
                        relative_links: true,
                        link_strategy: target.link_strategy,
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
                report.add_aggregator_summary(
                    &target,
                    summary.aggregator,
                    &summary.aggregator_pending,
                );
            }
        }
    }

    report.print();

    if ctx.dry_run || report.totals.pending() == 0 {
        Ok(0)
    } else {
        Ok(2)
    }
}

fn warn_legacy_project_layout(target: &Target) {
    if let Some(message) = legacy_project_canonical_warning(target) {
        eprintln!("warning: {message}");
    }
}

fn scoped_targets(
    ctx: &Context,
    scopes: &[Scope],
    link_strategy: Option<LinkStrategy>,
) -> Result<Vec<Target>> {
    let mut targets = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let target = match scope {
            Scope::Global => ctx
                .config
                .global_target_with_link_override(&ctx.mirror_root, link_strategy)?,
            Scope::Project(name) => {
                let project = ctx
                    .project(name)
                    .ok_or_else(|| anyhow::anyhow!("unknown project `{name}`"))?;
                ctx.config.project_target_with_link_override(
                    &ctx.mirror_root,
                    project,
                    link_strategy,
                )?
            }
        };

        if let Some(project_root) = &target.project_root {
            if !project_root.is_dir() {
                eprintln!(
                    "warn: [{}] project repository path {} does not exist; skipping",
                    target.name, project_root
                );
                continue;
            }
        }
        targets.push(target);
    }
    Ok(targets)
}

fn dry_run_target(
    ctx: &Context,
    target: &Target,
    options: &PromotionOptions,
    report: &mut OverallReport,
) -> Result<()> {
    println!("# {} sync {}", target_kind(target), target.name);
    println!("from: {}", target.canonical_path);
    println!("allow_delete: {}", options.allow_delete);
    println!("force: {}", options.force_demote);
    println!("apply_promote: {}", options.apply_promote);
    if target.scope == TargetScope::Global {
        if let Some(bundle) = ctx.bundle_plan(target)? {
            println!("bundle_root: {}", bundle.bundle_root);
            println!("bundle_mode: generated bundles; promotion disabled");
        }
    }
    if target.scope == TargetScope::Project {
        for entry in view::project_canonical_drift(target)? {
            println!("canonical: {} {}", drift_marker(entry.kind), entry.skill);
        }
    }
    let view_root = match target.scope {
        TargetScope::Project => target
            .aggregator_path
            .as_ref()
            .unwrap_or(&target.canonical_path),
        TargetScope::Global => &target.canonical_path,
    };
    let bundle = if target.scope == TargetScope::Global {
        ctx.bundle_plan(target)?
    } else {
        None
    };
    for view in &target.views {
        println!("to: {}\t{}", view.label, view.path);
        let mut summary = PromotionSummary::default();
        let drift = match &bundle {
            Some(bundle) => view::view_status_with_expected(
                bundle.expected_links(),
                view,
                ViewSyncOptions {
                    link_strategy: LinkStrategy::Symlink,
                    ..ViewSyncOptions::default()
                },
            )?,
            None => view::view_status(view_root, view)?,
        };
        for entry in drift {
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
    if let Some(plan) = view::project_aggregator_plan(target)? {
        match plan.strategy {
            LinkStrategy::Symlink => {
                println!("working-copy: symlink {} -> {}", plan.path, plan.canonical);
            }
            LinkStrategy::Hardlink => {
                println!(
                    "working-copy: hardlink {} ({} files)",
                    plan.path, plan.file_count
                );
            }
        }
        match plan.action {
            AggregatorPlanAction::Create => println!("  + create working copy"),
            AggregatorPlanAction::Update => println!("  ~ replace working copy"),
            AggregatorPlanAction::Relink { files } => {
                println!(
                    "  ~ relink {} severed file{}",
                    files.len(),
                    plural(files.len())
                );
            }
            AggregatorPlanAction::Diverged { files }
                if options.force_demote || options.prefer == Some(view::Preference::Canonical) =>
            {
                println!(
                    "  ~ relink {} diverged file{}",
                    files.len(),
                    plural(files.len())
                );
            }
            AggregatorPlanAction::Diverged { files } => println!(
                "  ! {} diverged file{}; pass --force or --prefer canonical",
                files.len(),
                plural(files.len())
            ),
            AggregatorPlanAction::Unchanged => println!("  = unchanged"),
        }
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
    aggregators: Vec<AggregatorReport>,
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

    fn add_aggregator_summary(
        &mut self,
        target: &Target,
        status: Option<AggregatorStatus>,
        pending: &[AggregatorPending],
    ) {
        if status.is_none() && pending.is_empty() {
            return;
        }
        self.totals.aggregator_pending += pending.len();
        self.aggregators.push(AggregatorReport {
            target_name: target.name.clone(),
            path: target.aggregator_path.clone(),
            status,
            pending: pending.to_vec(),
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
        for aggregator in &self.aggregators {
            if let (Some(path), Some(status)) = (&aggregator.path, aggregator.status) {
                println!(
                    "{}:working-copy  {} ({})",
                    aggregator.target_name,
                    path,
                    format_aggregator_status(status)
                );
            }
            for entry in &aggregator.pending {
                print_aggregator_pending(entry);
            }
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
    aggregator_pending: usize,
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
        self.would_promote
            + self.would_demote_destructive
            + self.needs_tie_break
            + self.aggregator_pending
    }
}

struct TargetReport {
    target_name: String,
    canonical_path: Utf8PathBuf,
    view_label: String,
    view_path: Utf8PathBuf,
    summary: PromotionSummary,
}

struct AggregatorReport {
    target_name: String,
    path: Option<Utf8PathBuf>,
    status: Option<AggregatorStatus>,
    pending: Vec<AggregatorPending>,
}

fn format_aggregator_status(status: AggregatorStatus) -> &'static str {
    match status {
        AggregatorStatus::Created => "created",
        AggregatorStatus::Updated => "updated",
        AggregatorStatus::Unchanged => "unchanged",
    }
}

fn print_aggregator_pending(entry: &AggregatorPending) {
    match entry.kind {
        AggregatorPendingKind::Diverged => println!(
            "working copy needs tie-break: {} diverged file{} at {}; pass --prefer canonical or --force",
            entry.files.len(),
            plural(entry.files.len()),
            entry.path
        ),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
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

fn drift_marker(kind: DriftKind) -> char {
    match kind {
        DriftKind::Missing => '-',
        DriftKind::WrongTarget | DriftKind::NonSymlink => '~',
        DriftKind::Stale => '+',
    }
}
