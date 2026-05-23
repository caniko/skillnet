use std::fs;

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::Utf8Path;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

use super::Context;
use crate::config::expand_path;

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

fn remove_project_from_doc(doc: &mut DocumentMut, name: &str) -> Result<()> {
    let projects = projects_array_mut(doc)?;
    let idx = projects
        .iter()
        .position(|project| project.get("name").and_then(Item::as_str) == Some(name))
        .with_context(|| format!("unknown project `{name}`"))?;
    projects.remove(idx);
    Ok(())
}
