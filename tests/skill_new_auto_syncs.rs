use std::{fs, path::Path, process::Command as StdCommand};

use assert_cmd::Command;
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
        let config = self.path("skillnet.toml");
        fs::write(
            &config,
            format!(
                r#"
[global]
views = [
  {{ label = "claude", path = "{}" }},
  {{ label = "agents", path = "{}" }},
]
"#,
                self.path("home/.claude/skills").display(),
                self.path("home/.agents/skills").display()
            ),
        )
        .unwrap();
        config
    }

    fn command(&self, config: &Path) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.args([
            "--config",
            config.to_str().unwrap(),
            "--mirror-root",
            self.tmp.path().to_str().unwrap(),
        ]);
        command
    }
}

fn init_clean_git_repo(path: &Path) {
    StdCommand::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["add", "-A"])
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(path)
        .output()
        .unwrap();
}

#[test]
fn skill_new_global_auto_syncs_configured_views() {
    let fixture = Fixture::new();
    let config = fixture.write_config();
    init_clean_git_repo(fixture.tmp.path());

    fixture
        .command(&config)
        .args(["skill", "new", "global/test-skill"])
        .assert()
        .success();

    let canonical = fixture.path("global/test-skill");
    assert!(canonical.join("SKILL.md").is_file());
    assert_eq!(
        fs::read_link(fixture.path("home/.claude/skills/test-skill")).unwrap(),
        canonical
    );
    assert_eq!(
        fs::read_link(fixture.path("home/.agents/skills/test-skill")).unwrap(),
        canonical
    );
}
