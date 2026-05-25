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
            self.root().to_str().unwrap(),
        ]);
        command
    }
}

#[test]
fn doctor_warns_when_project_repo_is_missing() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.path("global")).unwrap();
    let missing = fixture.path("repos/missing");
    let config = fixture.write_config(format!(
        r#"
[global]
views = []

[[projects]]
name = "missing"
path = "{}"
"#,
        missing.display()
    ));

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("warn: [missing]"))
        .stderr(predicate::str::contains("does not exist"))
        .stderr(predicate::str::contains("error:").not());
}
