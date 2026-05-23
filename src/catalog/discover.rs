use std::fs;

use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::commands::Context;

use super::config::CatalogConfig;
use super::entry::SkillEntry;
use super::frontmatter::parse_frontmatter;

pub(super) fn load_entries(ctx: &Context, config: &CatalogConfig) -> Result<Vec<SkillEntry>> {
    let mut entries = Vec::new();
    let global_root = ctx.mirror_root.join("global");
    if global_root.exists() {
        for skill in skill_dirs(&global_root)? {
            entries.push(entry_from_skill(ctx, config, &skill, None)?);
        }
    }

    let projects_root = ctx.mirror_root.join("projects");
    if projects_root.exists() {
        for project in sorted_child_dirs(&projects_root)? {
            let project_name = project
                .file_name()
                .context("project path has no final component")?
                .to_string();
            for skill in skill_dirs(&project)? {
                entries.push(entry_from_skill(ctx, config, &skill, Some(&project_name))?);
            }
        }
    }

    entries.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
    Ok(entries)
}

fn entry_from_skill(
    ctx: &Context,
    config: &CatalogConfig,
    skill_dir: &Utf8Path,
    project: Option<&str>,
) -> Result<SkillEntry> {
    let skill_file = skill_dir.join("SKILL.md");
    let body = fs::read_to_string(&skill_file)
        .with_context(|| format!("failed to read skill file {skill_file}"))?;
    let frontmatter = parse_frontmatter(&body);
    let name = skill_dir
        .file_name()
        .unwrap_or("unknown")
        .trim()
        .to_string();
    let rel = relative_to(&ctx.mirror_root, skill_dir);
    let mut entry = SkillEntry {
        qualified_name: match project {
            Some(project) => format!("{project}/{name}"),
            None => format!("global/{name}"),
        },
        name,
        scope: if project.is_some() {
            "project".to_string()
        } else {
            "global".to_string()
        },
        project: project.map(ToOwned::to_owned),
        category: None,
        status: "active".to_string(),
        tags: Vec::new(),
        related_skills: Vec::new(),
        collision_note: None,
        description: frontmatter.description.unwrap_or_default(),
        path: skill_dir.to_path_buf(),
        line_count: body.lines().count(),
    };

    for rule in &config.rules {
        if rule.matches(&entry, &rel) {
            rule.apply(&mut entry);
        }
    }
    entry.tags.sort();
    entry.tags.dedup();
    entry.related_skills.sort();
    entry.related_skills.dedup();
    Ok(entry)
}

fn skill_dirs(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in mirror: {}", p.display()))?;
        if path.is_dir() && path.join("SKILL.md").is_file() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn sorted_child_dirs(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in mirror: {}", p.display()))?;
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn relative_to(root: &Utf8Path, path: &Utf8Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .as_str()
        .trim_start_matches('/')
        .to_string()
}
