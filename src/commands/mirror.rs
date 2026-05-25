use anyhow::Result;

use super::Context;
use crate::cli::Scope;
use crate::mirror::mirror_skill_dirs;

pub fn list(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        println!("# {}", target.name);
        for skill in mirror_skill_dirs(&target.canonical_path)? {
            println!("{}", skill.file_name().unwrap_or_default());
        }
    }
    Ok(())
}

pub fn targets(ctx: &Context) -> Result<()> {
    for target in ctx.all_targets()? {
        println!("{}", target.name);
    }
    Ok(())
}
