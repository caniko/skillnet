use std::{
    fs,
    os::unix::fs::{self as unix_fs, PermissionsExt},
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use filetime::{set_file_mtime, FileTime};
use predicates::prelude::*;
use tempfile::{tempdir, TempDir};

struct Fixture {
    tmp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            tmp: tempdir().unwrap(),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    fn canonical(&self) -> PathBuf {
        self.path("canonical")
    }

    fn view(&self) -> PathBuf {
        self.path("view")
    }

    fn write_config(&self) -> PathBuf {
        fs::create_dir_all(self.canonical()).unwrap();
        fs::create_dir_all(self.view()).unwrap();
        let path = self.path("skillnet.toml");
        fs::write(
            &path,
            format!(
                r#"
[global]
canonical_path = "{}"
views = [
  {{ label = "test", path = "{}", scope = "global" }},
]
"#,
                self.canonical().display(),
                self.view().display()
            ),
        )
        .unwrap();
        path
    }

    fn command(&self, config: &Path) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.args([
            "--config",
            config.to_str().unwrap(),
            "--mirror-root",
            self.root().to_str().unwrap(),
        ]);
        command
    }
}

fn write_skill(root: &Path, name: &str, body: &str, mtime_secs: i64) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    write_skill_file(&dir, "SKILL.md", body, mtime_secs);
    dir
}

fn write_skill_file(skill: &Path, rel: &str, body: &str, mtime_secs: i64) {
    let path = skill.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, body).unwrap();
    set_file_mtime(path, FileTime::from_unix_time(mtime_secs, 0)).unwrap();
}

#[test]
fn doctor_classifies_identical_non_symlink_as_info() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "identical", "same", 100);
    write_skill(&fixture.view(), "identical", "same", 100);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .success()
        .stderr(predicate::str::contains("info: [global]"))
        .stderr(predicate::str::contains(
            "`skillnet sync` will silently demote to symlink",
        ));
}

#[test]
fn doctor_classifies_view_newer_as_warn() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "view-newer", "canonical", 100);
    write_skill(&fixture.view(), "view-newer", "view", 200);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("warn: [global]"))
        .stderr(predicate::str::contains("--apply-promote"));
}

#[test]
fn doctor_classifies_canonical_newer_as_error() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "canonical-newer", "canonical", 200);
    write_skill(&fixture.view(), "canonical-newer", "view", 100);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error: [global]"))
        .stderr(predicate::str::contains("will destroy view-side edits"));
}

#[test]
fn doctor_classifies_equal_mtime_different_content_as_error() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "tie", "canonical", 100);
    write_skill(&fixture.view(), "tie", "view", 100);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error: [global]"))
        .stderr(predicate::str::contains("--prefer view|canonical"));
}

#[test]
fn doctor_classifies_both_advanced_as_error() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    let canonical = write_skill(&fixture.canonical(), "both", "canonical", 100);
    write_skill_file(&canonical, "canonical-only.md", "canonical-only", 100);
    let view = write_skill(&fixture.view(), "both", "view", 200);
    write_skill_file(&view, "view-only.md", "view-only", 200);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error: [global]"))
        .stderr(predicate::str::contains("per-file merge is not supported"));
}

#[test]
fn doctor_classifies_adopt_candidate_as_info() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.view(), "adopt", "view", 100);

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .success()
        .stderr(predicate::str::contains("info: [global]"))
        .stderr(predicate::str::contains("--adopt-new"));
}

#[test]
fn doctor_falls_through_for_other_drift_kinds() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "missing", "canonical", 100);
    unix_fs::symlink(
        "/missing/skillnet-doctor-stale",
        fixture.view().join("stale"),
    )
    .unwrap();

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error: [global]"))
        .stderr(predicate::str::contains(
            "canonical skill `missing` is missing from view",
        ))
        .stderr(predicate::str::contains("broken symlink"))
        .stderr(predicate::str::contains("not managed by skillnet"));
}

#[test]
fn doctor_handles_classify_io_error_gracefully() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    write_skill(&fixture.canonical(), "unreadable", "canonical", 100);
    let view = write_skill(&fixture.view(), "unreadable", "view", 200);
    fs::set_permissions(&view, fs::Permissions::from_mode(0o000)).unwrap();

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("could not classify entry:"))
        .stderr(predicate::str::contains(
            "rerun doctor or check permissions",
        ));

    fs::set_permissions(&view, fs::Permissions::from_mode(0o755)).unwrap();
}
