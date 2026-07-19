use std::fs;

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use super::Context;
use crate::cli::{Scope, SkillPath};
use crate::{
    catalog, fs_ops,
    view::{
        materialize_project_with_options, materialize_view_with_options, ProjectSyncOptions,
        ViewSyncOptions,
    },
};

pub fn show(ctx: &Context, skill_path: &SkillPath) -> Result<()> {
    let target = ctx.target(&skill_path.scope)?;
    let path = target.canonical_path.join(&skill_path.skill);
    ensure_skill_exists(skill_path, &path)?;

    let skill_file = path.join("SKILL.md");
    let body = fs::read_to_string(&skill_file)
        .with_context(|| format!("failed to read skill file {skill_file}"))?;
    let frontmatter = catalog::parse_frontmatter(&body);
    let (file_count, dir_count) = tree_counts(&path)?;
    let entries = top_level_entries(&path)?;
    let catalog_entry = catalog::entry_for(ctx, skill_path)?;

    println!("skill: {}/{}", skill_path.scope, skill_path.skill);
    println!("path:  {}", display_path(&path));
    println!("files: {file_count} files, {dir_count} dirs");
    for entry in entries {
        println!("  {entry}");
    }
    println!();
    println!("frontmatter:");
    println!(
        "  name:        {}",
        frontmatter.name.as_deref().unwrap_or("(none)")
    );
    println!(
        "  description: {}",
        frontmatter.description.as_deref().unwrap_or("(none)")
    );
    println!();

    if let Some(entry) = catalog_entry {
        println!("catalog entry:");
        println!("  description:    {}", empty_as_none(&entry.description));
        println!(
            "  category:       {}",
            entry.category.as_deref().unwrap_or("uncategorized")
        );
        println!("  status:         {}", entry.status);
        println!("  tags:           {}", comma_list(&entry.tags));
        println!("  related_skills: {}", comma_list(&entry.related_skills));
        println!("  scope:          {}", entry.scope);
        if let Some(project) = &entry.project {
            println!("  project:        {project}");
        }
        println!("  catalog_path:   {}", entry.path);
        println!("  lines:          {}", entry.line_count);
        if let Some(note) = &entry.collision_note {
            println!("  collision_note: {note}");
        }
    } else {
        println!("catalog entry: (none - run 'skillnet catalog generate')");
    }

    Ok(())
}

pub fn new(ctx: &Context, skill_path: &SkillPath, view_sync: bool) -> Result<()> {
    let target = ctx.target(&skill_path.scope)?;
    ensure_canonical_clean(ctx, &target)?;
    ensure_manifest_mutation_allowed(&target, &skill_path.skill, "create")?;
    let path = target.canonical_path.join(&skill_path.skill);
    if path.exists() {
        bail!("skill `{}` already exists at {path}", skill_path.skill);
    }

    if ctx.dry_run {
        println!("create {path}");
    } else {
        fs::create_dir_all(&path)?;
        fs::write(path.join("SKILL.md"), skill_template(&skill_path.skill))?;
    }

    if view_sync {
        sync_after_mutation(ctx, &skill_path.scope, false)?;
    }
    Ok(())
}

pub fn delete(ctx: &Context, skill_path: &SkillPath, view_sync: bool) -> Result<()> {
    let target = ctx.target(&skill_path.scope)?;
    ensure_canonical_clean(ctx, &target)?;
    ensure_manifest_mutation_allowed(&target, &skill_path.skill, "delete")?;
    let path = target.canonical_path.join(&skill_path.skill);
    ensure_skill_exists(skill_path, &path)?;
    if ctx.dry_run {
        println!("delete {path}");
    } else {
        fs::remove_dir_all(&path)?;
    }
    if view_sync {
        sync_after_mutation(ctx, &skill_path.scope, true)?;
    }
    Ok(())
}

pub fn rename(
    ctx: &Context,
    skill_path: &SkillPath,
    new: &str,
    force: bool,
    view_sync: bool,
) -> Result<()> {
    let target = ctx.target(&skill_path.scope)?;
    ensure_canonical_clean(ctx, &target)?;
    ensure_manifest_mutation_allowed(&target, &skill_path.skill, "rename")?;
    ensure_manifest_mutation_allowed(&target, new, "rename destination")?;
    let src = target.canonical_path.join(&skill_path.skill);
    let dest = target.canonical_path.join(new);
    ensure_skill_exists(skill_path, &src)?;
    prepare_dest(&src, &dest, force)?;
    if ctx.dry_run {
        println!("rename {src} {dest}");
    } else {
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::rename(&src, &dest)?;
    }
    if view_sync {
        sync_after_mutation(ctx, &skill_path.scope, true)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn move_skill(
    ctx: &Context,
    from_path: &SkillPath,
    to_scope: &Scope,
    as_name: Option<&str>,
    copy: bool,
    force: bool,
    view_sync: bool,
) -> Result<()> {
    let from = ctx.target(&from_path.scope)?;
    let to = ctx.target(to_scope)?;
    ensure_canonical_clean(ctx, &from)?;
    ensure_canonical_clean(ctx, &to)?;
    ensure_manifest_mutation_allowed(&from, &from_path.skill, "move")?;
    ensure_manifest_mutation_allowed(&to, as_name.unwrap_or(&from_path.skill), "move destination")?;
    let src = from.canonical_path.join(&from_path.skill);
    let dest = to.canonical_path.join(as_name.unwrap_or(&from_path.skill));
    ensure_skill_exists(from_path, &src)?;
    prepare_dest(&src, &dest, force)?;

    if ctx.dry_run {
        let action = if copy { "copy" } else { "move" };
        println!("{action} {src} {dest}");
    } else {
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        if copy {
            fs_ops::copy_dir(&src, &dest)?;
        } else {
            fs::create_dir_all(dest.parent().context("destination has no parent")?)?;
            fs::rename(&src, &dest)?;
        }
    }
    if view_sync {
        sync_after_mutation(ctx, &from_path.scope, true)?;
        if &from_path.scope != to_scope || copy {
            sync_after_mutation(ctx, to_scope, false)?;
        }
    }
    Ok(())
}

fn ensure_canonical_clean(ctx: &Context, target: &crate::model::Target) -> Result<()> {
    ctx.ensure_target_clean(&target.canonical_path)
}

fn sync_after_mutation(ctx: &Context, scope: &Scope, allow_delete: bool) -> Result<()> {
    if ctx.dry_run {
        println!("sync views for {scope} (allow_delete: {allow_delete})");
        return Ok(());
    }

    let target = ctx.target(scope)?;
    match scope {
        Scope::Global => {
            if let Some(bundle) = ctx.bundle_plan(&target)? {
                if target.link_strategy != crate::link::LinkStrategy::Symlink {
                    bail!("Skillnet manifest bundles require symlink views");
                }
                bundle.materialize()?;
                for view in &target.views {
                    crate::view::materialize_view_with_expected(
                        bundle.expected_links(),
                        view,
                        ViewSyncOptions {
                            allow_delete,
                            link_strategy: crate::link::LinkStrategy::Symlink,
                            ..ViewSyncOptions::default()
                        },
                    )?;
                }
            } else {
                for view in &target.views {
                    materialize_view_with_options(
                        &target.canonical_path,
                        view,
                        ViewSyncOptions {
                            allow_delete,
                            link_strategy: target.link_strategy,
                            ..ViewSyncOptions::default()
                        },
                    )?;
                }
            }
        }
        Scope::Project(_) => {
            materialize_project_with_options(
                &target,
                ProjectSyncOptions {
                    allow_delete,
                    force: false,
                    link_strategy: target.link_strategy,
                },
            )?;
        }
    }
    Ok(())
}

fn ensure_manifest_mutation_allowed(
    target: &crate::model::Target,
    skill: &str,
    operation: &str,
) -> Result<()> {
    let Some(manifest) = crate::manifest::load(&target.canonical_path)? else {
        return Ok(());
    };
    if manifest.document.skills.contains_key(skill) {
        bail!(
            "cannot {operation} manifest-managed skill `{skill}`; update {} first",
            manifest.path
        );
    }
    Ok(())
}

fn skill_template(name: &str) -> String {
    format!(
        r#"---
name: {name}
description: TODO
---

# {name}
"#
    )
}

fn prepare_dest(src: &Utf8Path, dest: &Utf8Path, force: bool) -> Result<()> {
    if !dest.exists() {
        return Ok(());
    }
    fs_ops::ensure_skill_dir(dest)?;
    if force {
        return Ok(());
    }

    let src_mtime = fs_ops::newest_mtime_nanos(src)?;
    let dest_mtime = fs_ops::newest_mtime_nanos(dest)?;
    if src_mtime > dest_mtime {
        return Ok(());
    }
    if src_mtime == dest_mtime
        && fs_ops::content_signature(src)? == fs_ops::content_signature(dest)?
    {
        return Ok(());
    }
    bail!("destination `{dest}` exists and is not older; pass --force to overwrite");
}

fn ensure_skill_exists(skill_path: &SkillPath, path: &Utf8Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "skill `{}` not found in scope `{}`",
            skill_path.skill,
            skill_path.scope
        );
    }
    fs_ops::ensure_skill_dir(path)
}

fn tree_counts(path: &Utf8Path) -> Result<(usize, usize)> {
    let mut files = 0;
    let mut dirs = 0;
    for entry in WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            dirs += 1;
        } else if entry.file_type().is_file() || entry.file_type().is_symlink() {
            files += 1;
        }
    }
    Ok((files, dirs))
}

fn top_level_entries(path: &Utf8Path) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        let name = entry_path.file_name().unwrap_or_default();
        let metadata = fs::symlink_metadata(&entry_path)?;
        let rendered = if metadata.is_dir() {
            format!("{name}/")
        } else {
            format!("{name} ({} bytes)", metadata.len())
        };
        entries.push(rendered);
    }
    entries.sort();
    Ok(entries)
}

fn display_path(path: &Utf8Path) -> String {
    fs::canonicalize(path)
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .unwrap_or_else(|| path.to_path_buf())
        .to_string()
}

fn comma_list(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

fn empty_as_none(value: &str) -> &str {
    if value.is_empty() {
        "(none)"
    } else {
        value
    }
}
