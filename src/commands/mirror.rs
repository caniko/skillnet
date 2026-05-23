use anyhow::Result;

use super::Context;
use crate::cli::Scope;
use crate::reconcile as reconcile_ops;

pub fn list(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        println!("# {}", target.name);
        for skill in reconcile_ops::mirror_skill_dirs(&target.mirror_path)? {
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

pub fn sources(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        println!("# {}", target.name);
        for source in target.sources {
            println!("{}\t{}\t{}", source.label, source.priority, source.path);
        }
    }
    Ok(())
}
