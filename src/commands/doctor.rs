use std::{
    collections::BTreeSet,
    fmt, fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};

use super::Context;
use crate::{
    config::legacy_project_canonical_warning,
    fs_ops::{hardlink_dir_status, HardlinkStatus},
    link::LinkStrategy,
    model::{Target, TargetScope, ViewTarget},
    view::{compare_view_entry, ReconcileOutcome},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("info"),
            Self::Warn => f.write_str("warn"),
            Self::Error => f.write_str("error"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    pub scope: String,
}

pub fn run(ctx: &Context) -> Result<()> {
    let issues = lint(ctx)?;
    if issues.is_empty() {
        println!("doctor: no issues");
        return Ok(());
    }

    for issue in &issues {
        eprintln!("{}: [{}] {}", issue.severity, issue.scope, issue.message);
    }
    if issues.iter().any(|issue| issue.severity != Severity::Info) {
        std::process::exit(1);
    }
    Ok(())
}

pub fn lint(ctx: &Context) -> Result<Vec<Issue>> {
    let mut issues = Vec::new();
    for target in ctx.all_targets()? {
        warn_legacy_project_layout(&target);
        match target.scope {
            TargetScope::Global => check_global(ctx, &target, &mut issues)?,
            TargetScope::Project => check_project(&target, &mut issues)?,
        }
    }
    issues.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then(left.scope.cmp(&right.scope))
            .then(left.message.cmp(&right.message))
    });
    Ok(issues)
}

fn warn_legacy_project_layout(target: &Target) {
    if let Some(message) = legacy_project_canonical_warning(target) {
        eprintln!("warning: {message}");
    }
}

fn check_global(ctx: &Context, target: &Target, issues: &mut Vec<Issue>) -> Result<()> {
    if !target.canonical_path.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "canonical store {} is missing or not a directory",
                target.canonical_path
            ),
        );
        return Ok(());
    }

    let canonical_real = canonicalize_utf8(&target.canonical_path)?;
    let canonical_skills = canonical_skill_names(&target.canonical_path)?;
    let bundle = crate::bundle::plan(&target.canonical_path, &target.name, &ctx.data_dir)?;
    if let Some(bundle) = &bundle {
        check_bundle_materialization(target, bundle, issues)?;
    }
    for view in &target.views {
        if let Some(bundle) = &bundle {
            check_bundle_view(target, view, bundle, issues)?;
        } else {
            check_global_view(target, view, &canonical_real, &canonical_skills, issues)?;
        }
    }
    Ok(())
}

fn check_bundle_materialization(
    target: &Target,
    bundle: &crate::bundle::BundlePlan,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    for message in bundle.materialization_issues()? {
        issue(issues, target, Severity::Error, message);
    }
    Ok(())
}

fn check_bundle_view(
    target: &Target,
    view: &ViewTarget,
    bundle: &crate::bundle::BundlePlan,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    if !view.path.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "view `{}` at {} is missing or not a directory",
                view.label, view.path
            ),
        );
        return Ok(());
    }
    let expected = bundle.expected_links();
    let mut entries = BTreeSet::new();
    for entry in read_dir_utf8(&view.path)? {
        let name = entry_name(&entry)?;
        entries.insert(name.clone());
        let metadata = fs::symlink_metadata(&entry)
            .with_context(|| format!("failed to inspect view entry {entry}"))?;
        if !metadata.file_type().is_symlink() {
            issue_non_symlink(
                issues,
                target,
                &bundle.bundle_root.join(&name),
                &entry,
                format!(
                    "bundle view `{}` entry `{name}` is not a symlink: {entry}",
                    view.label
                ),
            );
            continue;
        }
        let Some(expected_target) = expected.get(&name) else {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "bundle view `{}` exposes non-entrypoint `{name}`",
                    view.label
                ),
            );
            continue;
        };
        match (
            canonicalize_utf8(&entry),
            canonicalize_utf8(expected_target),
        ) {
            (Ok(actual), Ok(expected_real)) if actual == expected_real => {}
            (Ok(actual), Ok(expected_real)) => issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "bundle view `{}` entry `{name}` points to {actual}, expected {expected_real}",
                    view.label
                ),
            ),
            (Err(err), _) => issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "bundle view `{}` entry `{name}` is broken: {err}",
                    view.label
                ),
            ),
            (_, Err(err)) => issue(
                issues,
                target,
                Severity::Error,
                format!("generated bundle target `{name}` is unavailable: {err}"),
            ),
        }
    }
    for name in expected.keys() {
        if !entries.contains(name) {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "entrypoint `{name}` is missing from bundle view `{}`",
                    view.label
                ),
            );
        }
    }
    Ok(())
}

fn check_global_view(
    target: &Target,
    view: &ViewTarget,
    canonical_real: &Utf8Path,
    canonical_skills: &BTreeSet<String>,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    if !view.path.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "view `{}` at {} is missing or not a directory",
                view.label, view.path
            ),
        );
        return Ok(());
    }

    let mut view_entries = BTreeSet::new();
    for entry in read_dir_utf8(&view.path)? {
        let name = entry_name(&entry)?;
        view_entries.insert(name.clone());
        let metadata = fs::symlink_metadata(&entry)
            .with_context(|| format!("failed to inspect view entry {entry}"))?;
        if !metadata.file_type().is_symlink() {
            issue_non_symlink(
                issues,
                target,
                &target.canonical_path.join(&name),
                &entry,
                format!(
                    "view `{}` entry `{name}` is not a symlink: {entry}",
                    view.label
                ),
            );
            continue;
        }

        match canonicalize_utf8(&entry) {
            Ok(resolved) => {
                if resolved.parent() != Some(canonical_real) {
                    issue(
                        issues,
                        target,
                        Severity::Error,
                        format!(
                            "view `{}` entry `{name}` resolves outside canonical store: {entry} -> {resolved}",
                            view.label
                        ),
                    );
                }
            }
            Err(err) => issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "view `{}` entry `{name}` is a broken symlink: {entry}: {err}",
                    view.label
                ),
            ),
        }

        if !canonical_skills.contains(&name) {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "view `{}` entry `{name}` is not managed by skillnet; either register it or move it out of {}",
                    view.label, view.path
                ),
            );
        }
    }

    for skill in canonical_skills.difference(&view_entries) {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "canonical skill `{skill}` is missing from view `{}` at {}",
                view.label, view.path
            ),
        );
    }
    Ok(())
}

fn check_project(target: &Target, issues: &mut Vec<Issue>) -> Result<()> {
    let Some(project_root) = &target.project_root else {
        issue(
            issues,
            target,
            Severity::Error,
            "project target is missing its project root metadata".to_string(),
        );
        return Ok(());
    };
    let canonical_rel = target
        .canonical_rel
        .as_deref()
        .unwrap_or(".skills")
        .trim_matches('/');

    if !project_root.exists() {
        issue(
            issues,
            target,
            Severity::Warn,
            format!(
                "project repository path {project_root} does not exist; skipping project checks"
            ),
        );
        return Ok(());
    }
    if !project_root.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!("project repository path {project_root} exists but is not a directory"),
        );
        return Ok(());
    }
    if !target.canonical_path.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "project canonical store {} is missing or not a directory",
                target.canonical_path
            ),
        );
        return Ok(());
    }

    check_project_canonical(target, issues)?;
    let canonical_skills = canonical_skill_names(&target.canonical_path)?;
    for view in &target.views {
        check_project_view(
            target,
            view,
            project_root,
            canonical_rel,
            &canonical_skills,
            issues,
        )?;
    }
    check_project_aggregator(target, issues)?;
    Ok(())
}

fn check_project_canonical(target: &Target, issues: &mut Vec<Issue>) -> Result<()> {
    for entry in read_dir_utf8(&target.canonical_path)? {
        let metadata = fs::symlink_metadata(&entry)
            .with_context(|| format!("failed to inspect project canonical entry {entry}"))?;
        if metadata.file_type().is_symlink() {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project canonical entry {entry} is a symlink; expected real skill directory under {}",
                    target.canonical_path
                ),
            );
        }
    }
    Ok(())
}

fn check_project_view(
    target: &Target,
    view: &ViewTarget,
    project_root: &Utf8Path,
    canonical_rel: &str,
    canonical_skills: &BTreeSet<String>,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    if !view.path.is_dir() {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "project view `{}` at {} is missing or not a directory",
                view.label, view.path
            ),
        );
        return Ok(());
    }

    let mut view_entries = BTreeSet::new();
    for entry in read_dir_utf8(&view.path)? {
        let name = entry_name(&entry)?;
        view_entries.insert(name.clone());
        let metadata = fs::symlink_metadata(&entry)
            .with_context(|| format!("failed to inspect project view entry {entry}"))?;
        if !metadata.file_type().is_symlink() {
            issue_non_symlink(
                issues,
                target,
                &target.canonical_path.join(&name),
                &entry,
                format!(
                    "project view `{}` entry `{name}` is not a symlink: {entry}",
                    view.label
                ),
            );
            continue;
        }

        let actual = read_link_utf8(&entry)?;
        if actual.is_absolute() {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project view `{}` entry `{name}` uses an absolute target: {entry} -> {actual}",
                    view.label
                ),
            );
        }
        let expected = relative_path(&view.path, &project_root.join(canonical_rel).join(&name))?;
        if actual != expected {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project view `{}` entry `{name}` has target {actual}; expected {expected}",
                    view.label
                ),
            );
        }
        if !canonical_skills.contains(&name) {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project view `{}` entry `{name}` has no canonical skill under {}",
                    view.label, target.canonical_path
                ),
            );
            continue;
        }
        let resolved = resolve_link_target(&view.path, &actual);
        let expected_resolved = project_root.join(canonical_rel).join(&name);
        if resolved != expected_resolved {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project view `{}` entry `{name}` resolves to {resolved}; expected {}",
                    view.label, expected_resolved
                ),
            );
        }
    }

    for skill in canonical_skills.difference(&view_entries) {
        issue(
            issues,
            target,
            Severity::Error,
            format!(
                "canonical skill `{skill}` is missing from project view `{}` at {}",
                view.label, view.path
            ),
        );
    }
    Ok(())
}

fn check_project_aggregator(target: &Target, issues: &mut Vec<Issue>) -> Result<()> {
    let Some(path) = &target.aggregator_path else {
        return Ok(());
    };
    match target.link_strategy {
        LinkStrategy::Symlink => check_project_symlink_aggregator(target, path, issues),
        LinkStrategy::Hardlink => check_project_hardlink_aggregator(target, path, issues),
    }
}

fn check_project_symlink_aggregator(
    target: &Target,
    path: &Utf8Path,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let actual = read_link_utf8(path)?;
            if actual != target.canonical_path {
                issue(
                    issues,
                    target,
                    Severity::Error,
                    format!(
                        "project working-copy symlink {path} points to {actual}; expected {}",
                        target.canonical_path
                    ),
                );
                return Ok(());
            }
            match canonicalize_utf8(path) {
                Ok(resolved) if resolved == canonicalize_utf8(&target.canonical_path)? => {}
                Ok(resolved) => issue(
                    issues,
                    target,
                    Severity::Error,
                    format!(
                        "project working-copy symlink {path} resolves to {resolved}; expected {}",
                        target.canonical_path
                    ),
                ),
                Err(err) => issue(
                    issues,
                    target,
                    Severity::Error,
                    format!("project working-copy symlink {path} is broken: {err}"),
                ),
            }
        }
        Ok(_) => issue(
            issues,
            target,
            Severity::Error,
            format!("project working-copy path {path} exists but is not a symlink"),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => issue(
            issues,
            target,
            Severity::Error,
            format!("project working-copy symlink {path} is missing"),
        ),
        Err(err) => return Err(err).with_context(|| format!("failed to inspect {path}")),
    }
    Ok(())
}

fn check_project_hardlink_aggregator(
    target: &Target,
    path: &Utf8Path,
    issues: &mut Vec<Issue>,
) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            issue(
                issues,
                target,
                Severity::Error,
                format!(
                    "project working copy {path} is a symlink but project uses hardlink strategy; run `skillnet project sync` to materialise the hardlinked copy"
                ),
            );
            return Ok(());
        }
    }

    match hardlink_dir_status(&target.canonical_path, path)? {
        HardlinkStatus::Missing => issue(
            issues,
            target,
            Severity::Error,
            format!(
                "project hardlink working-copy directory {path} is missing; run `skillnet project sync`"
            ),
        ),
        HardlinkStatus::Identical => {}
        HardlinkStatus::Severed { files } => issue(
            issues,
            target,
            Severity::Warn,
            format!(
                "{} project working-copy file{} no longer hardlinked ({}) but content matches; `skillnet project sync` will re-link",
                files.len(),
                plural(files.len()),
                sample_files(&files)
            ),
        ),
        HardlinkStatus::Diverged { files } => issue(
            issues,
            target,
            Severity::Error,
            format!(
                "{} project working-copy file{} diverged from canonical ({}); run `skillnet project sync --force` / `--prefer canonical` to overwrite, or reconcile manually",
                files.len(),
                plural(files.len()),
                sample_files(&files)
            ),
        ),
        HardlinkStatus::Foreign => issue(
            issues,
            target,
            Severity::Error,
            format!("project working-copy path {path} is not a faithful copy (extra/missing entries)"),
        ),
    }
    Ok(())
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn sample_files(files: &[Utf8PathBuf]) -> String {
    const LIMIT: usize = 3;
    let mut names = files
        .iter()
        .take(LIMIT)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if files.len() > LIMIT {
        names.push(format!("+{} more", files.len() - LIMIT));
    }
    names.join(", ")
}

fn canonical_skill_names(canonical_path: &Utf8Path) -> Result<BTreeSet<String>> {
    crate::mirror::mirror_skill_dirs(canonical_path)?
        .into_iter()
        .map(|path| entry_name(&path))
        .collect()
}

fn issue(issues: &mut Vec<Issue>, target: &Target, severity: Severity, message: String) {
    issues.push(Issue {
        severity,
        message,
        scope: target.name.clone(),
    });
}

fn issue_non_symlink(
    issues: &mut Vec<Issue>,
    target: &Target,
    canonical_skill: &Utf8Path,
    view_entry: &Utf8Path,
    prefix: String,
) {
    let (severity, hint) = match compare_view_entry(canonical_skill, view_entry) {
        Ok(outcome) => {
            let (severity, hint) = classify_non_symlink(&outcome);
            (severity, hint.to_string())
        }
        Err(err) => {
            let hint =
                format!("could not classify entry: {err}; rerun doctor or check permissions");
            (Severity::Error, hint)
        }
    };
    issue(issues, target, severity, format!("{prefix}; {hint}"));
}

fn classify_non_symlink(outcome: &ReconcileOutcome) -> (Severity, &'static str) {
    match outcome {
        ReconcileOutcome::Identical => {
            (Severity::Info, "`skillnet sync` will silently demote to symlink")
        }
        ReconcileOutcome::ViewNewer { .. } => {
            (
                Severity::Warn,
                "`skillnet sync --apply-promote` will pull view → canonical and re-link",
            )
        }
        ReconcileOutcome::CanonicalNewer { .. } => {
            (
                Severity::Error,
                "`skillnet sync --force` will destroy view-side edits; review before running",
            )
        }
        ReconcileOutcome::EqualMtimeDifferentContent { .. } => {
            (
                Severity::Error,
                "`skillnet sync --apply-promote --prefer view|canonical` required",
            )
        }
        ReconcileOutcome::BothAdvanced { .. } => {
            (
                Severity::Error,
                "`skillnet sync --apply-promote --prefer view|canonical` required; per-file merge is not supported",
            )
        }
        ReconcileOutcome::AdoptCandidate => {
            (
                Severity::Info,
                "view-only skill; `skillnet sync --apply-promote --adopt-new` to promote",
            )
        }
    }
}

fn read_dir_utf8(path: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("failed to read directory {path}"))? {
        let entry = entry?;
        entries.push(
            Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|path| anyhow::anyhow!("non-UTF-8 path: {}", path.display()))?,
        );
    }
    entries.sort();
    Ok(entries)
}

fn entry_name(path: &Utf8Path) -> Result<String> {
    path.file_name()
        .map(ToString::to_string)
        .with_context(|| format!("path has no final component: {path}"))
}

fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(fs::canonicalize(path)?)
        .map_err(|path| anyhow::anyhow!("non-UTF-8 path: {}", path.display()))
}

fn read_link_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(fs::read_link(path)?).map_err(|target| {
        anyhow::anyhow!("non-UTF-8 symlink target at {path}: {}", target.display())
    })
}

fn resolve_link_target(link_parent: &Utf8Path, target: &Utf8Path) -> Utf8PathBuf {
    if target.is_absolute() {
        return normalize_utf8(target);
    }
    normalize_utf8(&link_parent.join(target))
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
        .map_err(|path| anyhow::anyhow!("non-UTF-8 relative symlink target: {}", path.display()))
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

fn normalize_utf8(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = PathBuf::new();
    for component in path.as_std_path().components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push("/"),
            Component::Normal(part) => out.push(part),
        }
    }
    Utf8PathBuf::from_path_buf(out)
        .unwrap_or_else(|path| Utf8PathBuf::from(path.to_string_lossy().to_string()))
}
