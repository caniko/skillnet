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
    io::{self, Read, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::Context;
use crate::{
    cache::{self, Cache, ScopeStamp},
    cli::{args::StatusFormat, Scope},
    codex::{self, AutoCommitRequest},
    config::SyncOverrides,
    exit::ExitError,
    fs_ops, reconcile,
    reconcile::WriteOptions,
};

#[derive(Debug, Clone, Default)]
pub struct PullOptions {
    pub then_push: bool,
    pub sync_overrides: SyncOverrides,
    pub write_options: WriteOptions,
}

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DestinationDelta {
    pub written: usize,
    pub modified: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LiveHash {
    Single(String),
    Conflict,
}

pub fn pull(ctx: &Context, scopes: &[Scope], options: PullOptions) -> Result<()> {
    if options.then_push {
        eprintln!("warning: `sync pull --then-push` is deprecated; use `sync roundtrip` instead");
    }
    pull_only(ctx, scopes, &options.sync_overrides, options.write_options)
        .map_err(ExitError::pull)?;

    if options.then_push {
        push(ctx, scopes, options.write_options).map_err(ExitError::push)?;
    }

    Ok(())
}

pub fn roundtrip(
    ctx: &Context,
    scopes: &[Scope],
    check: bool,
    write_options: WriteOptions,
) -> Result<()> {
    if check {
        return roundtrip_check(ctx, scopes, write_options);
    }

    pull_only(ctx, scopes, &SyncOverrides::default(), write_options).map_err(ExitError::pull)?;
    push(ctx, scopes, write_options).map_err(ExitError::push)?;
    Ok(())
}

fn pull_only(
    ctx: &Context,
    scopes: &[Scope],
    sync_overrides: &SyncOverrides,
    write_options: WriteOptions,
) -> Result<()> {
    ensure_destination_ready_for_pull(ctx, scopes, sync_overrides)?;
    let mut cache = cache::load(&ctx.mirror_root);

    for target in ctx.targets(scopes)? {
        reconcile::reconcile_target_with_options(&target, false, ctx.dry_run, write_options)?;
        if ctx.dry_run {
            continue;
        }

        let mirror_content_hash = mirror_content_hash(&target.canonical_path)?;
        let live_source_max_mtime_nanos = 0;
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

    Ok(())
}

pub fn push(ctx: &Context, scopes: &[Scope], write_options: WriteOptions) -> Result<()> {
    for target in ctx.targets(scopes)? {
        if ctx.dry_run {
            println!("# sync {}", target.name);
            println!("from: {}", target.canonical_path);
            println!(
                "to: {}",
                target
                    .views
                    .iter()
                    .map(|view| view.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            continue;
        }
        let mut summaries = Vec::with_capacity(target.views.len());
        for view in &target.views {
            let before = mirror_files(&view.path)?;
            let summary = reconcile::write_flat_from_mirror_with_options(
                &target.canonical_path,
                &view.path,
                write_options,
            )?;
            let after = mirror_files(&view.path)?;
            summaries.push((
                view.path.clone(),
                destination_delta(&before, &after),
                summary,
            ));
        }
        println!("{}", format_push_summary(&target.name, &summaries));
    }
    Ok(())
}

pub fn status(ctx: &Context, scopes: &[Scope], format: StatusFormat) -> Result<()> {
    let summaries = status_summaries(ctx, scopes)?;
    match format {
        StatusFormat::Text => {
            for summary in &summaries {
                print_summary(summary);
            }
        }
        StatusFormat::Json => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            print_summary_json(&summaries, &mut handle)?;
            writeln!(handle)?;
        }
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

fn roundtrip_check(ctx: &Context, scopes: &[Scope], write_options: WriteOptions) -> Result<()> {
    ctx.ensure_destination_clean()?;
    let temp = tempfile::tempdir()?;
    let temp_root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
        .map_err(|path| anyhow::anyhow!("non-UTF-8 temporary path: {}", path.display()))?;
    let mut changed = false;

    for mut target in ctx.targets(scopes)? {
        let real_mirror_path = target.canonical_path.clone();
        target.canonical_path = temp_root.join(&target.name);
        if real_mirror_path.exists() {
            fs_ops::copy_dir(&real_mirror_path, &target.canonical_path)?;
        }
        reconcile::reconcile_target_with_options(&target, false, false, write_options)
            .map_err(ExitError::pull)?;

        for view in &target.views {
            let destination = &view.path;
            let temp_destination = temp_root.join(format!(
                "{}-{}",
                target.name,
                destination.as_str().replace('/', "__")
            ));
            if destination.exists() {
                fs_ops::copy_dir(destination, &temp_destination)?;
            }
            reconcile::write_flat_from_mirror_with_options(
                &target.canonical_path,
                &temp_destination,
                write_options,
            )?;
            let deltas = diff_mirror_to_destination(&temp_destination, destination)?;
            print_destination_check(destination, &deltas);
            changed |= !deltas.is_empty();
        }
    }

    if changed {
        return Err(ExitError::check_drift(
            "`sync roundtrip --check` found destinations that would change",
        )
        .into());
    }

    Ok(())
}

fn ensure_destination_ready_for_pull(
    ctx: &Context,
    scopes: &[Scope],
    sync_overrides: &SyncOverrides,
) -> Result<()> {
    if ctx.dry_run || ctx.allow_dirty_destination {
        return Ok(());
    }

    let Some(status) = crate::vcs::status(&ctx.mirror_root)? else {
        return Ok(());
    };
    if !status.is_dirty() {
        return Ok(());
    }

    let sync_config = ctx.resolve_sync_config(sync_overrides);
    if !sync_config.auto_commit_dirty_destination {
        crate::vcs::ensure_clean(&ctx.mirror_root)?;
        return Ok(());
    }

    let allowed_prefixes = allowed_dirty_prefixes(ctx, scopes)?;
    let blocked = blocked_dirty_paths(&status.dirty, &allowed_prefixes);
    if !blocked.is_empty() {
        anyhow::bail!(
            "destination repository `{}` has dirty entries outside the selected skillnet-managed paths:\n{}",
            status.root,
            blocked
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let before_head = crate::vcs::head_commit(&ctx.mirror_root)?;
    let dirty_paths = status
        .dirty
        .iter()
        .flat_map(|entry| entry.paths.iter().cloned())
        .collect::<Vec<_>>();

    codex::auto_commit(
        &ctx.mirror_root,
        &AutoCommitRequest {
            model: sync_config.codex_model,
            reasoning_effort: sync_config.codex_reasoning_effort,
            dirty_paths,
        },
    )?;

    let after_head = crate::vcs::head_commit(&ctx.mirror_root)?;
    if before_head == after_head {
        anyhow::bail!(
            "Codex auto-commit reported success but did not create a new commit in `{}`",
            ctx.mirror_root
        );
    }

    if let Some(status) = crate::vcs::status(&ctx.mirror_root)? {
        if status.is_dirty() {
            let remaining = status
                .dirty
                .iter()
                .flat_map(|entry| entry.paths.iter())
                .map(|path| format!("- {}", path))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "Codex auto-commit completed but destination repository `{}` is still dirty:\n{}",
                status.root,
                remaining
            );
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
        let live_mtime = 0;
        let cache_state = match stamp {
            None => CacheState::Missing,
            Some(stamp) if cache::is_stale(stamp, live_mtime) => CacheState::Stale,
            Some(_) => CacheState::Fresh,
        };

        let state = match (stamp, cache_state) {
            (Some(stamp), CacheState::Fresh)
                if mirror_content_hash(&target.canonical_path)? == stamp.mirror_content_hash =>
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

pub(crate) fn print_summary_json(
    summaries: &[ScopeSummary],
    writer: &mut impl Write,
) -> Result<()> {
    serde_json::to_writer_pretty(
        writer,
        &StatusJson {
            schema: "skillnet.status.v1",
            scopes: summaries.iter().map(StatusScopeJson::from).collect(),
        },
    )
    .context("failed to serialize status JSON")
}

#[derive(Serialize)]
struct StatusJson<'a> {
    schema: &'a str,
    scopes: Vec<StatusScopeJson>,
}

#[derive(Serialize)]
struct StatusScopeJson {
    name: String,
    state: &'static str,
    diverged_files: usize,
    last_pulled_at: Option<String>,
    cache_state: &'static str,
}

impl From<&ScopeSummary> for StatusScopeJson {
    fn from(summary: &ScopeSummary) -> Self {
        let (state, diverged_files) = match summary.state {
            ScopeState::Clean => ("clean", 0),
            ScopeState::Diverged(count) => ("diverged", count),
        };
        let cache_state = match summary.cache_state {
            CacheState::Fresh => "fresh",
            CacheState::Stale => "stale",
            CacheState::Missing => "missing",
        };

        Self {
            name: summary.scope.to_string(),
            state,
            diverged_files,
            last_pulled_at: summary.last_pulled_at.map(rfc3339_utc),
            cache_state,
        }
    }
}

fn rfc3339_utc(time: SystemTime) -> String {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    let secs = duration.as_secs();
    let days = (secs / 86_400) as i64;
    let seconds_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);

    (year, month as u32, day as u32)
}

fn diff_target(target: &crate::model::Target) -> Result<Vec<Delta>> {
    let mirror_files = mirror_files(&target.canonical_path)?;
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

fn allowed_dirty_prefixes(ctx: &Context, scopes: &[Scope]) -> Result<Vec<Utf8PathBuf>> {
    let targets = ctx.targets(scopes)?;
    let mut prefixes = Vec::new();
    for target in targets {
        prefixes.push(path_relative_to_mirror_root(ctx, &target.canonical_path)?);
        for view in &target.views {
            push_if_relative_to_mirror_root(ctx, &view.path, &mut prefixes);
        }
    }
    push_if_relative_to_mirror_root(ctx, &ctx.config_path, &mut prefixes);
    push_if_relative_to_mirror_root(ctx, &ctx.catalog_config_path, &mut prefixes);
    prefixes.push(Utf8PathBuf::from(".skillnet/cache.toml"));
    Ok(prefixes)
}

fn path_relative_to_mirror_root(ctx: &Context, path: &Utf8Path) -> Result<Utf8PathBuf> {
    path.strip_prefix(&ctx.mirror_root)
        .map(|path| path.to_path_buf())
        .with_context(|| {
            format!(
                "target mirror path `{}` does not live under mirror root `{}`",
                path, ctx.mirror_root
            )
        })
}

fn push_if_relative_to_mirror_root(
    ctx: &Context,
    path: &Utf8Path,
    prefixes: &mut Vec<Utf8PathBuf>,
) {
    if let Ok(relative) = path.strip_prefix(&ctx.mirror_root) {
        prefixes.push(relative.to_path_buf());
    }
}

fn blocked_dirty_paths(
    dirty_entries: &[crate::vcs::DirtyEntry],
    allowed_prefixes: &[Utf8PathBuf],
) -> Vec<String> {
    dirty_entries
        .iter()
        .flat_map(|entry| {
            entry
                .paths
                .iter()
                .filter(|path| !dirty_path_allowed(entry.kind, path, allowed_prefixes))
        })
        .map(|path| path.as_str().to_string())
        .collect()
}

fn dirty_path_allowed(
    kind: crate::vcs::DirtyKind,
    path: &Utf8Path,
    allowed_prefixes: &[Utf8PathBuf],
) -> bool {
    allowed_prefixes.iter().any(|prefix| {
        path.starts_with(prefix)
            || (kind == crate::vcs::DirtyKind::Untracked && prefix.starts_with(path))
    })
}

fn diff_mirror_to_destination(mirror: &Utf8Path, destination: &Utf8Path) -> Result<Vec<Delta>> {
    let expected_files = mirror_files(mirror)?;
    let destination_files = mirror_files(destination)?;
    diff_file_maps(&expected_files, &destination_files)
}

fn diff_empty_to_destination(destination: &Utf8Path) -> Result<Vec<Delta>> {
    let empty = BTreeMap::new();
    let destination_files = mirror_files(destination)?;
    diff_file_maps(&empty, &destination_files)
}

fn diff_file_maps(
    mirror_files: &BTreeMap<String, String>,
    destination_files: &BTreeMap<String, String>,
) -> Result<Vec<Delta>> {
    let paths = mirror_files
        .keys()
        .chain(destination_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut deltas = Vec::new();
    for path in paths {
        match (mirror_files.get(&path), destination_files.get(&path)) {
            (None, Some(_)) => deltas.push(Delta {
                kind: DeltaKind::OnlyLive,
                path,
            }),
            (Some(_), None) => deltas.push(Delta {
                kind: DeltaKind::OnlyMirror,
                path,
            }),
            (Some(mirror), Some(destination)) if mirror != destination => deltas.push(Delta {
                kind: DeltaKind::Modified,
                path,
            }),
            _ => {}
        }
    }

    Ok(deltas)
}

fn destination_delta(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> DestinationDelta {
    let paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut delta = DestinationDelta::default();

    for path in paths {
        match (before.get(&path), after.get(&path)) {
            (None, Some(_)) => delta.written += 1,
            (Some(_), None) => delta.removed += 1,
            (Some(before), Some(after)) if before != after => delta.modified += 1,
            _ => {}
        }
    }

    delta
}

fn format_push_summary(
    scope: &str,
    destinations: &[(Utf8PathBuf, DestinationDelta, reconcile::WriteSummary)],
) -> String {
    let destinations = destinations
        .iter()
        .map(|(path, delta, summary)| {
            format!(
                "{} (+{} ~{} -{}){}",
                path,
                delta.written,
                delta.modified,
                delta.removed,
                reconcile::format_write_summary(*summary)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("synced {scope} → {destinations}")
}

fn print_destination_check(destination: &Utf8Path, deltas: &[Delta]) {
    if deltas.is_empty() {
        println!("{}  clean", destination);
    } else {
        println!("{}  would change ({} files)", destination, deltas.len());
    }
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
    let _ = target;
    todo!("phase 03 owns Option B live view diffing")
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
