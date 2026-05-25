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

#[test]
fn project_clone_all_dry_run_prints_missing_project_clones() {
    let fixture = Fixture::new();
    let existing = fixture.path("repos/existing");
    fs::create_dir_all(&existing).unwrap();
    let missing = fixture.path("repos/missing");
    let no_origin = fixture.path("repos/no-origin");
    let config = fixture.write_config(format!(
        r#"
[global]
views = []

[[projects]]
name = "existing"
path = "{}"
origin = "git@codeberg.org:caniko/existing.git"

[[projects]]
name = "missing"
path = "{}"
origin = "git@codeberg.org:caniko/missing.git"

[[projects]]
name = "no-origin"
path = "{}"
"#,
        existing.display(),
        missing.display(),
        no_origin.display(),
    ));

    fixture
        .command(&config)
        .args(["project", "clone", "--all", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "clone missing\tgit@codeberg.org:caniko/missing.git\t{}",
            missing.display()
        )))
        .stdout(predicate::str::contains("project sync --all"))
        .stdout(predicate::str::contains("existing").not())
        .stderr(predicate::str::contains("no origin configured"));

    assert!(!missing.exists());
    assert!(!no_origin.exists());
}
