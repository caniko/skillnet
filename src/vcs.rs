use std::process::Command;

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct GitStatus {
    pub root: Utf8PathBuf,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub dirty_entries: usize,
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
    let dirty_entries = porcelain
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    Ok(Some(GitStatus {
        root: root.to_path_buf(),
        branch: non_empty(branch),
        remote: remote.and_then(non_empty),
        dirty_entries,
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
