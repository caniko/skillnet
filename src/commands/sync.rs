//! Sync orchestration and divergence reporting.
//!
//! A scope is clean when the flattened mirror file set and the flattened live
//! source file set contain the same relative file paths with matching content
//! hashes. A scope is diverged when a file exists only in the mirror, only in
//! live sources, has different content, or live sources disagree with each
//! other for the same flattened path.
//!
//! Cache writes are pull-only. `status` and `diff` are observation commands:
//! missing or corrupt cache data falls back to a one-shot walk, and status never
//! writes refreshed cache entries. During pull, the mirror hash is computed
//! before the live source mtime stamp is recorded so the cache represents the
//! mirror state as of that live-source mtime threshold.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    time::SystemTime,
};

use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::Context;
use crate::{
    cache::{self, Cache, ScopeStamp},
    cli::Scope,
    fs_ops, reconcile,
};

#[derive(Debug, Clone)]
pub(crate) struct ScopeSummary {
    pub scope: Scope,
    pub state: ScopeState,
    pub last_pulled_at: Option<SystemTime>,
    pub cache_state: CacheState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeState {
    Clean,
    Diverged(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheState {
    Fresh,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeltaKind {
    OnlyLive,
    OnlyMirror,
    Modified,
}

#[derive(Debug, Clone)]
struct Delta {
    kind: DeltaKind,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveHash {
    Single(String),
    Conflict,
}

pub fn pull(ctx: &Context, scopes: &[Scope], then_push: bool) -> Result<()> {
    ctx.ensure_destination_clean()?;
    let mut cache = cache::load(&ctx.mirror_root);

    for target in ctx.targets(scopes)? {
        reconcile::reconcile_target(&target, false, ctx.dry_run)?;
        if ctx.dry_run {
            continue;
        }

        let mirror_content_hash = mirror_content_hash(&target.mirror_path)?;
        let live_source_max_mtime_nanos = live_source_max_mtime(&target.sources)?;
        cache.stamps.insert(
            target.name,
            ScopeStamp {
                last_pulled_at: SystemTime::now(),
                live_source_max_mtime_nanos,
                mirror_content_hash,
            },
        );
    }

    if !ctx.dry_run {
        cache::save(&ctx.mirror_root, &cache)?;
    }

    if then_push {
        push(ctx, scopes)?;
    }

    Ok(())
}

pub fn push(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        if ctx.dry_run {
            println!("# sync {}", target.name);
            println!("from: {}", target.mirror_path);
            println!(
                "to: {}",
                target
                    .sync_paths
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            continue;
        }
        reconcile::sync_target(&target)?;
        println!("synced {}", target.name);
    }
    Ok(())
}

pub fn status(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for summary in status_summaries(ctx, scopes)? {
        print_summary(&summary);
    }
    Ok(())
}

pub fn diff(ctx: &Context, scopes: &[Scope]) -> Result<()> {
    for target in ctx.targets(scopes)? {
        let deltas = diff_target(&target)?;
        println!("# {}", target.name);
        if deltas.is_empty() {
            println!("clean");
            continue;
        }
        for delta in deltas {
            let marker = match delta.kind {
                DeltaKind::OnlyLive => '+',
                DeltaKind::OnlyMirror => '-',
                DeltaKind::Modified => '~',
            };
            println!("{marker} {}", delta.path);
        }
    }
    Ok(())
}

pub(crate) fn status_summaries(ctx: &Context, scopes: &[Scope]) -> Result<Vec<ScopeSummary>> {
    let cache = cache::load(&ctx.mirror_root);
    let targets = ctx.targets(scopes)?;
    let mut summaries = Vec::with_capacity(scopes.len());

    for (scope, target) in scopes.iter().zip(targets) {
        let stamp = cache.stamps.get(&scope.to_string());
        let live_mtime = live_source_max_mtime(&target.sources)?;
        let cache_state = match stamp {
            None => CacheState::Missing,
            Some(stamp) if cache::is_stale(stamp, live_mtime) => CacheState::Stale,
            Some(_) => CacheState::Fresh,
        };

        let state = match (stamp, cache_state) {
            (Some(stamp), CacheState::Fresh)
                if mirror_content_hash(&target.mirror_path)? == stamp.mirror_content_hash =>
            {
                ScopeState::Clean
            }
            _ => {
                let delta_count = diff_target(&target)?.len();
                if delta_count == 0 {
                    ScopeState::Clean
                } else {
                    ScopeState::Diverged(delta_count)
                }
            }
        };

        summaries.push(ScopeSummary {
            scope: scope.clone(),
            state,
            last_pulled_at: stamp.map(|stamp| stamp.last_pulled_at),
            cache_state,
        });
    }

    Ok(summaries)
}

pub(crate) fn cache_metadata(ctx: &Context) -> (Utf8PathBuf, Cache, Option<SystemTime>) {
    let path = cache::cache_path(&ctx.mirror_root);
    let cache = cache::load(&ctx.mirror_root);
    let modified = fs::metadata(&path)
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    (path, cache, modified)
}

pub(crate) fn relative_time(time: Option<SystemTime>) -> String {
    let Some(time) = time else {
        return "never".to_string();
    };

    match time.elapsed() {
        Ok(elapsed) => {
            let secs = elapsed.as_secs();
            if secs < 60 {
                format!("{secs}s ago")
            } else if secs < 60 * 60 {
                format!("{}m ago", secs / 60)
            } else if secs < 60 * 60 * 24 {
                format!("{}h ago", secs / (60 * 60))
            } else {
                format!("{}d ago", secs / (60 * 60 * 24))
            }
        }
        Err(_) => "in the future".to_string(),
    }
}

fn print_summary(summary: &ScopeSummary) {
    let state = match summary.state {
        ScopeState::Clean => "clean".to_string(),
        ScopeState::Diverged(count) => format!("diverged ({count} files)"),
    };
    let cache = match summary.cache_state {
        CacheState::Fresh => "cache fresh",
        CacheState::Stale => "cache stale",
        CacheState::Missing => "no cache",
    };
    println!(
        "{}  {}  last-pulled {}  {}",
        summary.scope,
        state,
        relative_time(summary.last_pulled_at),
        cache
    );
}

fn diff_target(target: &crate::model::Target) -> Result<Vec<Delta>> {
    let mirror_files = mirror_files(&target.mirror_path)?;
    let live_files = live_files(target)?;
    let paths = mirror_files
        .keys()
        .chain(live_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut deltas = Vec::new();
    for path in paths {
        match (mirror_files.get(&path), live_files.get(&path)) {
            (None, Some(_)) => deltas.push(Delta {
                kind: DeltaKind::OnlyLive,
                path,
            }),
            (Some(_), None) => deltas.push(Delta {
                kind: DeltaKind::OnlyMirror,
                path,
            }),
            (Some(_), Some(LiveHash::Conflict)) => deltas.push(Delta {
                kind: DeltaKind::Modified,
                path,
            }),
            (Some(mirror), Some(LiveHash::Single(live))) if mirror != live => deltas.push(Delta {
                kind: DeltaKind::Modified,
                path,
            }),
            _ => {}
        }
    }

    Ok(deltas)
}

fn mirror_files(mirror_root: &Utf8Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for skill_dir in reconcile::mirror_skill_dirs(mirror_root)? {
        let skill = skill_dir
            .file_name()
            .context("mirror skill directory has no final component")?
            .to_string();
        collect_skill_files(&skill_dir, &skill, &mut files)?;
    }
    Ok(files)
}

fn live_files(target: &crate::model::Target) -> Result<BTreeMap<String, LiveHash>> {
    let mut files = BTreeMap::new();
    for candidate in reconcile::discover_candidates(&target.sources)? {
        let mut candidate_files = BTreeMap::new();
        collect_skill_files(&candidate.path, &candidate.skill, &mut candidate_files)?;
        for (path, hash) in candidate_files {
            files
                .entry(path)
                .and_modify(|existing| match existing {
                    LiveHash::Single(existing_hash) if existing_hash == &hash => {}
                    _ => *existing = LiveHash::Conflict,
                })
                .or_insert(LiveHash::Single(hash));
        }
    }
    Ok(files)
}

fn collect_skill_files(
    skill_dir: &Utf8Path,
    skill: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    for entry in WalkDir::new(skill_dir).follow_links(false).min_depth(1) {
        let entry = entry?;
        if !(entry.file_type().is_file() || entry.file_type().is_symlink()) {
            continue;
        }

        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        let rel = path.strip_prefix(skill_dir)?;
        files.insert(format!("{skill}/{rel}"), file_hash(&path)?);
    }
    Ok(())
}

fn file_hash(path: &Utf8Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    let mut hasher = Sha256::new();
    if metadata.file_type().is_symlink() {
        hasher.update(b"symlink");
        hasher.update(fs::read_link(path)?.to_string_lossy().as_bytes());
    } else {
        hasher.update(b"file");
        let mut file = fs::File::open(path)?;
        let mut buf = [0; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn mirror_content_hash(mirror_path: &Utf8Path) -> Result<String> {
    if mirror_path.exists() {
        fs_ops::content_signature(mirror_path)
    } else {
        Ok(String::new())
    }
}

fn live_source_max_mtime(sources: &[crate::model::Source]) -> Result<u128> {
    cache::live_source_max_mtime(
        &sources
            .iter()
            .map(|source| source.path.clone())
            .collect::<Vec<_>>(),
    )
}
