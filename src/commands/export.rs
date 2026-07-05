use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use super::Context;
use crate::{fs_ops::copy_dir, model::TargetScope};

const DEFAULT_SOURCE: &str = "skills";

pub fn run(
    ctx: &Context,
    source: Option<&Utf8Path>,
    skills: &[String],
    prune: bool,
    view_sync: bool,
) -> Result<()> {
    let source = source
        .map(Utf8Path::to_path_buf)
        .unwrap_or_else(|| Utf8PathBuf::from(DEFAULT_SOURCE));
    let source = crate::config::expand_path(source.as_str())
        .with_context(|| format!("failed to resolve source directory `{source}`"))?;
    if !source.is_dir() {
        bail!("export source `{source}` does not exist or is not a directory");
    }

    let target = ctx.config.global_target(&ctx.mirror_root)?;
    if target.scope != TargetScope::Global {
        bail!("export currently supports only the global destination scope");
    }

    let plan = export_plan(&source, skills)?;
    if plan.skills.is_empty() {
        println!("no skills to export from {source}");
        return Ok(());
    }

    if ctx.dry_run {
        println!("# export global");
        println!("from: {source}");
        println!("to: {}", target.canonical_path);
        for skill in &plan.skills {
            println!("copy {}\t{}", skill.name, skill.source);
        }
        if prune {
            for stale in stale_destination_skills(&target.canonical_path, &plan.source_names)? {
                println!("prune {stale}");
            }
        }
        if view_sync {
            println!("view sync global");
        }
        return Ok(());
    }

    ctx.ensure_target_clean(&target.canonical_path)?;
    fs::create_dir_all(&target.canonical_path)
        .with_context(|| format!("failed to create {}", target.canonical_path))?;

    for skill in &plan.skills {
        let dest = target.canonical_path.join(&skill.name);
        copy_dir(&skill.source, &dest)
            .with_context(|| format!("failed to export skill `{}` to {dest}", skill.name))?;
        println!("exported {}", skill.name);
    }

    if prune {
        for stale in stale_destination_skills(&target.canonical_path, &plan.source_names)? {
            fs::remove_dir_all(&stale).with_context(|| format!("failed to prune {stale}"))?;
            println!("pruned {stale}");
        }
    }

    if view_sync {
        super::view::sync(ctx, false, false, None)?;
    }

    Ok(())
}

#[derive(Debug)]
struct ExportPlan {
    skills: Vec<ExportSkill>,
    source_names: BTreeSet<String>,
}

#[derive(Debug)]
struct ExportSkill {
    name: String,
    source: Utf8PathBuf,
}

fn export_plan(source: &Utf8Path, skills: &[String]) -> Result<ExportPlan> {
    if skills.is_empty() {
        return discover_source_skills(source);
    }

    let discovered = discover_source_skills(source)?;
    let by_name: BTreeMap<_, _> = discovered
        .skills
        .into_iter()
        .map(|skill| (skill.name.clone(), skill))
        .collect();
    let mut seen = BTreeSet::new();
    let mut planned = Vec::new();
    for name in skills {
        validate_skill_name(name)?;
        if !seen.insert(name.clone()) {
            continue;
        }
        let path = source.join(name);
        if !path.is_dir() {
            bail!("requested skill `{name}` does not exist as a directory under {source}");
        }
        let Some(skill) = by_name.get(name) else {
            bail!("requested skill `{name}` is missing required SKILL.md under {path}");
        };
        planned.push(ExportSkill {
            name: skill.name.clone(),
            source: skill.source.clone(),
        });
    }

    Ok(ExportPlan {
        source_names: discovered.source_names,
        skills: planned,
    })
}

fn discover_source_skills(source: &Utf8Path) -> Result<ExportPlan> {
    let mut planned = Vec::new();
    let mut source_names = BTreeSet::new();
    for entry in fs::read_dir(source).with_context(|| format!("failed to read {source}"))? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 source path: {}", path.display()))?;
        let Some(name) = path.file_name().map(ToOwned::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_dir() {
            bail!("source entry `{path}` is not a directory");
        }
        if !is_skill_dir(&path)? {
            continue;
        }
        validate_skill_name(&name)?;
        source_names.insert(name.to_owned());
        planned.push(ExportSkill {
            name: name.to_owned(),
            source: path,
        });
    }
    planned.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(ExportPlan {
        skills: planned,
        source_names,
    })
}

fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\\')
        || name == "global"
        || name == "all"
        || name == "projects"
    {
        bail!("invalid skill name `{name}`");
    }
    Ok(())
}

fn is_skill_dir(path: &Utf8Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.file_type().is_dir() && path.join("SKILL.md").is_file())
}

fn stale_destination_skills(
    target: &Utf8Path,
    source_names: &BTreeSet<String>,
) -> Result<Vec<Utf8PathBuf>> {
    if !target.exists() {
        return Ok(Vec::new());
    }

    let mut stale = Vec::new();
    for entry in WalkDir::new(target)
        .follow_links(false)
        .min_depth(1)
        .max_depth(1)
    {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 destination path: {}", path.display()))?;
        let Some(name) = path.file_name() else {
            continue;
        };
        if name.starts_with('.') || source_names.contains(name) || !is_skill_dir(&path)? {
            continue;
        }
        stale.push(path);
    }
    stale.sort();
    Ok(stale)
}
