use std::{fs, process::Command as StdCommand};

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::Utf8Path;
use serde::Serialize;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use super::{status::promotion_status_counts, view::format_view_summary, Context};
use crate::cli::args::StatusFormat;
use crate::config::expand_path;
use crate::view::{
    materialize_project_with_options, project_diff, project_status, AggregatorStatus, DriftEntry,
    DriftKind, FileDeltaKind, ProjectSyncOptions,
};

pub fn project_list(ctx: &Context) {
    for project in &ctx.config.projects {
        println!("{}\t{}", project.name, project.path);
    }
}

pub fn project_add(ctx: &Context, name: &str, path: &Utf8Path, allow_missing: bool) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        bail!("project name must be a non-empty scope name, not a path");
    }
    if name == "global" || name == "all" || name == "project" {
        bail!("`{name}` is reserved and cannot be used as a project name");
    }
    if ctx.config.projects.iter().any(|p| p.name == name) {
        bail!("project `{name}` is already configured");
    }

    let expanded_path = expand_path(path.as_str())?;
    if !allow_missing && !expanded_path.is_dir() {
        bail!("project path `{expanded_path}` does not exist or is not a directory");
    }

    if ctx.dry_run {
        println!("add project {name}\t{expanded_path}");
        return Ok(());
    }

    let mut doc = load_config_doc(&ctx.config_path)?;
    projects_array_mut(&mut doc)?.push(project_table(name, &expanded_path));
    write_config_doc(&ctx.config_path, &doc)?;
    println!("added project {name}");
    Ok(())
}

pub fn project_remove(ctx: &Context, name: &str, prune_mirror: bool) -> Result<()> {
    if prune_mirror {
        ctx.ensure_destination_clean()?;
    }
    let project = ctx
        .project(name)
        .with_context(|| format!("unknown project `{name}`"))?;
    let mirror_path = ctx.mirror_root.join("projects").join(&project.name);

    if ctx.dry_run {
        println!("remove project {name}");
        if prune_mirror {
            println!("delete mirror {mirror_path}");
        }
        return Ok(());
    }

    let mut doc = load_config_doc(&ctx.config_path)?;
    remove_project_from_doc(&mut doc, name)?;
    write_config_doc(&ctx.config_path, &doc)?;

    if prune_mirror && mirror_path.exists() {
        fs::remove_dir_all(&mirror_path)?;
    }

    println!("removed project {name}");
    Ok(())
}

pub fn project_sync(
    ctx: &Context,
    names: &[String],
    all: bool,
    allow_delete: bool,
    force: bool,
) -> Result<()> {
    for target in project_targets(ctx, names, all)? {
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
            println!("# project sync {}", target.name);
            println!("from: {}", target.canonical_path);
            println!("allow_delete: {allow_delete}");
            println!("force: {force}");
            for view in &target.views {
                println!("to: {}\t{}", view.label, view.path);
            }
            if let Some(aggregator) = &target.aggregator_path {
                println!("aggregator: {aggregator}");
            }
            continue;
        }

        let summary = materialize_project_with_options(
            &target,
            ProjectSyncOptions {
                allow_delete,
                force,
            },
        )?;
        println!("# {}", target.name);
        for view in &summary.views {
            println!(
                "{}  {}",
                view.label,
                format_view_summary(&view.path, &view.summary)
            );
        }
        if let Some(status) = summary.aggregator {
            println!("aggregator  {}", format_aggregator_status(status));
        }
    }
    Ok(())
}

pub fn project_clone_all(ctx: &Context, all: bool, dry_run: bool, ssh_strict: bool) -> Result<()> {
    if !all {
        bail!("must pass --all");
    }

    let dry_run = dry_run || ctx.dry_run;
    let mut cloned_or_planned = false;
    for project in &ctx.config.projects {
        let path = expand_path(&project.path)
            .with_context(|| format!("failed to resolve project path for `{}`", project.name))?;
        if path.exists() {
            continue;
        }
        let Some(origin) = project
            .origin
            .as_deref()
            .filter(|origin| !origin.is_empty())
        else {
            eprintln!(
                "warn: [{}] no origin configured; skipping clone to {}",
                project.name, path
            );
            continue;
        };
        if ssh_strict && is_https_origin(origin) {
            bail!(
                "project `{}` origin `{origin}` uses HTTPS; pass --ssh-strict=false to allow it",
                project.name
            );
        }

        cloned_or_planned = true;
        if dry_run {
            println!("clone {}\t{}\t{}", project.name, origin, path);
            continue;
        }

        create_parent_dir(&path)?;
        let status = StdCommand::new("git")
            .args(["clone", origin, path.as_str()])
            .status()
            .with_context(|| format!("failed to run git clone for project `{}`", project.name))?;
        if !status.success() {
            bail!(
                "git clone failed for project `{}` with status {status}",
                project.name
            );
        }
    }

    if cloned_or_planned {
        if dry_run {
            println!("project sync --all");
        } else {
            project_sync(ctx, &[], true, false, false)?;
        }
    }

    Ok(())
}

pub fn project_status_command(
    ctx: &Context,
    names: &[String],
    all: bool,
    format: StatusFormat,
) -> Result<()> {
    let mut rows = Vec::new();
    for target in project_targets(ctx, names, all)? {
        let drift = project_status(&target)?;
        let promotion_status = promotion_status_counts(&drift);
        rows.push(ProjectStatusRow {
            name: target.name.clone(),
            would_promote: promotion_status.would_promote,
            needs_tie_break: promotion_status.needs_tie_break,
            drift,
        });
    }

    match format {
        StatusFormat::Text => {
            for row in &rows {
                if row.drift.is_empty() {
                    println!("{}  clean", row.name);
                } else {
                    println!("{}  drift ({} entries)", row.name, row.drift.len());
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

pub fn project_diff_command(ctx: &Context, names: &[String], all: bool) -> Result<()> {
    for target in project_targets(ctx, names, all)? {
        println!("# {}", target.name);
        let deltas = project_diff(&target)?;
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

fn project_targets(
    ctx: &Context,
    names: &[String],
    all: bool,
) -> Result<Vec<crate::model::Target>> {
    if all && !names.is_empty() {
        bail!("use either --all or --name, not both");
    }
    if !all && names.is_empty() {
        bail!("must pass --name or --all");
    }

    let projects = if all {
        ctx.config
            .projects
            .iter()
            .map(|project| project.name.clone())
            .collect::<Vec<_>>()
    } else {
        names.to_vec()
    };

    let mut targets = Vec::with_capacity(projects.len());
    for name in projects {
        let project = ctx
            .project(&name)
            .with_context(|| format!("unknown project `{name}`"))?;
        targets.push(ctx.config.project_target(&ctx.mirror_root, project)?);
    }
    Ok(targets)
}

fn format_aggregator_status(status: AggregatorStatus) -> &'static str {
    match status {
        AggregatorStatus::Created => "created",
        AggregatorStatus::Updated => "updated",
        AggregatorStatus::Unchanged => "unchanged",
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
struct ProjectStatusRow {
    name: String,
    would_promote: usize,
    needs_tie_break: usize,
    drift: Vec<DriftEntry>,
}

fn load_config_doc(path: &Utf8Path) -> Result<DocumentMut> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read config file {path}"))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse config file {path}"))
}

fn write_config_doc(path: &Utf8Path, doc: &DocumentMut) -> Result<()> {
    fs::write(path, doc.to_string()).with_context(|| format!("failed to write config file {path}"))
}

fn projects_array_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables> {
    if doc.get("projects").is_none() {
        doc["projects"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    doc["projects"]
        .as_array_of_tables_mut()
        .context("config `projects` entry is not an array of tables")
}

fn project_table(name: &str, path: &Utf8Path) -> Table {
    let mut table = Table::new();
    table["name"] = value(name);
    table["path"] = value(path.as_str());
    table
}

fn create_parent_dir(path: &Utf8Path) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("project path {path} has no parent directory"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create parent directory {parent}"))
}

fn is_https_origin(origin: &str) -> bool {
    origin.starts_with("https://") || origin.starts_with("http://")
}

fn remove_project_from_doc(doc: &mut DocumentMut, name: &str) -> Result<()> {
    let projects = projects_array_mut(doc)?;
    let idx = projects
        .iter()
        .position(|project| project.get("name").and_then(Item::as_str) == Some(name))
        .with_context(|| format!("unknown project `{name}`"))?;
    projects.remove(idx);
    Ok(())
}
