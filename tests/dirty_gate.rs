use std::{fs, process::Command};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use filetime::{set_file_mtime, FileTime};
use skillnet::{
    model::ViewTarget,
    view::{materialize_view_with_promotion, PromotionOptions},
};
use tempfile::{tempdir, TempDir};

struct Fixture {
    _tmp: TempDir,
    project: Utf8PathBuf,
    canonical: Utf8PathBuf,
    view_path: Utf8PathBuf,
    view: ViewTarget,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let project = root.join("project");
        let canonical = project.join(".skills");
        let view_path = project.join(".claude/skills");
        let view = ViewTarget {
            label: "claude".into(),
            path: view_path.clone(),
        };
        Self {
            _tmp: tmp,
            project,
            canonical,
            view_path,
            view,
        }
    }
}

fn write_skill(root: &Utf8Path, name: &str, body: &str, mtime_secs: i64) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    let skill = dir.join("SKILL.md");
    fs::write(&skill, body).unwrap();
    set_file_mtime(skill, FileTime::from_unix_time(mtime_secs, 0)).unwrap();
}

fn init_dirty_git_repo(path: &Utf8Path) {
    fs::create_dir_all(path).unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
    fs::write(path.join("dirty.txt"), "dirty").unwrap();
}

fn git_gate(allow_dirty: bool) -> impl Fn(&Utf8Path) -> Result<()> {
    move |target| {
        if allow_dirty || !target.join(".git").exists() {
            return Ok(());
        }
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(target)
            .output()
            .with_context(|| format!("failed to run git in {target}"))?;
        if !output.status.success() {
            bail!(
                "git command failed in `{}`: {}",
                target,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if !String::from_utf8_lossy(&output.stdout).trim().is_empty() {
            bail!("destination target `{target}` is dirty");
        }
        Ok(())
    }
}

fn promotion_options(project: &Utf8Path) -> PromotionOptions {
    PromotionOptions {
        apply_promote: true,
        project_root: Some(project.to_path_buf()),
        relative_links: true,
        ..PromotionOptions::default()
    }
}

#[test]
fn per_project_dirty_gate_refuses_promotion() {
    let fixture = Fixture::new();
    init_dirty_git_repo(&fixture.project);
    write_skill(&fixture.canonical, "dirty-gate", "canonical", 100);
    write_skill(&fixture.view_path, "dirty-gate", "view", 200);

    let err = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        promotion_options(&fixture.project),
        git_gate(false),
    )
    .unwrap_err();

    assert!(err.to_string().contains(fixture.project.as_str()));
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("dirty-gate/SKILL.md")).unwrap(),
        "canonical"
    );
}

#[test]
fn per_project_dirty_gate_bypassed_with_allow_dirty() {
    let fixture = Fixture::new();
    init_dirty_git_repo(&fixture.project);
    write_skill(&fixture.canonical, "dirty-bypass", "canonical", 100);
    write_skill(&fixture.view_path, "dirty-bypass", "view", 200);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        promotion_options(&fixture.project),
        git_gate(true),
    )
    .unwrap();

    assert_eq!(summary.promoted, vec!["dirty-bypass"]);
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("dirty-bypass/SKILL.md")).unwrap(),
        "view"
    );
}

#[test]
fn non_git_target_skips_gate() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.project).unwrap();
    write_skill(&fixture.canonical, "non-git", "canonical", 100);
    write_skill(&fixture.view_path, "non-git", "view", 200);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        promotion_options(&fixture.project),
        git_gate(false),
    )
    .unwrap();

    assert_eq!(summary.promoted, vec!["non-git"]);
}
