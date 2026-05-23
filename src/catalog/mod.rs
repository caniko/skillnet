mod config;
mod discover;
mod entry;
mod frontmatter;
mod render;
mod validate;

use anyhow::{bail, Result};

use crate::cli::SkillPath;
use crate::commands::Context;

use config::CatalogConfig;
use discover::load_entries;
use entry::SkillEntry;
use render::{
    render_catalog, render_conflicts, render_routing, write_generated_doc, write_project_indexes,
};
use validate::validate_entries;

pub(crate) use frontmatter::parse_frontmatter;

pub fn generate(ctx: &Context) -> Result<()> {
    let config = CatalogConfig::load(&ctx.catalog_config_path)?;
    let entries = load_entries(ctx, &config)?;
    let lint_errors = validate_entries(&entries, &config);
    if !lint_errors.is_empty() {
        bail!("catalog metadata is invalid:\n{}", lint_errors.join("\n"));
    }

    write_generated_doc(
        &ctx.mirror_root.join("CATALOG.md"),
        &render_catalog(&entries)?,
    )?;
    write_generated_doc(
        &ctx.mirror_root.join("ROUTING.md"),
        &render_routing(&entries)?,
    )?;
    write_generated_doc(
        &ctx.mirror_root.join("SKILL_CONFLICTS.md"),
        &render_conflicts(&entries),
    )?;
    write_project_indexes(ctx, &entries)?;
    println!("generated catalog for {} skills", entries.len());
    Ok(())
}

pub fn lint(ctx: &Context) -> Result<()> {
    let config = CatalogConfig::load(&ctx.catalog_config_path)?;
    let entries = load_entries(ctx, &config)?;
    let errors = validate_entries(&entries, &config);
    if errors.is_empty() {
        println!("catalog lint passed for {} skills", entries.len());
        Ok(())
    } else {
        bail!("catalog lint failed:\n{}", errors.join("\n"))
    }
}

pub(crate) fn entry_for(ctx: &Context, path: &SkillPath) -> Result<Option<SkillEntry>> {
    let config = CatalogConfig::load(&ctx.catalog_config_path)?;
    let entries = load_entries(ctx, &config)?;
    let qualified_name = format!("{}/{}", path.scope, path.skill);
    Ok(entries
        .into_iter()
        .find(|entry| entry.qualified_name == qualified_name))
}

pub fn search(ctx: &Context, query: &str) -> Result<()> {
    let config = CatalogConfig::load(&ctx.catalog_config_path)?;
    let query = query.to_lowercase();
    let entries = load_entries(ctx, &config)?;
    for entry in entries.iter().filter(|entry| entry.matches(&query)) {
        println!(
            "{}\t{}\t{}\t{}",
            entry.qualified_name,
            entry.category.as_deref().unwrap_or("uncategorized"),
            entry.status,
            entry.description
        );
    }
    Ok(())
}
