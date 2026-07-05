use std::{fs, path::Path};

use assert_cmd::Command;
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

    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.tmp.path().join(rel)
    }

    fn write_config(&self) -> std::path::PathBuf {
        let path = self.path("skillnet.toml");
        fs::write(
            &path,
            format!(
                r#"
[global]
views = [{{ label = "agents", path = "{}" }}]
"#,
                self.path("agents").display()
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
            self.path("mirror").to_str().unwrap(),
        ]);
        command.env("SKILLNET_DATA_DIR", self.path("data"));
        command
    }

    fn write_source_skill(&self, name: &str, body: &str) {
        let dir = self.path(&format!("repo/skills/{name}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    fn write_global_skill(&self, name: &str, body: &str) {
        let dir = self.path(&format!("mirror/global/{name}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }
}

#[test]
fn export_copies_source_skills_to_global_and_syncs_view() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fixture.write_source_skill("alpha", "alpha v1");
    fixture.write_source_skill("beta", "beta v1");

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("exported alpha"))
        .stdout(predicate::str::contains("exported beta"));

    assert_eq!(
        fs::read_to_string(fixture.path("mirror/global/alpha/SKILL.md")).unwrap(),
        "alpha v1"
    );
    assert_eq!(
        fs::read_link(fixture.path("agents/alpha")).unwrap(),
        fixture.path("mirror/global/alpha")
    );
}

#[test]
fn export_dry_run_does_not_write_files() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fixture.write_source_skill("alpha", "alpha v1");

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["--dry-run", "export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("copy alpha"));

    assert!(!fixture.path("mirror/global/alpha").exists());
    assert!(!fixture.path("agents/alpha").exists());
}

#[test]
fn export_filter_copies_only_requested_skill() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fixture.write_source_skill("alpha", "alpha v1");
    fixture.write_source_skill("beta", "beta v1");

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export", "--skill", "beta", "--no-view-sync"])
        .assert()
        .success();

    assert!(!fixture.path("mirror/global/alpha").exists());
    assert_eq!(
        fs::read_to_string(fixture.path("mirror/global/beta/SKILL.md")).unwrap(),
        "beta v1"
    );
    assert!(!fixture.path("agents/beta").exists());
}

#[test]
fn export_filter_errors_when_requested_skill_is_missing() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fs::create_dir_all(fixture.path("repo/skills")).unwrap();

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export", "--skill", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requested skill `missing`"));
}

#[test]
fn export_prune_removes_stale_skill_dirs_and_preserves_non_skills() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fixture.write_source_skill("alpha", "alpha v1");
    fixture.write_global_skill("alpha", "old alpha");
    fixture.write_global_skill("stale", "stale");
    fs::create_dir_all(fixture.path("mirror/global/not-a-skill")).unwrap();
    fs::write(fixture.path("mirror/global/not-a-skill/README.md"), "keep").unwrap();

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export", "--prune", "--no-view-sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pruned"));

    assert!(fixture.path("mirror/global/alpha/SKILL.md").is_file());
    assert!(!fixture.path("mirror/global/stale").exists());
    assert!(fixture
        .path("mirror/global/not-a-skill/README.md")
        .is_file());
}

#[test]
fn export_filtered_prune_keeps_unselected_skills_present_in_source() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fixture.write_source_skill("alpha", "alpha v1");
    fixture.write_source_skill("beta", "beta v1");
    fixture.write_global_skill("alpha", "existing alpha");
    fixture.write_global_skill("stale", "stale");

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export", "--skill", "beta", "--prune", "--no-view-sync"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(fixture.path("mirror/global/alpha/SKILL.md")).unwrap(),
        "existing alpha"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("mirror/global/beta/SKILL.md")).unwrap(),
        "beta v1"
    );
    assert!(!fixture.path("mirror/global/stale").exists());
}

#[test]
fn export_rejects_non_directory_source_entries() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    fs::create_dir_all(fixture.path("repo/skills")).unwrap();
    fs::write(fixture.path("repo/skills/not-dir"), "nope").unwrap();

    fixture
        .command(&config)
        .current_dir(fixture.path("repo"))
        .args(["export"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a directory"));
}
