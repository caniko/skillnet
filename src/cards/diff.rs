use std::process::Command;

use anyhow::{ensure, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) struct FileDiff {
    pub file: Utf8PathBuf,
    pub added: String,
}

pub(crate) fn git_diff(cwd: &Utf8Path, base: &str) -> Result<Vec<FileDiff>> {
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "diff",
            "--raw",
            "-z",
            "--patch",
            "--unified=0",
            "--inter-hunk-context=0",
            "--no-relative",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--submodule=short",
            "--output-indicator-new=+",
            "--end-of-options",
            base,
            "--",
        ])
        .current_dir(cwd)
        .output()
        .with_context(|| format!("run git diff in {cwd}"))?;
    ensure!(
        output.status.success(),
        "git diff against {base:?} failed in {cwd}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    parse(&output.stdout).context("read git diff")
}

fn parse(mut raw: &[u8]) -> Result<Vec<FileDiff>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    // Git emits NUL-delimited raw records in patch order, then a NUL and the patch.
    // Use the raw paths instead of splitting quoted `diff --git` filenames on spaces.
    let mut paths = Vec::new();
    let patch = loop {
        let header = field(&mut raw)?;
        if header.is_empty() {
            break String::from_utf8_lossy(raw);
        }
        let status = header
            .rsplit(|&byte| byte == b' ')
            .next()
            .unwrap_or_default();
        let mut path = field(&mut raw)?;
        if matches!(status.first(), Some(b'R' | b'C')) {
            path = field(&mut raw)?;
        }
        paths.push((
            Utf8PathBuf::from(std::str::from_utf8(path)?),
            status == b"T",
        ));
    };
    ensure!(patch.starts_with("diff --git "), "missing Git patch");
    let mut sections = patch.split("\ndiff --git ");
    let mut diffs = Vec::new();
    for (file, typechange) in paths {
        if typechange {
            // Git represents a type change as a deletion followed by an addition.
            sections.next().context("missing Git typechange deletion")?;
        }
        let section = sections.next().context("missing Git file patch")?;
        let mut added = String::new();
        let mut in_hunk = false;
        for line in section.lines() {
            if line.starts_with("@@ ") {
                in_hunk = true;
                if !added.is_empty() {
                    added.push('\n');
                }
            } else if in_hunk && line.starts_with('+') {
                added.push_str(line);
                added.push('\n');
            }
        }
        diffs.push(FileDiff { file, added });
    }
    ensure!(sections.next().is_none(), "unexpected extra Git file patch");
    Ok(diffs)
}

fn field<'a>(raw: &mut &'a [u8]) -> Result<&'a [u8]> {
    let end = raw
        .iter()
        .position(|&byte| byte == 0)
        .context("unterminated Git diff field")?;
    let (value, rest) = raw.split_at(end);
    *raw = &rest[1..];
    Ok(value)
}

#[test]
fn handles_typechanges_deleted_lines_renames_and_patch_like_added_lines() {
    let raw = b":100644 120000 old new T\0a.rs\0:100644 100644 old new R100\0b.rs\0c\t.rs\0\
        :100644 100644 old new M\0d.nix\0\0\
        diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-removed\n\
        diff --git a/a.rs b/a.rs\n--- /dev/null\n+++ b/a.rs\n@@ -0,0 +1 @@\n+link\n\
        diff --git a/b.rs \"b/c\\t.rs\"\nrename from b.rs\nrename to \"c\\t.rs\"\n\
        diff --git a/d.nix b/d.nix\n--- a/d.nix\n+++ b/d.nix\n@@ -2 +2 @@\n-old\n+++ data\n\
        @@ -5 +5 @@\n-old\n+new\n";
    let diffs = parse(raw).unwrap();
    let actual: Vec<_> = diffs
        .iter()
        .map(|diff| (diff.file.as_str(), diff.added.as_str()))
        .collect();
    assert_eq!(
        actual,
        [
            ("a.rs", "+link\n"),
            ("c\t.rs", ""),
            ("d.nix", "+++ data\n\n+new\n")
        ]
    );
}
