use std::{collections::BTreeSet, fs, process::Command as StdCommand};

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::{
    commands::Context,
    config::{expand_path, SubscriptionConfig, SubscriptionDeletePolicy},
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
    validate_subscription(name, subscription)?;
    let checkout = checkout_path(&ctx.data_dir, name);
    if subscription.provider {
        validate_provider_storage(ctx, name)?;
    }
    let target = subscription
        .target
        .as_deref()
        .map(expand_path)
        .transpose()
        .with_context(|| format!("failed to resolve target for subscription `{name}`"))?;
    if let Some(target) = &target {
        reject_canonical_target(ctx, name, target)?;
    }

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
        if subscription.provider {
            println!("would compose subscription {name} into global skill views");
        } else {
            println!(
                "would sync subscription {name} {} -> {} ({:?})",
                checkout.join(&subscription.source),
                target.as_ref().expect("validated copy subscription target"),
                subscription.delete_policy
            );
        }
        return Ok(());
    }

    if !subscription.provider {
        update_checkout(name, subscription, &checkout)?;
        let source = provider_source_path(&ctx.data_dir, name, subscription)?
            .context("updated subscription source is missing")?;
        let target = target.expect("validated copy subscription target");
        materialize_source(&source, &target, subscription.delete_policy)
            .with_context(|| format!("failed to sync subscription `{name}` to {target}"))?;
        println!("synced subscription {name} -> {target}");
        return Ok(());
    }

    let previous = checkout_head(&checkout)?;
    let mut compose_started = false;
    let result: Result<()> = (|| {
        update_checkout(name, subscription, &checkout)?;
        provider_source_path(&ctx.data_dir, name, subscription)?
            .context("updated provider source is missing")?;
        compose_started = true;
        super::view::sync(ctx, true, false, None)?;
        println!("synced provider subscription {name}");
        Ok(())
    })();
    if let Err(error) = result {
        let original = format!("{error:#}");
        if let Err(rollback) = rollback_checkout(name, &checkout, previous.as_deref()) {
            bail!("provider subscription `{name}` update failed: {original}; rollback also failed: {rollback:#}");
        }
        if previous.is_some() || compose_started {
            if let Err(restore) = super::view::sync(ctx, true, false, None) {
                bail!("provider subscription `{name}` update failed: {original}; checkout was restored but view restoration failed: {restore:#}");
            }
        }
        let recovery = if previous.is_some() {
            "restored last-known-good checkout"
        } else {
            "discarded rejected initial checkout"
        };
        return Err(anyhow::anyhow!(original)).with_context(|| {
            format!("provider subscription `{name}` update was rejected; {recovery}")
        });
    }
    Ok(())
}

fn reject_canonical_target(ctx: &Context, name: &str, target: &Utf8Path) -> Result<()> {
    reject_canonical_overlap(ctx, name, "target", target)
}

pub(crate) fn validate_provider_storage(ctx: &Context, name: &str) -> Result<()> {
    reject_canonical_overlap(ctx, name, "checkout", &checkout_path(&ctx.data_dir, name))?;
    reject_canonical_overlap(ctx, name, "bundle", &ctx.data_dir.join("bundles/global"))
}

fn reject_canonical_overlap(ctx: &Context, name: &str, kind: &str, path: &Utf8Path) -> Result<()> {
    let resolved = resolve_existing_ancestor(path)?;
    for scope in ctx.config.targets(&ctx.mirror_root)? {
        let canonical = resolve_existing_ancestor(&scope.canonical_path)?;
        if resolved == canonical
            || resolved.starts_with(&canonical)
            || canonical.starts_with(&resolved)
        {
            bail!(
                "subscription `{name}` {kind} `{path}` overlaps canonical scope `{}`; subscriptions must use separate storage",
                scope.canonical_path,
            );
        }
    }
    Ok(())
}

fn resolve_existing_ancestor(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();
    while !current.exists() {
        missing.push(
            current
                .file_name()
                .with_context(|| format!("path has no existing ancestor: {path}"))?
                .to_string(),
        );
        current = current
            .parent()
            .with_context(|| format!("path has no existing ancestor: {path}"))?
            .to_path_buf();
    }
    let mut resolved = canonical_utf8(&current)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn checkout_path(data_dir: &Utf8Path, name: &str) -> Utf8PathBuf {
    data_dir.join("subscriptions").join(name).join("repo")
}

pub(crate) fn provider_source_path(
    data_dir: &Utf8Path,
    name: &str,
    subscription: &SubscriptionConfig,
) -> Result<Option<Utf8PathBuf>> {
    validate_subscription(name, subscription)?;
    let checkout = checkout_path(data_dir, name);
    let source = checkout.join(&subscription.source);
    if !source.exists() {
        return Ok(None);
    }
    let checkout = canonical_utf8(&checkout)?;
    let source = canonical_utf8(&source)?;
    if source != checkout && !source.starts_with(&checkout) {
        bail!("subscription `{name}` source escapes its checkout: {source}");
    }
    if !source.is_dir() {
        bail!("subscription `{name}` source is not a directory: {source}");
    }
    Ok(Some(source))
}

fn validate_subscription(name: &str, subscription: &SubscriptionConfig) -> Result<()> {
    let name_path = Utf8Path::new(name);
    if name.is_empty()
        || name == "."
        || name == ".."
        || name_path.components().count() != 1
        || name_path.is_absolute()
    {
        bail!("subscription name must be one path component: `{name}`");
    }
    if subscription.ref_name.is_empty() || subscription.ref_name.starts_with('-') {
        bail!("subscription `{name}` has an invalid Git ref");
    }
    let source = Utf8Path::new(&subscription.source);
    if subscription.source.is_empty()
        || source.is_absolute()
        || source.components().any(|component| {
            matches!(
                component,
                Utf8Component::ParentDir | Utf8Component::RootDir | Utf8Component::Prefix(_)
            )
        })
    {
        bail!("subscription `{name}` source must stay within its checkout");
    }
    match (subscription.provider, subscription.target.as_deref()) {
        (true, Some(_)) => bail!("provider subscription `{name}` must not set target"),
        (false, None) => bail!("copy subscription `{name}` must set target"),
        _ => Ok(()),
    }
}

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    let canonical = path
        .canonicalize_utf8()
        .with_context(|| format!("failed to canonicalize {path}"))?;
    Ok(canonical)
}

fn checkout_head(checkout: &Utf8Path) -> Result<Option<String>> {
    if !checkout.join(".git").exists() {
        return Ok(None);
    }
    let output = StdCommand::new("git")
        .current_dir(checkout)
        .args(["rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("failed to inspect subscription checkout {checkout}"))?;
    if !output.status.success() {
        bail!("failed to inspect subscription checkout {checkout}");
    }
    Ok(Some(String::from_utf8(output.stdout)?.trim().to_string()))
}

fn rollback_checkout(name: &str, checkout: &Utf8Path, previous: Option<&str>) -> Result<()> {
    if let Some(previous) = previous {
        git(checkout, ["checkout", "--force", previous])
            .with_context(|| format!("failed to restore provider subscription `{name}`"))
    } else {
        let root = checkout
            .parent()
            .context("subscription checkout has no parent")?;
        if root.exists() {
            fs::remove_dir_all(root)
                .with_context(|| format!("failed to remove rejected provider checkout {root}"))?;
        }
        Ok(())
    }
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
