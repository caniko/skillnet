use std::{fs, os::unix::fs as unix_fs, path::Path};

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

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.root().join(rel)
    }

    fn write_config(&self, body: impl AsRef<str>) -> std::path::PathBuf {
        let path = self.path("skillnet.toml");
        fs::write(&path, body.as_ref()).unwrap();
        path
    }

    fn command(&self, config: &Path) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.args([
            "--config",
            config.to_str().unwrap(),
            "--mirror-root",
            self.root().join("mirror").to_str().unwrap(),
        ]);
        command
    }
}

fn write_skill(root: &Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), name).unwrap();
}

#[test]
fn doctor_checks_project_views_and_aggregator() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.path("mirror/global")).unwrap();
    let project = fixture.path("repos/demo");
    let canonical = project.join(".skills");
    write_skill(&canonical, "alpha");
    write_skill(&canonical, "beta");

    let view = project.join(".claude/skills");
    fs::create_dir_all(&view).unwrap();
    unix_fs::symlink("../../.skills/alpha", view.join("alpha")).unwrap();
    unix_fs::symlink("../../.skills/beta", view.join("beta")).unwrap();

    let aggregator = fixture.path("mirror/projects/demo");
    fs::create_dir_all(aggregator.parent().unwrap()).unwrap();
    unix_fs::symlink(&canonical, &aggregator).unwrap();

    let config = fixture.write_config(format!(
        r#"
[global]
views = []

[[projects]]
name = "demo"
path = "{}"
canonical_rel = ".skills"
views = [{{ rel = ".claude/skills", label = "claude" }}]
"#,
        project.display()
    ));

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor: no issues"));

    fs::remove_file(view.join("beta")).unwrap();
    unix_fs::symlink("../../.skills/missing", view.join("beta")).unwrap();

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("beta"))
        .stderr(predicate::str::contains("expected ../../.skills/beta"));
}
