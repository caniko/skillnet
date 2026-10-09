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

    let root = checkout
        .parent()
        .context("subscription checkout has no parent")?;
    let revisions = root.join("revisions");
    fs::create_dir_all(&revisions)?;
    let staged = tempfile::Builder::new()
        .prefix("revision-")
        .tempdir_in(&revisions)?;
    let staged_path =
        Utf8Path::from_path(staged.path()).context("provider revision path is not UTF-8")?;
    let validation: Result<()> = (|| {
        update_checkout(name, subscription, staged_path)?;
        let source = subscription_source_path(staged_path, name, subscription)?
            .context("updated provider source is missing")?;
        let target = ctx.config.global_target(&ctx.mirror_root)?;
        let _ = ctx.bundle_plan_with_provider(&target, Some((name, &source)))?;
        Ok(())
    })();
    validation.with_context(|| {
        format!("provider subscription `{name}` update was rejected; retained last-known-good checkout")
    })?;
    let current = root.join("current");
    let previous = match fs::read_link(&current) {
        Ok(previous) => Some(previous),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("read provider pointer {current}")),
    };
    // ponytail: retain published revisions so interrupted bundle writes cannot dangle;
    // reclaim them only with a future collector that traces all live resource links.
    let published = canonical_utf8(staged_path)?;
    let _ = staged.keep();
    crate::view::atomic_symlink(&published, &current)?;
    if let Err(error) = super::view::sync(ctx, true, false, None) {
        let original = format!("{error:#}");
        let rollback = if let Some(previous) = previous {
            Utf8PathBuf::from_path_buf(previous)
                .map_err(|path| anyhow::anyhow!("provider pointer is not UTF-8: {}", path.display()))
                .and_then(|previous| crate::view::atomic_symlink(&previous, &current))
        } else {
            fs::remove_file(&current).map_err(anyhow::Error::from)
        };
        if let Err(rollback) = rollback {
            bail!("provider subscription `{name}` update failed: {original}; rollback also failed: {rollback:#}");
        }
        if let Err(restore) = super::view::sync(ctx, true, false, None) {
            bail!("provider subscription `{name}` update failed: {original}; checkout pointer was restored but view restoration failed: {restore:#}");
        }
        return Err(anyhow::anyhow!(original)).with_context(|| {
            format!("provider subscription `{name}` update was rejected; retained last-known-good checkout")
        });
    }
    println!("synced provider subscription {name}");
    Ok(())
}

fn reject_canonical_target(ctx: &Context, name: &str, target: &Utf8Path) -> Result<()> {
    reject_canonical_overlap(ctx, name, "target", target)
}

pub(crate) fn validate_provider_storage(ctx: &Context, name: &str) -> Result<()> {
    reject_canonical_overlap(ctx, name, "checkout", &checkout_path(&ctx.data_dir, name))?;
    let root = ctx.data_dir.join("subscriptions").join(name);
    reject_canonical_overlap(ctx, name, "revisions", &root.join("revisions"))?;
    reject_canonical_overlap(ctx, name, "current", &root.join("current"))?;
    let bundle_root = match ctx.config.bundles_root.as_deref() {
        Some(root) => expand_path(root)?,
        None => ctx.data_dir.join("bundles"),
    };
    reject_canonical_overlap(ctx, name, "bundle", &bundle_root.join("global"))
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
    let legacy = checkout_path(data_dir, name);
    let current = legacy
        .parent()
        .context("subscription checkout has no parent")?
        .join("current");
    let checkout = if subscription.provider && fs::symlink_metadata(&current).is_ok() {
        current
    } else {
        legacy
    };
    subscription_source_path(&checkout, name, subscription)
}

fn subscription_source_path(
    checkout: &Utf8Path,
    name: &str,
    subscription: &SubscriptionConfig,
) -> Result<Option<Utf8PathBuf>> {
    let source = checkout.join(&subscription.source);
    if !source.exists() {
        return Ok(None);
    }
    let checkout = canonical_utf8(checkout)?;
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
