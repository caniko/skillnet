use std::{collections::BTreeSet, fs, process::Command as StdCommand};

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::{
    commands::Context,
    config::{default_data_dir, expand_path, SubscriptionConfig, SubscriptionDeletePolicy},
    fs_ops::copy_dir,
};

pub fn sync(ctx: &Context, names: &[String], all: bool) -> Result<()> {
    let selected = selected_subscriptions(ctx, names, all)?;
    for (name, subscription) in selected {
        sync_one(ctx, name, subscription)?;
    }
    Ok(())
}

fn selected_subscriptions<'a>(
    ctx: &'a Context,
    names: &[String],
    all: bool,
) -> Result<Vec<(&'a str, &'a SubscriptionConfig)>> {
    if all && !names.is_empty() {
        bail!("use either --all or explicit subscription names, not both");
    }
    if !all && names.is_empty() {
        bail!("must pass --all or at least one subscription name");
    }

    if all {
        return Ok(ctx
            .config
            .subscriptions
            .iter()
            .map(|(name, subscription)| (name.as_str(), subscription))
            .collect());
    }

    names
        .iter()
        .map(|name| {
            ctx.config
                .subscriptions
                .get_key_value(name)
                .map(|(name, subscription)| (name.as_str(), subscription))
                .with_context(|| format!("unknown subscription `{name}`"))
        })
        .collect()
}

fn sync_one(ctx: &Context, name: &str, subscription: &SubscriptionConfig) -> Result<()> {
    let checkout = checkout_path(name);
    let source = checkout.join(subscription.source.trim_matches('/'));
    let target = expand_path(&subscription.target)
        .with_context(|| format!("failed to resolve target for subscription `{name}`"))?;
    reject_canonical_target(ctx, name, &target)?;

    if ctx.dry_run {
        if checkout.join(".git").exists() {
            println!(
                "would fetch subscription {name} from {} at {}",
                subscription.url, checkout
            );
        } else {
            println!(
                "would clone subscription {name} from {} to {}",
                subscription.url, checkout
            );
        }
        println!(
            "would checkout subscription {name} ref {}",
            subscription.ref_name
        );
        println!(
            "would sync subscription {name} {} -> {} ({:?})",
            source, target, subscription.delete_policy
        );
        return Ok(());
    }

    update_checkout(name, subscription, &checkout)?;
    if !source.is_dir() {
        bail!(
            "subscription `{name}` source `{}` does not exist or is not a directory",
            source
        );
    }

    materialize_source(&source, &target, subscription.delete_policy)
        .with_context(|| format!("failed to sync subscription `{name}` to {target}"))?;
    println!("synced subscription {name} -> {target}");
    Ok(())
}

fn reject_canonical_target(ctx: &Context, name: &str, target: &Utf8Path) -> Result<()> {
    for scope in ctx.config.targets(&ctx.mirror_root)? {
        if target == scope.canonical_path || target.starts_with(&scope.canonical_path) {
            bail!(
                "subscription `{name}` target `{target}` is inside canonical scope `{scope}`; subscriptions must write to a separate non-canonical directory",
                name = name,
                target = target,
                scope = scope.canonical_path
            );
        }
    }
    Ok(())
}

fn checkout_path(name: &str) -> Utf8PathBuf {
    Utf8PathBuf::from_path_buf(default_data_dir())
        .unwrap_or_else(|path| Utf8PathBuf::from(path.to_string_lossy().into_owned()))
        .join("subscriptions")
        .join(name)
        .join("repo")
}

fn update_checkout(
    name: &str,
    subscription: &SubscriptionConfig,
    checkout: &Utf8Path,
) -> Result<()> {
    if checkout.join(".git").exists() {
        git(
            checkout,
            ["fetch", "--prune", "origin", subscription.ref_name.as_str()],
        )
        .with_context(|| format!("failed to fetch subscription `{name}`"))?;
    } else {
        let parent = checkout
            .parent()
            .with_context(|| format!("subscription checkout path {checkout} has no parent"))?;
        fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
        let status = StdCommand::new("git")
            .args([
                "clone",
                "--no-checkout",
                subscription.url.as_str(),
                checkout.as_str(),
            ])
            .status()
            .with_context(|| format!("failed to run git clone for subscription `{name}`"))?;
        if !status.success() {
            bail!("git clone failed for subscription `{name}` with status {status}");
        }
        git(
            checkout,
            ["fetch", "--prune", "origin", subscription.ref_name.as_str()],
        )
        .with_context(|| format!("failed to fetch subscription `{name}`"))?;
    }

    git(checkout, ["checkout", "--force", "FETCH_HEAD"])
        .with_context(|| format!("failed to checkout subscription `{name}`"))?;
    Ok(())
}

fn git<const N: usize>(cwd: &Utf8Path, args: [&str; N]) -> Result<()> {
    let status = StdCommand::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .with_context(|| format!("failed to run git in {cwd}"))?;
    if !status.success() {
        bail!("git command failed in `{cwd}` with status {status}");
    }
    Ok(())
}

fn materialize_source(
    source: &Utf8Path,
    target: &Utf8Path,
    delete_policy: SubscriptionDeletePolicy,
) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create {target}"))?;

    let mut source_names = BTreeSet::new();
    for entry in fs::read_dir(source).with_context(|| format!("failed to read {source}"))? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let source_entry = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 source path: {}", path.display()))?;
        if !is_skill_dir(&source_entry)? {
            continue;
        }
        source_names.insert(name.to_string());
        copy_dir(&source_entry, &target.join(name.as_ref()))?;
    }

    if delete_policy == SubscriptionDeletePolicy::Prune {
        prune_target(target, &source_names)?;
    }

    Ok(())
}

fn is_skill_dir(path: &Utf8Path) -> Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.file_type().is_dir() && path.join("SKILL.md").is_file())
}

fn prune_target(target: &Utf8Path, source_names: &BTreeSet<String>) -> Result<()> {
    for entry in WalkDir::new(target)
        .follow_links(false)
        .min_depth(1)
        .max_depth(1)
    {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|path| anyhow::anyhow!("non-UTF-8 target path: {}", path.display()))?;
        let Some(name) = path.file_name() else {
            continue;
        };
        if name.starts_with('.') || source_names.contains(name) || !is_skill_dir(&path)? {
            continue;
        }
        fs::remove_dir_all(&path).with_context(|| format!("failed to prune {path}"))?;
    }
    Ok(())
}
