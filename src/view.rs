//! View materialisation primitives.
//!
//! A view is a flat directory of symlinks, one symlink per canonical skill
//! directory. Global views use absolute symlink targets because they live under
//! user homes. Project views use relative symlink targets so checked-in project
//! links remain portable across machines.
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
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::{
    mirror::mirror_skill_dirs,
    model::{Target, ViewTarget},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewSyncOptions {
    pub allow_delete: bool,
    pub force: bool,
    pub relative_links: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        let desired = desired_link_target(&view.path, target, options.relative_links)?;
        let metadata = match fs::symlink_metadata(&link) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                drift.push(DriftEntry {
                    skill: skill.clone(),
                    kind: DriftKind::Missing,
                    expected: Some(desired),
                    actual: None,
                });
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("failed to inspect {link}")),
        };
        if !metadata.file_type().is_symlink() {
            drift.push(DriftEntry {
                skill: skill.clone(),
                kind: DriftKind::NonSymlink,
                expected: Some(desired),
                actual: Some(link),
            });
            continue;
        }
        let actual = read_link_utf8(&link)?;
        if actual != desired {
            drift.push(DriftEntry {
                skill: skill.clone(),
                kind: DriftKind::WrongTarget,
                expected: Some(desired),
                actual: Some(actual),
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
        });
    }

    Ok(drift)
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
        },
    )
}

pub fn materialize_project_with_options(
    target: &Target,
    options: ProjectSyncOptions,
) -> Result<ProjectSyncSummary> {
    let mut views = Vec::with_capacity(target.views.len());
    for view in &target.views {
        let summary = materialize_view_with_options(
            &target.canonical_path,
            view,
            ViewSyncOptions {
                allow_delete: options.allow_delete,
                force: options.force,
                relative_links: true,
            },
        )?;
        views.push(ProjectViewSummary {
            label: view.label.clone(),
            path: view.path.clone(),
            summary,
        });
    }

    let aggregator = match &target.aggregator_path {
        Some(path) => Some(ensure_aggregator_symlink(path, &target.canonical_path)?),
        None => None,
    };

    Ok(ProjectSyncSummary { views, aggregator })
}

pub fn project_status(target: &Target) -> Result<Vec<DriftEntry>> {
    let mut drift = Vec::new();
    for view in &target.views {
        drift.extend(view_status_with_options(
            &target.canonical_path,
            view,
            ViewSyncOptions {
                relative_links: true,
                ..ViewSyncOptions::default()
            },
        )?);
    }
    if let Some(path) = &target.aggregator_path {
        if aggregator_status(path, &target.canonical_path)? != AggregatorStatus::Unchanged {
            drift.push(DriftEntry {
                skill: "aggregator".to_string(),
                kind: DriftKind::WrongTarget,
                expected: Some(target.canonical_path.clone()),
                actual: fs::read_link(path)
                    .ok()
                    .and_then(|path| Utf8PathBuf::from_path_buf(path).ok()),
            });
        }
    }
    Ok(drift)
}

pub fn project_diff(target: &Target) -> Result<Vec<FileDelta>> {
    let mut deltas = Vec::new();
    for view in &target.views {
        deltas.extend(view_diff_with_options(
            &target.canonical_path,
            view,
            ViewSyncOptions {
                relative_links: true,
                ..ViewSyncOptions::default()
            },
        )?);
    }
    if let Some(path) = &target.aggregator_path {
        if aggregator_status(path, &target.canonical_path)? != AggregatorStatus::Unchanged {
            deltas.push(FileDelta {
                skill: "aggregator".to_string(),
                kind: FileDeltaKind::Modified,
                expected: Some(target.canonical_path.clone()),
                actual: fs::read_link(path)
                    .ok()
                    .and_then(|path| Utf8PathBuf::from_path_buf(path).ok()),
            });
        }
    }
    Ok(deltas)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectSyncOptions {
    pub allow_delete: bool,
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSyncSummary {
    pub views: Vec<ProjectViewSummary>,
    pub aggregator: Option<AggregatorStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectViewSummary {
    pub label: String,
    pub path: Utf8PathBuf,
    pub summary: ViewSyncSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregatorStatus {
    Created,
    Updated,
    Unchanged,
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

fn aggregator_status(path: &Utf8Path, canonical: &Utf8Path) -> Result<AggregatorStatus> {
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
