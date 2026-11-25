use anyhow::Result;
use camino::Utf8PathBuf;

use super::Context;
use crate::{
    cli::Scope,
    mirror::mirror_skill_dirs,
    model::{Target, TargetScope},
    templates,
};

pub(super) fn canonical_skill_dirs(target: &Target) -> Result<Vec<Utf8PathBuf>> {
    match target.scope {
        TargetScope::Global => templates::canonical_skill_dirs(&target.canonical_path),
        TargetScope::Project => mirror_skill_dirs(&target.canonical_path),
    }
}

pub(crate) fn list(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        println!("# {}", target.name);
        for skill in canonical_skill_dirs(&target)? {
            println!("{}", skill.file_name().unwrap_or_default());
        }
    }
    Ok(())
}

pub(crate) fn targets(ctx: &Context) -> Result<()> {
    for target in ctx.all_targets()? {
        println!("{}", target.name);
    }
    Ok(())
}
