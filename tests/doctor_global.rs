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
            self.root().to_str().unwrap(),
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
fn doctor_flags_global_orphan_and_broken_view_symlinks() {
    let fixture = Fixture::new();
    let canonical = fixture.path("global");
    write_skill(&canonical, "alpha");
    write_skill(&canonical, "beta");

    let views = [
        "home/.claude/skills",
        "home/.agents/skills",
        "home/.codex/skills",
    ];
    for view in views {
        let view = fixture.path(view);
        fs::create_dir_all(&view).unwrap();
        for skill in ["alpha", "beta"] {
            unix_fs::symlink(canonical.join(skill), view.join(skill)).unwrap();
        }
    }

    let config = fixture.write_config(format!(
        r#"
[global]
views = [
  {{ label = "claude", path = "{}", scope = "global" }},
  {{ label = "agents", path = "{}", scope = "global" }},
  {{ label = "codex", path = "{}", scope = "global" }},
]
"#,
        fixture.path("home/.claude/skills").display(),
        fixture.path("home/.agents/skills").display(),
        fixture.path("home/.codex/skills").display(),
    ));

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor: no issues"));

    unix_fs::symlink(
        "/nonexistent/skillnet-test-orphan",
        fixture.path("home/.claude/skills/orphan"),
    )
    .unwrap();

    fixture
        .command(&config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("orphan"))
        .stderr(predicate::str::contains("broken symlink"))
        .stderr(predicate::str::contains("not managed by skillnet"));
}
