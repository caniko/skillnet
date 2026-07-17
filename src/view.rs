//! View materialisation primitives.
//!
//! A view is a flat directory with one materialised entry per canonical skill
//! directory. Symlink views use absolute global targets and relative project
//! targets; hardlink views copy the directory shape while sharing regular-file
//! inodes with the canonical store.
//!
//! Atomicity is per skill: each link is checked and, if needed, replaced by
//! creating a temporary symlink in the same directory and renaming it into
//! place. A partial failure may leave earlier skills synced and later skills
//! untouched. There is no transactional rollback; rerun the sync after fixing
//! the underlying error. Stale entries are only removed when `allow_delete` is
//! set. A failed write exits with the per-skill error, and status/doctor can
//! surface the remaining drift on the next inspection.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs as unix_fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use walkdir::WalkDir;

use crate::{
    fs_ops::{
        content_signature, copy_dir, hardlink_dir, hardlink_dir_status, newest_mtime_nanos,
        relink_files, HardlinkStatus,
    },
    link::LinkStrategy,
    mirror::mirror_skill_dirs,
    model::{Target, ViewTarget},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewSyncOptions {
    pub allow_delete: bool,
    pub force: bool,
    pub relative_links: bool,
    pub link_strategy: LinkStrategy,
}

impl Default for ViewSyncOptions {
    fn default() -> Self {
        Self {
            allow_delete: false,
            force: false,
            relative_links: false,
            link_strategy: LinkStrategy::Symlink,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ViewSyncSummary {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DriftEntry {
    pub skill: String,
    pub kind: DriftKind,
    pub expected: Option<Utf8PathBuf>,
    pub actual: Option<Utf8PathBuf>,
    pub view_mtime_nanos: Option<u128>,
    pub canonical_mtime_nanos: Option<u128>,
    pub view_sha: Option<String>,
    pub canonical_sha: Option<String>,
    #[serde(skip)]
    pub reconcile_outcome: Option<ReconcileOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DriftKind {
    Missing,
    WrongTarget,
    NonSymlink,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDelta {
    pub skill: String,
    pub kind: FileDeltaKind,
    pub expected: Option<Utf8PathBuf>,
    pub actual: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ReconcileOutcome {
    Identical,
    ViewNewer {
        view_mtime: u128,
        canonical_mtime: u128,
    },
    CanonicalNewer {
        view_mtime: u128,
        canonical_mtime: u128,
    },
    EqualMtimeDifferentContent {
        view_sha: String,
        canonical_sha: String,
        mtime: u128,
    },
    BothAdvanced {
        view_only: Vec<Utf8PathBuf>,
        canonical_only: Vec<Utf8PathBuf>,
    },
    AdoptCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Preference {
    View,
    Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionOptions {
    pub apply_promote: bool,
    pub force_demote: bool,
    pub prefer: Option<Preference>,
    pub adopt_new: bool,
    pub allow_delete: bool,
    pub relative_links: bool,
    pub link_strategy: LinkStrategy,
    pub project_root: Option<Utf8PathBuf>,
    pub link_root: Option<Utf8PathBuf>,
}

impl Default for PromotionOptions {
    fn default() -> Self {
        Self {
            apply_promote: false,
            force_demote: false,
            prefer: None,
            adopt_new: false,
            allow_delete: false,
            relative_links: false,
            link_strategy: LinkStrategy::Symlink,
            project_root: None,
            link_root: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PromotionSummary {
    pub view: ViewSyncSummary,
    pub promoted: Vec<String>,
    pub demoted_destructive: Vec<String>,
    pub adopted: Vec<String>,
    pub would_promote: Vec<WouldEntry>,
    pub would_demote_destructive: Vec<WouldEntry>,
    pub needs_tie_break: Vec<WouldEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WouldEntry {
    pub skill: String,
    pub outcome: ReconcileOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FileDeltaKind {
    Missing,
    Extra,
    Modified,
}

pub fn materialize_view(canonical: &Utf8Path, view: &ViewTarget) -> Result<ViewSyncSummary> {
    materialize_view_with_options(canonical, view, ViewSyncOptions::default())
}

pub fn materialize_view_with_options(
    canonical: &Utf8Path,
    view: &ViewTarget,
    options: ViewSyncOptions,
) -> Result<ViewSyncSummary> {
    let expected = expected_skill_links(canonical)?;
    fs::create_dir_all(&view.path)
        .with_context(|| format!("failed to create view directory {}", view.path))?;

    let mut summary = ViewSyncSummary::default();
    for (skill, target) in &expected {
        let link = view.path.join(skill);
        if options.link_strategy == LinkStrategy::Hardlink {
            match hardlink_dir_status(target, &link)
                .with_context(|| format!("failed to inspect hardlinked skill `{skill}`"))?
            {
                HardlinkStatus::Identical => summary.unchanged += 1,
                HardlinkStatus::Severed { files } => {
                    relink_files(target, &link, &files)
                        .with_context(|| format!("failed to relink skill `{skill}`"))?;
                    summary.updated += 1;
                }
                HardlinkStatus::Diverged { .. } if !options.force => {
                    bail!(
                        "hardlinked view entry {link} diverged from canonical skill; pass --force to replace it"
                    );
                }
                HardlinkStatus::Missing => {
                    hardlink_dir(target, &link)
                        .with_context(|| format!("failed to hardlink skill `{skill}`"))?;
                    summary.created += 1;
                }
                HardlinkStatus::Foreign | HardlinkStatus::Diverged { .. } => {
                    hardlink_dir(target, &link)
                        .with_context(|| format!("failed to replace hardlinked skill `{skill}`"))?;
                    summary.updated += 1;
                }
            }
            continue;
        }
        let desired = desired_link_target(&view.path, target, options.relative_links)?;
        match ensure_symlink(&link, &desired, options.force)
            .with_context(|| format!("failed to sync skill `{skill}` into {}", view.path))?
        {
            LinkAction::Created => summary.created += 1,
            LinkAction::Updated => summary.updated += 1,
            LinkAction::Unchanged => summary.unchanged += 1,
        }
    }

    if options.allow_delete {
        for stale in stale_view_entries(&view.path, expected.keys())? {
            remove_view_entry(&stale)
                .with_context(|| format!("failed to remove stale view entry {stale}"))?;
            summary.removed += 1;
        }
    }

    Ok(summary)
}

pub fn materialize_view_with_promotion(
    canonical_root: &Utf8Path,
    view: &ViewTarget,
    options: PromotionOptions,
    ensure_clean: impl Fn(&Utf8Path) -> Result<()>,
) -> Result<PromotionSummary> {
    ensure_clean(canonical_root)?;

    // Hardlinked views are derived materialisations, not editable promotion
    // surfaces. Keep them convergent and preserve the hardlink strategy rather
    // than demoting them to symlinks through the legacy promotion path.
    if options.link_strategy == LinkStrategy::Hardlink {
        let view = materialize_view_with_options(
            canonical_root,
            view,
            ViewSyncOptions {
                allow_delete: options.allow_delete,
                force: options.force_demote,
                relative_links: options.relative_links,
                link_strategy: LinkStrategy::Hardlink,
            },
        )?;
        return Ok(PromotionSummary {
            view,
            ..PromotionSummary::default()
        });
    }

    let expected = expected_skill_links(canonical_root)?;
    fs::create_dir_all(&view.path)
        .with_context(|| format!("failed to create view directory {}", view.path))?;

    let mut summary = PromotionSummary::default();
    for (skill, canonical_skill) in &expected {
        let link = view.path.join(skill);
        let desired_target = options
            .link_root
            .as_ref()
            .map(|root| root.join(skill))
            .unwrap_or_else(|| canonical_skill.clone());
        let desired = desired_link_target(&view.path, &desired_target, options.relative_links)?;
        match fs::symlink_metadata(&link) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let current = read_link_utf8(&link)?;
                if current == desired {
                    summary.view.unchanged += 1;
                } else {
                    atomic_symlink(&desired, &link)?;
                    summary.view.updated += 1;
                }
            }
            Ok(_) => {
                let outcome = compare_view_entry(canonical_skill, &link)
                    .with_context(|| format!("failed to compare skill `{skill}`"))?;
                apply_reconcile_outcome(
                    ReconcileAction {
                        skill,
                        link: &link,
                        canonical_skill,
                        desired: &desired,
                    },
                    outcome,
                    &options,
                    &ensure_clean,
                    &mut summary,
                )?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                atomic_symlink(&desired, &link)?;
                summary.view.created += 1;
            }
            Err(err) => return Err(err).with_context(|| format!("failed to inspect {link}")),
        }
    }

    for stale in stale_view_entries(&view.path, expected.keys())? {
        let skill = stale
            .file_name()
            .context("stale view entry has no final component")?
            .to_string();
        let metadata = fs::symlink_metadata(&stale)
            .with_context(|| format!("failed to inspect stale view entry {stale}"))?;
        if metadata.file_type().is_symlink() {
            if options.allow_delete {
                remove_view_entry(&stale)
                    .with_context(|| format!("failed to remove stale view entry {stale}"))?;
                summary.view.removed += 1;
            }
            continue;
        }

        let canonical_skill = canonical_root.join(&skill);
        let outcome = compare_view_entry(&canonical_skill, &stale)
            .with_context(|| format!("failed to compare stale skill `{skill}`"))?;
        if matches!(outcome, ReconcileOutcome::AdoptCandidate)
            && options.adopt_new
            && options.apply_promote
        {
            ensure_project_clean_for_write(&options, &ensure_clean)?;
            promote_view_to_canonical(&stale, &canonical_skill)
                .with_context(|| format!("failed to adopt skill `{skill}`"))?;
            summary.adopted.push(skill);
        }
    }

    Ok(summary)
}

struct ReconcileAction<'a> {
    skill: &'a str,
    link: &'a Utf8Path,
    canonical_skill: &'a Utf8Path,
    desired: &'a Utf8Path,
}

fn apply_reconcile_outcome(
    action: ReconcileAction<'_>,
    outcome: ReconcileOutcome,
    options: &PromotionOptions,
    ensure_clean: &impl Fn(&Utf8Path) -> Result<()>,
    summary: &mut PromotionSummary,
) -> Result<()> {
    match outcome {
        ReconcileOutcome::Identical => {
            demote_to_symlink(action.link, action.desired)?;
            summary.view.updated += 1;
        }
        ReconcileOutcome::ViewNewer { .. } if options.apply_promote => {
            ensure_project_clean_for_write(options, ensure_clean)?;
            promote_view_to_canonical(action.link, action.canonical_skill)
                .with_context(|| format!("failed to promote skill `{}`", action.skill))?;
            demote_to_symlink(action.link, action.desired)?;
            summary.promoted.push(action.skill.to_string());
            summary.view.updated += 1;
        }
        ReconcileOutcome::ViewNewer { .. } => {
            summary.would_promote.push(WouldEntry {
                skill: action.skill.to_string(),
                outcome,
            });
        }
        ReconcileOutcome::CanonicalNewer { .. } if options.force_demote => {
            demote_to_symlink(action.link, action.desired)?;
            summary.demoted_destructive.push(action.skill.to_string());
            summary.view.updated += 1;
        }
        ReconcileOutcome::CanonicalNewer { .. } => {
            summary.would_demote_destructive.push(WouldEntry {
                skill: action.skill.to_string(),
                outcome,
            });
        }
        ReconcileOutcome::EqualMtimeDifferentContent { .. }
        | ReconcileOutcome::BothAdvanced { .. }
            if options.prefer == Some(Preference::View) && options.apply_promote =>
        {
            ensure_project_clean_for_write(options, ensure_clean)?;
            promote_view_to_canonical(action.link, action.canonical_skill)
                .with_context(|| format!("failed to promote skill `{}`", action.skill))?;
            demote_to_symlink(action.link, action.desired)?;
            summary.promoted.push(action.skill.to_string());
            summary.view.updated += 1;
        }
        ReconcileOutcome::EqualMtimeDifferentContent { .. }
        | ReconcileOutcome::BothAdvanced { .. }
            if options.prefer == Some(Preference::Canonical) && options.force_demote =>
        {
            demote_to_symlink(action.link, action.desired)?;
            summary.demoted_destructive.push(action.skill.to_string());
            summary.view.updated += 1;
        }
        ReconcileOutcome::EqualMtimeDifferentContent { .. }
        | ReconcileOutcome::BothAdvanced { .. } => {
            summary.needs_tie_break.push(WouldEntry {
                skill: action.skill.to_string(),
                outcome,
            });
        }
        ReconcileOutcome::AdoptCandidate if options.adopt_new && options.apply_promote => {
            ensure_project_clean_for_write(options, ensure_clean)?;
            promote_view_to_canonical(action.link, action.canonical_skill)
                .with_context(|| format!("failed to adopt skill `{}`", action.skill))?;
            summary.adopted.push(action.skill.to_string());
        }
        ReconcileOutcome::AdoptCandidate => {}
    }
    Ok(())
}

fn ensure_project_clean_for_write(
    options: &PromotionOptions,
    ensure_clean: &impl Fn(&Utf8Path) -> Result<()>,
) -> Result<()> {
    if let Some(project_root) = &options.project_root {
        ensure_clean(project_root)?;
    }
    Ok(())
}

fn demote_to_symlink(link: &Utf8Path, desired: &Utf8Path) -> Result<()> {
    remove_view_entry(link)?;
    atomic_symlink(desired, link)
}

pub fn compare_view_entry(
    canonical_skill: &Utf8Path,
    view_entry: &Utf8Path,
) -> Result<ReconcileOutcome> {
    let metadata = fs::symlink_metadata(view_entry)
        .with_context(|| format!("failed to inspect view entry {view_entry}"))?;
    if metadata.file_type().is_symlink() {
        bail!("{view_entry} is a symlink; compare_view_entry expects a non-symlink view entry");
    }
    if !canonical_skill.exists() {
        return Ok(ReconcileOutcome::AdoptCandidate);
    }

    let view_mtime = newest_mtime_nanos(view_entry)
        .with_context(|| format!("failed to compute newest mtime for {view_entry}"))?;
    let canonical_mtime = newest_mtime_nanos(canonical_skill)
        .with_context(|| format!("failed to compute newest mtime for {canonical_skill}"))?;
    let view_sha = content_signature(view_entry)
        .with_context(|| format!("failed to compute content signature for {view_entry}"))?;
    let canonical_sha = content_signature(canonical_skill)
        .with_context(|| format!("failed to compute content signature for {canonical_skill}"))?;

    if view_mtime > now_nanos()? {
        return Ok(ReconcileOutcome::EqualMtimeDifferentContent {
            view_sha,
            canonical_sha,
            mtime: view_mtime,
        });
    }

    if view_sha == canonical_sha {
        return Ok(ReconcileOutcome::Identical);
    }

    match view_mtime.cmp(&canonical_mtime) {
        std::cmp::Ordering::Greater => {
            let (view_only, canonical_only) = file_set_delta(view_entry, canonical_skill)?;
            if canonical_only.is_empty() {
                Ok(ReconcileOutcome::ViewNewer {
                    view_mtime,
                    canonical_mtime,
                })
            } else {
                Ok(ReconcileOutcome::BothAdvanced {
                    view_only,
                    canonical_only,
                })
            }
        }
        std::cmp::Ordering::Less => {
            let (view_only, canonical_only) = file_set_delta(view_entry, canonical_skill)?;
            if view_only.is_empty() {
                Ok(ReconcileOutcome::CanonicalNewer {
                    view_mtime,
                    canonical_mtime,
                })
            } else {
                Ok(ReconcileOutcome::BothAdvanced {
                    view_only,
                    canonical_only,
                })
            }
        }
        std::cmp::Ordering::Equal => Ok(ReconcileOutcome::EqualMtimeDifferentContent {
            view_sha,
            canonical_sha,
            mtime: view_mtime,
        }),
    }
}

pub fn promote_view_to_canonical(view_entry: &Utf8Path, canonical_skill: &Utf8Path) -> Result<()> {
    let canonical_parent = canonical_skill
        .parent()
        .context("canonical skill path has no parent")?;
    let skill = canonical_skill
        .file_name()
        .context("canonical skill path has no file name")?;
    let staging_parent = canonical_parent.join(".skillnet-tmp");
    let staging = staging_parent.join(format!("{skill}-{}", std::process::id()));

    if staging.exists() {
        fs::remove_dir_all(&staging)
            .with_context(|| format!("failed to remove stale staging directory {staging}"))?;
    }
    fs::create_dir_all(&staging_parent)
        .with_context(|| format!("failed to create staging parent {staging_parent}"))?;
    copy_dir(view_entry, &staging)
        .with_context(|| format!("failed to stage promoted skill from {view_entry}"))?;

    if canonical_skill.exists() {
        fs::remove_dir_all(canonical_skill).with_context(|| {
            format!("failed to remove existing canonical skill {canonical_skill}")
        })?;
    }
    fs::rename(&staging, canonical_skill)
        .with_context(|| format!("failed to replace canonical skill {canonical_skill}"))?;
    Ok(())
}

pub fn view_status(canonical: &Utf8Path, view: &ViewTarget) -> Result<Vec<DriftEntry>> {
    view_status_with_options(canonical, view, ViewSyncOptions::default())
}

pub fn view_status_with_options(
    canonical: &Utf8Path,
    view: &ViewTarget,
    options: ViewSyncOptions,
) -> Result<Vec<DriftEntry>> {
    let expected = expected_skill_links(canonical)?;
    let mut drift = Vec::new();

    for (skill, target) in &expected {
        let link = view.path.join(skill);
        if options.link_strategy == LinkStrategy::Hardlink {
            let status = hardlink_dir_status(target, &link)
                .with_context(|| format!("failed to inspect hardlinked skill `{skill}`"))?;
            if !matches!(status, HardlinkStatus::Identical) {
                let kind = match status {
                    HardlinkStatus::Missing => DriftKind::Missing,
                    HardlinkStatus::Severed { .. } | HardlinkStatus::Diverged { .. } => {
                        DriftKind::NonSymlink
                    }
                    HardlinkStatus::Foreign => DriftKind::NonSymlink,
                    HardlinkStatus::Identical => unreachable!(),
                };
                drift.push(DriftEntry {
                    skill: skill.clone(),
                    kind,
                    expected: Some(target.clone()),
                    actual: fs::symlink_metadata(&link).ok().map(|_| link.clone()),
                    view_mtime_nanos: None,
                    canonical_mtime_nanos: None,
                    view_sha: None,
                    canonical_sha: None,
                    reconcile_outcome: None,
                });
            }
            continue;
        }
        let desired = desired_link_target(&view.path, target, options.relative_links)?;
        let metadata = match fs::symlink_metadata(&link) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                drift.push(DriftEntry {
                    skill: skill.clone(),
                    kind: DriftKind::Missing,
                    expected: Some(desired),
                    actual: None,
                    view_mtime_nanos: None,
                    canonical_mtime_nanos: None,
                    view_sha: None,
                    canonical_sha: None,
                    reconcile_outcome: None,
                });
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("failed to inspect {link}")),
        };
        if !metadata.file_type().is_symlink() {
            drift.push(non_symlink_drift_entry(skill, &desired, target, &link));
            continue;
        }
        let actual = read_link_utf8(&link)?;
        if actual != desired {
            drift.push(DriftEntry {
                skill: skill.clone(),
                kind: DriftKind::WrongTarget,
                expected: Some(desired),
                actual: Some(actual),
                view_mtime_nanos: None,
                canonical_mtime_nanos: None,
                view_sha: None,
                canonical_sha: None,
                reconcile_outcome: None,
            });
        }
    }

    for stale in stale_view_entries(&view.path, expected.keys())? {
        drift.push(DriftEntry {
            skill: stale
                .file_name()
                .context("stale view entry has no final component")?
                .to_string(),
            kind: DriftKind::Stale,
            expected: None,
            actual: Some(stale),
            view_mtime_nanos: None,
            canonical_mtime_nanos: None,
            view_sha: None,
            canonical_sha: None,
            reconcile_outcome: None,
        });
    }

    Ok(drift)
}

fn non_symlink_drift_entry(
    skill: &str,
    desired: &Utf8Path,
    canonical_skill: &Utf8Path,
    link: &Utf8Path,
) -> DriftEntry {
    let mut entry = DriftEntry {
        skill: skill.to_string(),
        kind: DriftKind::NonSymlink,
        expected: Some(desired.to_path_buf()),
        actual: Some(link.to_path_buf()),
        view_mtime_nanos: None,
        canonical_mtime_nanos: None,
        view_sha: None,
        canonical_sha: None,
        reconcile_outcome: None,
    };

    match compare_view_entry(canonical_skill, link) {
        Ok(outcome) => {
            match &outcome {
                ReconcileOutcome::ViewNewer {
                    view_mtime,
                    canonical_mtime,
                }
                | ReconcileOutcome::CanonicalNewer {
                    view_mtime,
                    canonical_mtime,
                } => {
                    entry.view_mtime_nanos = Some(*view_mtime);
                    entry.canonical_mtime_nanos = Some(*canonical_mtime);
                }
                ReconcileOutcome::EqualMtimeDifferentContent {
                    view_sha,
                    canonical_sha,
                    mtime,
                } => {
                    entry.view_mtime_nanos = Some(*mtime);
                    entry.canonical_mtime_nanos = Some(*mtime);
                    entry.view_sha = Some(view_sha.clone());
                    entry.canonical_sha = Some(canonical_sha.clone());
                }
                ReconcileOutcome::BothAdvanced { .. }
                | ReconcileOutcome::Identical
                | ReconcileOutcome::AdoptCandidate => {}
            }
            entry.reconcile_outcome = Some(outcome);
        }
        Err(err) => eprintln!("warn: failed to classify non-symlink view entry {link}: {err:#}"),
    }

    entry
}

pub fn view_diff(canonical: &Utf8Path, view: &ViewTarget) -> Result<Vec<FileDelta>> {
    view_diff_with_options(canonical, view, ViewSyncOptions::default())
}

pub fn view_diff_with_options(
    canonical: &Utf8Path,
    view: &ViewTarget,
    options: ViewSyncOptions,
) -> Result<Vec<FileDelta>> {
    view_status_with_options(canonical, view, options).map(|entries| {
        entries
            .into_iter()
            .map(|entry| FileDelta {
                skill: entry.skill,
                kind: match entry.kind {
                    DriftKind::Missing => FileDeltaKind::Missing,
                    DriftKind::Stale => FileDeltaKind::Extra,
                    DriftKind::WrongTarget | DriftKind::NonSymlink => FileDeltaKind::Modified,
                },
                expected: entry.expected,
                actual: entry.actual,
            })
            .collect()
    })
}

pub fn materialize_project(target: &Target) -> Result<ProjectSyncSummary> {
    materialize_project_with_options(
        target,
        ProjectSyncOptions {
            allow_delete: false,
            force: false,
            link_strategy: target.link_strategy,
        },
    )
}

pub fn materialize_project_with_options(
    target: &Target,
    options: ProjectSyncOptions,
) -> Result<ProjectSyncSummary> {
    ensure_project_canonical(target)?;

    let (aggregator, aggregator_pending) = match &target.aggregator_path {
        Some(path) => {
            let outcome = ensure_aggregator(
                path,
                &target.canonical_path,
                options.link_strategy,
                true,
                None,
            )
            .with_context(|| {
                format!(
                    "[{}] failed to materialise project working copy",
                    target.name
                )
            })?;
            (Some(outcome.status), outcome.pending)
        }
        None => (None, Vec::new()),
    };

    let mut views = Vec::with_capacity(target.views.len());
    let view_root = target
        .aggregator_path
        .as_ref()
        .unwrap_or(&target.canonical_path);
    for view in &target.views {
        let summary = materialize_view_with_options(
            view_root,
            view,
            ViewSyncOptions {
                allow_delete: options.allow_delete,
                force: options.force,
                relative_links: true,
                // Project aggregators may be hardlinked, but their checked-in
                // agent views remain portable relative symlinks.
                link_strategy: LinkStrategy::Symlink,
            },
        )?;
        views.push(ProjectViewSummary {
            label: view.label.clone(),
            path: view.path.clone(),
            summary,
        });
    }

    Ok(ProjectSyncSummary {
        views,
        aggregator,
        aggregator_pending,
    })
}

pub fn materialize_project_with_promotion(
    target: &Target,
    options: PromotionOptions,
    ensure_clean: impl Fn(&Utf8Path) -> Result<()>,
) -> Result<ProjectPromotionSummary> {
    ensure_project_canonical(target)?;

    if let Some(path) = &target.aggregator_path {
        ensure_aggregator(
            path,
            &target.canonical_path,
            options.link_strategy,
            true,
            options.prefer,
        )
        .with_context(|| {
            format!(
                "[{}] failed to materialise project working copy",
                target.name
            )
        })?;
    }

    let mut views = Vec::with_capacity(target.views.len());
    for view in &target.views {
        let summary = materialize_view_with_promotion(
            &target.canonical_path,
            view,
            PromotionOptions {
                relative_links: true,
                // Keep project views as relative symlinks; only the working
                // copy aggregator follows the project's link strategy.
                link_strategy: LinkStrategy::Symlink,
                project_root: target.project_root.clone(),
                link_root: target.aggregator_path.clone(),
                ..options.clone()
            },
            &ensure_clean,
        )?;
        views.push(ProjectPromotionViewSummary {
            label: view.label.clone(),
            path: view.path.clone(),
            summary,
        });
    }

    let (aggregator, aggregator_pending) = match &target.aggregator_path {
        Some(path) => {
            let outcome = ensure_aggregator(
                path,
                &target.canonical_path,
                options.link_strategy,
                true,
                options.prefer,
            )
            .with_context(|| {
                format!(
                    "[{}] failed to materialise project working copy",
                    target.name
                )
            })?;
            (Some(outcome.status), outcome.pending)
        }
        None => (None, Vec::new()),
    };

    Ok(ProjectPromotionSummary {
        views,
        aggregator,
        aggregator_pending,
    })
}

pub fn project_status(target: &Target) -> Result<Vec<DriftEntry>> {
    let mut drift = Vec::new();
    drift.extend(project_canonical_drift(target)?);
    let view_root = target
        .aggregator_path
        .as_ref()
        .unwrap_or(&target.canonical_path);
    for view in &target.views {
        drift.extend(view_status_with_options(
            view_root,
            view,
            ViewSyncOptions {
                relative_links: true,
                ..ViewSyncOptions::default()
            },
        )?);
    }
    if target.canonical_path.exists() {
        if let Some(path) = &target.aggregator_path {
            if let Some(entry) =
                aggregator_drift_entry(path, &target.canonical_path, target.link_strategy)?
            {
                drift.push(entry);
            }
        }
    }
    Ok(drift)
}

pub fn project_diff(target: &Target) -> Result<Vec<FileDelta>> {
    let mut deltas = Vec::new();
    deltas.extend(
        project_canonical_drift(target)?
            .into_iter()
            .map(|entry| FileDelta {
                skill: entry.skill,
                kind: FileDeltaKind::Modified,
                expected: entry.expected,
                actual: entry.actual,
            }),
    );
    let view_root = target
        .aggregator_path
        .as_ref()
        .unwrap_or(&target.canonical_path);
    for view in &target.views {
        deltas.extend(view_diff_with_options(
            view_root,
            view,
            ViewSyncOptions {
                relative_links: true,
                ..ViewSyncOptions::default()
            },
        )?);
    }
    if target.canonical_path.exists() {
        if let Some(path) = &target.aggregator_path {
            if let Some(delta) =
                aggregator_file_delta(path, &target.canonical_path, target.link_strategy)?
            {
                deltas.push(delta);
            }
        }
    }
    Ok(deltas)
}

pub fn project_aggregator_plan(target: &Target) -> Result<Option<AggregatorPlan>> {
    let Some(path) = &target.aggregator_path else {
        return Ok(None);
    };
    let action = if !project_canonical_drift(target)?.is_empty() {
        AggregatorPlanAction::Update
    } else if !target.canonical_path.exists() {
        AggregatorPlanAction::Create
    } else {
        match target.link_strategy {
            LinkStrategy::Symlink => match aggregator_symlink_status(path, &target.canonical_path)?
            {
                AggregatorStatus::Created => AggregatorPlanAction::Create,
                AggregatorStatus::Updated => AggregatorPlanAction::Update,
                AggregatorStatus::Unchanged => AggregatorPlanAction::Unchanged,
            },
            LinkStrategy::Hardlink => match hardlink_dir_status(&target.canonical_path, path)? {
                HardlinkStatus::Missing => AggregatorPlanAction::Create,
                HardlinkStatus::Foreign => AggregatorPlanAction::Update,
                HardlinkStatus::Identical => AggregatorPlanAction::Unchanged,
                HardlinkStatus::Severed { files } => AggregatorPlanAction::Relink { files },
                HardlinkStatus::Diverged { files } => AggregatorPlanAction::Diverged { files },
            },
        }
    };
    Ok(Some(AggregatorPlan {
        strategy: target.link_strategy,
        path: path.clone(),
        canonical: target.canonical_path.clone(),
        file_count: regular_file_count(&target.canonical_path).unwrap_or(0),
        action,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSyncOptions {
    pub allow_delete: bool,
    pub force: bool,
    pub link_strategy: LinkStrategy,
}

impl Default for ProjectSyncOptions {
    fn default() -> Self {
        Self {
            allow_delete: false,
            force: false,
            link_strategy: LinkStrategy::Symlink,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProjectSyncSummary {
    pub views: Vec<ProjectViewSummary>,
    pub aggregator: Option<AggregatorStatus>,
    pub aggregator_pending: Vec<AggregatorPending>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectViewSummary {
    pub label: String,
    pub path: Utf8PathBuf,
    pub summary: ViewSyncSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProjectPromotionSummary {
    pub views: Vec<ProjectPromotionViewSummary>,
    pub aggregator: Option<AggregatorStatus>,
    pub aggregator_pending: Vec<AggregatorPending>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectPromotionViewSummary {
    pub label: String,
    pub path: Utf8PathBuf,
    pub summary: PromotionSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AggregatorStatus {
    Created,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregatorPending {
    pub path: Utf8PathBuf,
    pub canonical: Utf8PathBuf,
    pub kind: AggregatorPendingKind,
    pub files: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AggregatorPendingKind {
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatorPlan {
    pub strategy: LinkStrategy,
    pub path: Utf8PathBuf,
    pub canonical: Utf8PathBuf,
    pub file_count: usize,
    pub action: AggregatorPlanAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregatorPlanAction {
    Create,
    Update,
    Relink { files: Vec<Utf8PathBuf> },
    Diverged { files: Vec<Utf8PathBuf> },
    Unchanged,
}

struct AggregatorOutcome {
    status: AggregatorStatus,
    pending: Vec<AggregatorPending>,
}

fn ensure_project_canonical(target: &Target) -> Result<()> {
    let canonical = &target.canonical_path;
    fs::create_dir_all(canonical)
        .with_context(|| format!("failed to create project canonical store {canonical}"))?;

    let symlink_entries = symlink_skill_entries(canonical)?;
    if !symlink_entries.is_empty() {
        bail!(
            "project canonical store {canonical} contains symlink skill entries ({}); repair legacy project skills before syncing",
            sample_paths(&symlink_entries)
        );
    }

    Ok(())
}

fn symlink_skill_entries(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("failed to read {root}"))? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in project store: {}", p.display()))?;
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect project store entry {path}"))?;
        if metadata.file_type().is_symlink() {
            entries.push(path);
        }
    }
    entries.sort();
    Ok(entries)
}

pub(crate) fn project_canonical_drift(target: &Target) -> Result<Vec<DriftEntry>> {
    let mut drift = Vec::new();
    if !target.canonical_path.exists() {
        drift.push(DriftEntry {
            skill: "canonical".to_string(),
            kind: DriftKind::Missing,
            expected: Some(target.canonical_path.clone()),
            actual: None,
            view_mtime_nanos: None,
            canonical_mtime_nanos: None,
            view_sha: None,
            canonical_sha: None,
            reconcile_outcome: None,
        });
        return Ok(drift);
    }
    for entry in symlink_skill_entries(&target.canonical_path)? {
        drift.push(DriftEntry {
            skill: format!(
                "canonical/{}",
                entry
                    .file_name()
                    .context("project canonical entry has no final component")?
            ),
            kind: DriftKind::WrongTarget,
            expected: Some(target.canonical_path.clone()),
            actual: Some(entry),
            view_mtime_nanos: None,
            canonical_mtime_nanos: None,
            view_sha: None,
            canonical_sha: None,
            reconcile_outcome: None,
        });
    }
    Ok(drift)
}

fn sample_paths(paths: &[Utf8PathBuf]) -> String {
    const LIMIT: usize = 3;
    let mut names = paths
        .iter()
        .take(LIMIT)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if paths.len() > LIMIT {
        names.push(format!("+{} more", paths.len() - LIMIT));
    }
    names.join(", ")
}

fn now_nanos() -> Result<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos())
}

fn file_set_delta(
    view_entry: &Utf8Path,
    canonical_skill: &Utf8Path,
) -> Result<(Vec<Utf8PathBuf>, Vec<Utf8PathBuf>)> {
    let view_files = comparable_file_set(view_entry)?;
    let canonical_files = comparable_file_set(canonical_skill)?;
    let view_only = view_files.difference(&canonical_files).cloned().collect();
    let canonical_only = canonical_files.difference(&view_files).cloned().collect();
    Ok((view_only, canonical_only))
}

fn comparable_file_set(root: &Utf8Path) -> Result<BTreeSet<Utf8PathBuf>> {
    let mut files = BTreeSet::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        if path
            .components()
            .any(|component| component.as_str() == ".skillnet-tmp")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            files.insert(path.strip_prefix(root)?.to_path_buf());
        }
    }
    Ok(files)
}

fn regular_file_count(root: &Utf8Path) -> Result<usize> {
    let mut count = 0;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() {
            count += 1;
        }
    }
    Ok(count)
}

fn expected_skill_links(canonical: &Utf8Path) -> Result<BTreeMap<String, Utf8PathBuf>> {
    mirror_skill_dirs(canonical)?
        .into_iter()
        .map(|path| {
            let skill = path
                .file_name()
                .context("canonical skill directory has no final component")?
                .to_string();
            Ok((skill, path))
        })
        .collect()
}

fn ensure_symlink(link: &Utf8Path, desired: &Utf8Path, force: bool) -> Result<LinkAction> {
    match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let current = read_link_utf8(link)?;
            if current == desired {
                return Ok(LinkAction::Unchanged);
            }
            atomic_symlink(desired, link)?;
            Ok(LinkAction::Updated)
        }
        Ok(_) if force => {
            remove_view_entry(link)?;
            atomic_symlink(desired, link)?;
            Ok(LinkAction::Updated)
        }
        Ok(_) => bail!("{link} exists and is not a symlink; pass --force to replace it"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            atomic_symlink(desired, link)?;
            Ok(LinkAction::Created)
        }
        Err(err) => Err(err).with_context(|| format!("failed to inspect {link}")),
    }
}

fn ensure_aggregator(
    path: &Utf8Path,
    canonical: &Utf8Path,
    strategy: LinkStrategy,
    force: bool,
    prefer: Option<Preference>,
) -> Result<AggregatorOutcome> {
    match strategy {
        LinkStrategy::Symlink => {
            ensure_aggregator_symlink(path, canonical).map(|status| AggregatorOutcome {
                status,
                pending: Vec::new(),
            })
        }
        LinkStrategy::Hardlink => ensure_aggregator_hardlink(path, canonical, force, prefer),
    }
}

fn ensure_aggregator_symlink(path: &Utf8Path, canonical: &Utf8Path) -> Result<AggregatorStatus> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let current = read_link_utf8(path)?;
            if current == canonical {
                return Ok(AggregatorStatus::Unchanged);
            }
            atomic_symlink(canonical, path)?;
            Ok(AggregatorStatus::Updated)
        }
        Ok(metadata) if metadata.is_dir() => bail!(
            "aggregator path `{path}` is a directory; run `skillnet project clean <name>` first or remove it manually"
        ),
        Ok(_) => bail!("aggregator path `{path}` exists and is not a symlink"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            atomic_symlink(canonical, path)?;
            Ok(AggregatorStatus::Created)
        }
        Err(err) => Err(err).with_context(|| format!("failed to inspect {path}")),
    }
}

fn ensure_aggregator_hardlink(
    path: &Utf8Path,
    canonical: &Utf8Path,
    force: bool,
    prefer: Option<Preference>,
) -> Result<AggregatorOutcome> {
    match hardlink_dir_status(canonical, path)? {
        HardlinkStatus::Missing => {
            hardlink_dir(canonical, path)?;
            Ok(aggregator_outcome(AggregatorStatus::Created))
        }
        HardlinkStatus::Foreign => {
            hardlink_dir(canonical, path)?;
            Ok(aggregator_outcome(AggregatorStatus::Updated))
        }
        HardlinkStatus::Identical => Ok(aggregator_outcome(AggregatorStatus::Unchanged)),
        HardlinkStatus::Severed { files } => {
            relink_files(canonical, path, &files)?;
            Ok(aggregator_outcome(AggregatorStatus::Updated))
        }
        HardlinkStatus::Diverged { files } if force || prefer == Some(Preference::Canonical) => {
            relink_files(canonical, path, &files)?;
            Ok(aggregator_outcome(AggregatorStatus::Updated))
        }
        HardlinkStatus::Diverged { files } => Ok(AggregatorOutcome {
            status: AggregatorStatus::Unchanged,
            pending: vec![AggregatorPending {
                path: path.to_path_buf(),
                canonical: canonical.to_path_buf(),
                kind: AggregatorPendingKind::Diverged,
                files,
            }],
        }),
    }
}

fn aggregator_outcome(status: AggregatorStatus) -> AggregatorOutcome {
    AggregatorOutcome {
        status,
        pending: Vec::new(),
    }
}

fn aggregator_status(
    path: &Utf8Path,
    canonical: &Utf8Path,
    strategy: LinkStrategy,
) -> Result<AggregatorStatus> {
    match strategy {
        LinkStrategy::Symlink => aggregator_symlink_status(path, canonical),
        LinkStrategy::Hardlink => match hardlink_dir_status(canonical, path)? {
            HardlinkStatus::Missing => Ok(AggregatorStatus::Created),
            HardlinkStatus::Identical => Ok(AggregatorStatus::Unchanged),
            HardlinkStatus::Severed { .. }
            | HardlinkStatus::Diverged { .. }
            | HardlinkStatus::Foreign => Ok(AggregatorStatus::Updated),
        },
    }
}

fn aggregator_symlink_status(path: &Utf8Path, canonical: &Utf8Path) -> Result<AggregatorStatus> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let current = read_link_utf8(path)?;
            if current == canonical {
                Ok(AggregatorStatus::Unchanged)
            } else {
                Ok(AggregatorStatus::Updated)
            }
        }
        Ok(_) => Ok(AggregatorStatus::Updated),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AggregatorStatus::Created),
        Err(err) => Err(err).with_context(|| format!("failed to inspect {path}")),
    }
}

fn aggregator_drift_entry(
    path: &Utf8Path,
    canonical: &Utf8Path,
    strategy: LinkStrategy,
) -> Result<Option<DriftEntry>> {
    let status = aggregator_status(path, canonical, strategy)?;
    if status == AggregatorStatus::Unchanged {
        return Ok(None);
    }
    let (kind, actual) = aggregator_drift_kind_and_actual(path, strategy, status);
    Ok(Some(DriftEntry {
        skill: "working-copy".to_string(),
        kind,
        expected: Some(canonical.to_path_buf()),
        actual,
        view_mtime_nanos: None,
        canonical_mtime_nanos: None,
        view_sha: None,
        canonical_sha: None,
        reconcile_outcome: None,
    }))
}

fn aggregator_file_delta(
    path: &Utf8Path,
    canonical: &Utf8Path,
    strategy: LinkStrategy,
) -> Result<Option<FileDelta>> {
    let status = aggregator_status(path, canonical, strategy)?;
    if status == AggregatorStatus::Unchanged {
        return Ok(None);
    }
    let kind = match status {
        AggregatorStatus::Created => FileDeltaKind::Missing,
        AggregatorStatus::Updated => FileDeltaKind::Modified,
        AggregatorStatus::Unchanged => unreachable!("unchanged status returned early"),
    };
    let actual = match (strategy, status) {
        (_, AggregatorStatus::Created) => None,
        (LinkStrategy::Symlink, _) => fs::read_link(path)
            .ok()
            .and_then(|path| Utf8PathBuf::from_path_buf(path).ok()),
        (LinkStrategy::Hardlink, _) => Some(path.to_path_buf()),
    };
    Ok(Some(FileDelta {
        skill: "working-copy".to_string(),
        kind,
        expected: Some(canonical.to_path_buf()),
        actual,
    }))
}

fn aggregator_drift_kind_and_actual(
    path: &Utf8Path,
    strategy: LinkStrategy,
    status: AggregatorStatus,
) -> (DriftKind, Option<Utf8PathBuf>) {
    match status {
        AggregatorStatus::Created => (DriftKind::Missing, None),
        AggregatorStatus::Updated => {
            let actual = match strategy {
                LinkStrategy::Symlink => fs::read_link(path)
                    .ok()
                    .and_then(|path| Utf8PathBuf::from_path_buf(path).ok()),
                LinkStrategy::Hardlink => Some(path.to_path_buf()),
            };
            (DriftKind::WrongTarget, actual)
        }
        AggregatorStatus::Unchanged => unreachable!("unchanged status has no drift"),
    }
}

fn stale_view_entries<'a>(
    view_path: &Utf8Path,
    expected: impl Iterator<Item = &'a String>,
) -> Result<Vec<Utf8PathBuf>> {
    if !view_path.exists() {
        return Ok(Vec::new());
    }
    let expected = expected.cloned().collect::<BTreeSet<_>>();
    let mut stale = Vec::new();
    for entry in fs::read_dir(view_path)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in view: {}", p.display()))?;
        let name = path
            .file_name()
            .context("view entry has no final component")?
            .to_string();
        if name == ".skillnet-tmp" || name.starts_with('.') && name.ends_with(".skillnet-tmp") {
            continue;
        }
        if !expected.contains(&name) {
            stale.push(path);
        }
    }
    stale.sort();
    Ok(stale)
}

fn remove_view_entry(path: &Utf8Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("failed to remove {path}"))
}

fn atomic_symlink(target: &Utf8Path, link: &Utf8Path) -> Result<()> {
    let parent = link.parent().context("symlink path has no parent")?;
    fs::create_dir_all(parent)?;
    let file_name = link.file_name().context("symlink path has no file name")?;
    let temp = parent.join(format!(".{file_name}.skillnet-tmp-{}", std::process::id()));
    if temp.exists() {
        remove_view_entry(&temp)?;
    }
    unix_fs::symlink(target, &temp)
        .with_context(|| format!("failed to create temporary symlink {temp} -> {target}"))?;
    fs::rename(&temp, link).with_context(|| format!("failed to replace symlink {link}"))
}

fn desired_link_target(
    link_parent: &Utf8Path,
    canonical_target: &Utf8Path,
    relative: bool,
) -> Result<Utf8PathBuf> {
    if !relative {
        return Ok(canonical_target.to_path_buf());
    }
    relative_path(link_parent, canonical_target)
}

fn relative_path(from_dir: &Utf8Path, to: &Utf8Path) -> Result<Utf8PathBuf> {
    let from = absolutize_for_relative(from_dir)?;
    let to = absolutize_for_relative(to)?;
    let from_components = normal_components(&from);
    let to_components = normal_components(&to);
    let common = from_components
        .iter()
        .zip(&to_components)
        .take_while(|(a, b)| a == b)
        .count();

    let mut out = PathBuf::new();
    for _ in common..from_components.len() {
        out.push("..");
    }
    for component in &to_components[common..] {
        out.push(component);
    }
    Utf8PathBuf::from_path_buf(out)
        .map_err(|p| anyhow::anyhow!("non-UTF-8 relative symlink target: {}", p.display()))
}

fn absolutize_for_relative(path: &Utf8Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.as_std_path().to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path.as_std_path()))
            .context("failed to resolve current directory")
    }
}

fn normal_components(path: &Path) -> Vec<PathBuf> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(PathBuf::from(prefix.as_os_str())),
            Component::RootDir => Some(PathBuf::from("/")),
            Component::Normal(part) => Some(PathBuf::from(part)),
            Component::ParentDir => Some(PathBuf::from("..")),
            Component::CurDir => None,
        })
        .collect()
}

fn read_link_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(fs::read_link(path)?)
        .map_err(|p| anyhow::anyhow!("non-UTF-8 symlink target at {path}: {}", p.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkAction {
    Created,
    Updated,
    Unchanged,
}
