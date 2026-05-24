use std::process::Command;

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirtyKind {
    Modified,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirtyEntry {
    pub kind: DirtyKind,
    pub paths: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct GitStatus {
    pub root: Utf8PathBuf,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub dirty_entries: usize,
    pub dirty: Vec<DirtyEntry>,
}

impl GitStatus {
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty_entries > 0
    }
}

pub(crate) fn status(root: &Utf8Path) -> Result<Option<GitStatus>> {
    if !root.join(".git").exists() {
        return Ok(None);
    }

    let branch = git_output(root, ["branch", "--show-current"])?;
    let remote = git_output(root, ["remote", "get-url", "origin"]).ok();
    let porcelain = git_output(root, ["status", "--porcelain"])?;
    let dirty = parse_porcelain(&porcelain)?;
    let dirty_entries = dirty.len();

    Ok(Some(GitStatus {
        root: root.to_path_buf(),
        branch: non_empty(branch),
        remote: remote.and_then(non_empty),
        dirty_entries,
        dirty,
    }))
}

pub(crate) fn ensure_clean(root: &Utf8Path) -> Result<()> {
    let Some(status) = status(root)? else {
        return Ok(());
    };

    if status.is_dirty() {
        bail!(
            "destination repository `{}` has {} dirty entries; commit, stash, or pass --allow-dirty-destination",
            status.root,
            status.dirty_entries
        );
    }
    Ok(())
}

pub(crate) fn head_commit(root: &Utf8Path) -> Result<Option<String>> {
    match git_output(root, ["rev-parse", "--verify", "HEAD"]) {
        Ok(head) => Ok(non_empty(head)),
        Err(error) if is_missing_head_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn git_output<const N: usize>(root: &Utf8Path, args: [&str; N]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run git in {root}"))?;

    if !output.status.success() {
        bail!(
            "git command failed in `{}`: {}",
            root,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn parse_porcelain(porcelain: &str) -> Result<Vec<DirtyEntry>> {
    porcelain
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_porcelain_line)
        .collect()
}

fn parse_porcelain_line(line: &str) -> Result<DirtyEntry> {
    if line.len() < 4 {
        bail!("unsupported git status line: {line}");
    }

    let code = &line[..2];
    let raw_path = &line[3..];
    let paths = if matches!(code.chars().next(), Some('R' | 'C')) {
        raw_path
            .split(" -> ")
            .map(parse_dirty_path)
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![parse_dirty_path(raw_path)?]
    };

    Ok(DirtyEntry {
        kind: if code == "??" {
            DirtyKind::Untracked
        } else {
            DirtyKind::Modified
        },
        paths,
    })
}

fn parse_dirty_path(raw: &str) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(std::path::PathBuf::from(raw))
        .map_err(|path| anyhow::anyhow!("non-UTF-8 git status path: {}", path.display()))
}

fn is_missing_head_error(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("Needed a single revision") || text.contains("unknown revision or path")
}
