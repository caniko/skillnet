use std::fs;

use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::commands::Context;

use super::config::CatalogConfig;
use super::entry::SkillEntry;
use super::frontmatter::parse_frontmatter;

pub(super) fn load_entries(ctx: &Context, config: &CatalogConfig) -> Result<Vec<SkillEntry>> {
    let mut entries = Vec::new();
    let global_target = ctx.config.global_target(&ctx.mirror_root)?;
    if global_target.canonical_path.exists() {
        for skill in skill_dirs(&global_target.canonical_path)? {
            entries.push(entry_from_skill(config, &skill, None)?);
        }
    }

    for project in &ctx.config.projects {
        let target = ctx.config.project_target(&ctx.mirror_root, project)?;
        if target.canonical_path.exists() {
            for skill in skill_dirs(&target.canonical_path)? {
                entries.push(entry_from_skill(config, &skill, Some(&project.name))?);
            }
        }
    }

    entries.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
    Ok(entries)
}

fn entry_from_skill(
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
    let rel = match project {
        Some(project) => format!("projects/{project}/{name}"),
        None => format!("global/{name}"),
    };
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
