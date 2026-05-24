#![allow(deprecated)]

use std::fs;

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    fs_ops,
    model::{Candidate, Choice, Source, Target},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteOptions {
    pub allow_older: bool,
    pub allow_delete: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteSummary {
    pub written: usize,
    pub modified: usize,
    pub removed: usize,
    pub skipped_older: usize,
    pub preserved_missing: usize,
}

pub fn discover_candidates(sources: &[Source]) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for source in sources {
        if !source.path.exists() {
            continue;
        }
        discover_regular(source, &mut candidates)?;
        discover_system(source, &mut candidates)?;
    }
    Ok(candidates)
}

pub fn choose_latest(candidates: &[Candidate]) -> Result<Vec<Choice>> {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(|a, b| a.skill.cmp(&b.skill));

    let mut choices = Vec::new();
    let mut idx = 0;
    while idx < sorted.len() {
        let skill = sorted[idx].skill.clone();
        let start = idx;
        while idx < sorted.len() && sorted[idx].skill == skill {
            idx += 1;
        }
        let group = &sorted[start..idx];
        let mut ranked = group.to_vec();
        ranked.sort_by(|a, b| {
            b.newest_mtime_nanos
                .cmp(&a.newest_mtime_nanos)
                .then_with(|| b.priority.cmp(&a.priority))
        });
        let winner = &ranked[0];
        let tied: Vec<_> = ranked
            .iter()
            .filter(|c| c.newest_mtime_nanos == winner.newest_mtime_nanos)
            .collect();
        if tied.len() > 1 {
            let first_sig = &tied[0].content_signature;
            if tied.iter().any(|c| &c.content_signature != first_sig) {
                let details = tied
                    .iter()
                    .map(|c| format!("  - {}: {} ({})", c.skill, c.source, c.path))
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!("ambiguous newest source for skill `{skill}`:\n{details}");
            }
        }
        choices.push(Choice {
            skill,
            source: winner.source.clone(),
            path: winner.path.clone(),
            newest_mtime_nanos: winner.newest_mtime_nanos,
            candidate_count: group.len(),
        });
    }

    choices.sort_by(|a, b| a.skill.cmp(&b.skill));
    Ok(choices)
}

#[allow(dead_code)]
pub fn reconcile_target(target: &Target, sync: bool, dry_run: bool) -> Result<Vec<Choice>> {
    let (choices, _) = reconcile_target_with_options(
        target,
        sync,
        dry_run,
        WriteOptions {
            allow_older: true,
            allow_delete: true,
        },
    )?;
    Ok(choices)
}

pub fn reconcile_target_with_options(
    target: &Target,
    sync: bool,
    dry_run: bool,
    options: WriteOptions,
) -> Result<(Vec<Choice>, WriteSummary)> {
    let _ = (target, sync, dry_run, options);
    todo!("phase 03/04 owns reconcile against Option B canonical paths")
}

#[allow(dead_code)]
pub fn sync_target(target: &Target) -> Result<()> {
    sync_target_with_options(
        target,
        WriteOptions {
            allow_older: true,
            allow_delete: true,
        },
    )
}

pub fn sync_target_with_options(target: &Target, options: WriteOptions) -> Result<()> {
    for view in &target.views {
        write_flat_from_mirror_with_options(&target.canonical_path, &view.path, options)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn write_flat_from_mirror(mirror: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    write_flat_from_mirror_with_options(
        mirror,
        dest,
        WriteOptions {
            allow_older: true,
            allow_delete: true,
        },
    )?;
    Ok(())
}

pub fn write_flat_from_mirror_with_options(
    mirror: &Utf8Path,
    dest: &Utf8Path,
    options: WriteOptions,
) -> Result<WriteSummary> {
    let incoming = mirror_skill_dirs(mirror)?
        .into_iter()
        .map(|path| {
            let skill = path
                .file_name()
                .context("mirror skill directory has no final component")?
                .to_string();
            Ok((skill, path))
        })
        .collect::<Result<Vec<_>>>()?;
    write_skill_set(&incoming, dest, options, None)
}

pub fn format_write_summary(summary: WriteSummary) -> String {
    let mut parts = Vec::new();
    if summary.skipped_older > 0 {
        parts.push(format!("skipped {} older", summary.skipped_older));
    }
    if summary.preserved_missing > 0 {
        parts.push(format!("preserved {} missing", summary.preserved_missing));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

pub fn mirror_skill_dirs(mirror: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    if !mirror.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(mirror)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in mirror: {}", p.display()))?;
        if path.is_dir() && path.join("SKILL.md").is_file() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn write_mirror(
    target: &Target,
    choices: &[Choice],
    options: WriteOptions,
) -> Result<WriteSummary> {
    let incoming = choices
        .iter()
        .map(|choice| (choice.skill.clone(), choice.path.clone()))
        .collect::<Vec<_>>();
    write_skill_set(
        &incoming,
        &target.canonical_path,
        options,
        Some((target, choices)),
    )
}

fn write_skill_set(
    incoming: &[(String, Utf8PathBuf)],
    dest: &Utf8Path,
    options: WriteOptions,
    manifest: Option<(&Target, &[Choice])>,
) -> Result<WriteSummary> {
    let staging = Utf8PathBuf::from(format!("{dest}.skillnet-tmp"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let existing = mirror_skill_dirs(dest)?
        .into_iter()
        .map(|path| {
            let skill = path
                .file_name()
                .context("skill directory has no final component")?
                .to_string();
            Ok((skill, path))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let incoming = incoming
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut summary = WriteSummary::default();

    for (skill, path) in &incoming {
        match existing.get(skill) {
            None => {
                fs_ops::copy_dir(path, &staging.join(skill))?;
                summary.written += 1;
            }
            Some(existing_path) => {
                let action = overwrite_action(path, existing_path, options.allow_older)?;
                match action {
                    OverwriteAction::Incoming => {
                        fs_ops::copy_dir(path, &staging.join(skill))?;
                        summary.modified += 1;
                    }
                    OverwriteAction::Existing => {
                        fs_ops::copy_dir(existing_path, &staging.join(skill))?;
                        if fs_ops::content_signature(path)?
                            != fs_ops::content_signature(existing_path)?
                        {
                            summary.skipped_older += 1;
                        }
                    }
                }
            }
        }
    }

    for (skill, existing_path) in &existing {
        if incoming.contains_key(skill) {
            continue;
        }
        if options.allow_delete {
            summary.removed += 1;
        } else {
            fs_ops::copy_dir(existing_path, &staging.join(skill))?;
            summary.preserved_missing += 1;
        }
    }

    if let Some((target, choices)) = manifest {
        write_manifest(target, choices, &staging)?;
    }
    fs_ops::replace_dir(&staging, dest)?;
    Ok(summary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverwriteAction {
    Incoming,
    Existing,
}

fn overwrite_action(
    incoming: &Utf8Path,
    existing: &Utf8Path,
    allow_older: bool,
) -> Result<OverwriteAction> {
    if allow_older {
        return Ok(OverwriteAction::Incoming);
    }
    let incoming_mtime = fs_ops::newest_mtime_nanos(incoming)?;
    let existing_mtime = fs_ops::newest_mtime_nanos(existing)?;
    if incoming_mtime > existing_mtime {
        return Ok(OverwriteAction::Incoming);
    }
    if incoming_mtime < existing_mtime {
        return Ok(OverwriteAction::Existing);
    }
    if fs_ops::content_signature(incoming)? == fs_ops::content_signature(existing)? {
        return Ok(OverwriteAction::Existing);
    }
    bail!(
        "equal-mtime conflicting skill content: incoming `{incoming}` and existing `{existing}` both have mtime {incoming_mtime}; pass --allow-older to overwrite"
    )
}

fn write_manifest(target: &Target, choices: &[Choice], output: &Utf8Path) -> Result<()> {
    let mut body = String::new();
    body.push_str("# Skill Reconciliation\n\n");
    body.push_str("Generated by `skillnet reconcile`.\n\n");
    body.push_str("## Rule\n\n");
    body.push_str("Generated outputs are flat sets of skill directories.\n\n");
    body.push_str("## Canonical\n\n");
    body.push_str(&format!("- `{}`\n", target.canonical_path));
    body.push_str("\n## Choices\n\n");
    body.push_str("| Skill | Selected Source | Selected Path | Newest Mtime | Candidates |\n");
    body.push_str("|---|---|---|---:|---:|\n");
    for choice in choices {
        body.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` | {} |\n",
            choice.skill,
            choice.source,
            choice.path,
            choice.newest_mtime_nanos,
            choice.candidate_count
        ));
    }
    fs::write(output.join("RECONCILIATION.md"), body).context("failed to write manifest")
}

fn discover_regular(source: &Source, candidates: &mut Vec<Candidate>) -> Result<()> {
    for entry in fs::read_dir(&source.path)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in source: {}", p.display()))?;
        if path.file_name() == Some(".system") {
            continue;
        }
        if path.is_dir() && path.join("SKILL.md").is_file() {
            candidates.push(candidate_from_path(source, &path, source.label.clone())?);
        }
    }
    Ok(())
}

fn discover_system(source: &Source, candidates: &mut Vec<Candidate>) -> Result<()> {
    let system_root = source.path.join(".system");
    if !system_root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(system_root)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in system source: {}", p.display()))?;
        if path.is_dir() && path.join("SKILL.md").is_file() {
            candidates.push(candidate_from_path(
                source,
                &path,
                format!("{}-system", source.label),
            )?);
        }
    }
    Ok(())
}

pub fn candidate_from_path(source: &Source, path: &Utf8Path, label: String) -> Result<Candidate> {
    Ok(Candidate {
        skill: path
            .file_name()
            .context("skill path has no final component")?
            .to_string(),
        source: label,
        priority: source.priority,
        path: path.to_path_buf(),
        newest_mtime_nanos: fs_ops::newest_mtime_nanos(path)?,
        content_signature: fs_ops::content_signature(path)?,
    })
}

fn print_choices(target: &Target, choices: &[Choice], sync: bool) {
    println!("# {} -> {}", target.name, target.canonical_path);
    if choices.is_empty() {
        println!("no skills found");
    }
    for choice in choices {
        println!(
            "{}\t{}\t{}\t{}",
            choice.skill, choice.source, choice.path, choice.candidate_count
        );
    }
    if sync {
        println!(
            "sync-back: {}",
            target
                .views
                .iter()
                .map(|view| view.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap, fs, io::Read, os::unix::fs as unix_fs, thread, time::Duration,
    };

    use sha2::{Digest, Sha256};
    use tempfile::tempdir;
    use walkdir::WalkDir;

    use super::*;

    fn skill(root: &Utf8Path, name: &str, body: &str) -> Utf8PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
        dir
    }

    fn tree_entries(root: &Utf8Path) -> BTreeMap<String, String> {
        let mut entries = BTreeMap::new();
        for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
            let entry = entry.unwrap();
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            if !(metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
                continue;
            }
            let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf()).unwrap();
            let rel = path.strip_prefix(root).unwrap().to_string();
            entries.insert(rel, entry_hash(&path));
        }
        entries
    }

    fn entry_hash(path: &Utf8Path) -> String {
        let metadata = fs::symlink_metadata(path).unwrap();
        let mut hasher = Sha256::new();
        if metadata.file_type().is_symlink() {
            hasher.update(b"symlink");
            hasher.update(fs::read_link(path).unwrap().to_string_lossy().as_bytes());
        } else {
            hasher.update(b"file");
            let mut file = fs::File::open(path).unwrap();
            let mut buf = [0; 8192];
            loop {
                let n = file.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn write_flat_from_mirror_writes_identical_trees_to_every_destination() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let mirror = root.join("mirror");
        let alpha = skill(&mirror, "alpha", "alpha");
        fs::create_dir_all(alpha.join("examples")).unwrap();
        fs::write(alpha.join("examples/example.md"), "example").unwrap();
        unix_fs::symlink("SKILL.md", alpha.join("skill-link.md")).unwrap();
        fs::write(mirror.join("RECONCILIATION.md"), "manifest").unwrap();
        let first = root.join("first");
        let second = root.join("second");

        write_flat_from_mirror(&mirror, &first).unwrap();
        write_flat_from_mirror(&mirror, &second).unwrap();

        assert_eq!(tree_entries(&first), tree_entries(&second));
        assert!(!first.join("RECONCILIATION.md").exists());
        assert!(!second.join("RECONCILIATION.md").exists());
    }

    #[test]
    fn newest_candidate_wins() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let older = root.join("older");
        let newer = root.join("newer");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&newer).unwrap();
        skill(&older, "x", "old");
        thread::sleep(Duration::from_millis(5));
        skill(&newer, "x", "new");
        let candidates = discover_candidates(&[
            Source {
                label: "older".into(),
                path: older,
                priority: 2,
            },
            Source {
                label: "newer".into(),
                path: newer,
                priority: 1,
            },
        ])
        .unwrap();
        let choices = choose_latest(&candidates).unwrap();
        assert_eq!(choices[0].source, "newer");
    }

    #[test]
    fn identical_ties_are_allowed() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let a = skill(&root, "a", "same");
        let b = skill(&root, "b", "same");
        let source = Source {
            label: "s".into(),
            path: root,
            priority: 1,
        };
        let mut c1 = candidate_from_path(&source, &a, "a".into()).unwrap();
        let mut c2 = candidate_from_path(&source, &b, "b".into()).unwrap();
        c1.skill = "x".into();
        c2.skill = "x".into();
        c2.newest_mtime_nanos = c1.newest_mtime_nanos;
        choose_latest(&[c1, c2]).unwrap();
    }

    #[test]
    fn conflicting_ties_fail() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let a = skill(&root, "a", "one");
        let b = skill(&root, "b", "two");
        let source = Source {
            label: "s".into(),
            path: root,
            priority: 1,
        };
        let mut c1 = candidate_from_path(&source, &a, "a".into()).unwrap();
        let mut c2 = candidate_from_path(&source, &b, "b".into()).unwrap();
        c1.skill = "x".into();
        c2.skill = "x".into();
        c2.newest_mtime_nanos = c1.newest_mtime_nanos;
        assert!(choose_latest(&[c1, c2]).is_err());
    }
}
