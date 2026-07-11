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
            self.path("mirror").to_str().unwrap(),
        ]);
        command.env("SKILLNET_DATA_DIR", self.path("data"));
        command
    }
}

fn init_source_repo(path: &Path) {
    fs::create_dir_all(path.join("global_skills/alpha")).unwrap();
    fs::write(path.join("global_skills/alpha/SKILL.md"), "alpha v1").unwrap();
    git(path, ["init", "-b", "main"]);
    git(path, ["config", "user.email", "skillnet@example.invalid"]);
    git(path, ["config", "user.name", "Skillnet Test"]);
    git(path, ["add", "."]);
    git(path, ["commit", "-m", "initial"]);
}

fn replace_source_alpha_with_beta(path: &Path) {
    fs::remove_dir_all(path.join("global_skills/alpha")).unwrap();
    fs::create_dir_all(path.join("global_skills/beta")).unwrap();
    fs::write(path.join("global_skills/beta/SKILL.md"), "beta v1").unwrap();
    git(path, ["add", "-A"]);
    git(path, ["commit", "-m", "replace alpha with beta"]);
}

fn git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let status = StdCommand::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed with {status}");
}

fn subscription_config(source: &Path, target: &Path, delete_policy: &str) -> String {
    format!(
        r#"
[global]
views = []

[subscriptions.ai-skills]
url = "{}"
ref = "main"
target = "{}"
delete_policy = "{delete_policy}"
"#,
        source.display(),
        target.display(),
    )
}

#[test]
fn subscription_sync_keep_preserves_skills_deleted_upstream() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    let target = fixture.path("target");
    init_source_repo(&source);
    let config = fixture.write_config(subscription_config(&source, &target, "keep"));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(target.join("alpha/SKILL.md")).unwrap(),
        "alpha v1"
    );

    replace_source_alpha_with_beta(&source);
    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(target.join("alpha/SKILL.md")).unwrap(),
        "alpha v1"
    );
    assert_eq!(
        fs::read_to_string(target.join("beta/SKILL.md")).unwrap(),
        "beta v1"
    );
}

#[test]
fn subscription_sync_prune_removes_skills_deleted_upstream() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    let target = fixture.path("target");
    init_source_repo(&source);
    let config = fixture.write_config(subscription_config(&source, &target, "prune"));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();
    assert!(target.join("alpha/SKILL.md").is_file());

    replace_source_alpha_with_beta(&source);
    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();

    assert!(!target.join("alpha").exists());
    assert_eq!(
        fs::read_to_string(target.join("beta/SKILL.md")).unwrap(),
        "beta v1"
    );
}

#[test]
fn subscription_sync_rejects_canonical_targets() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    let target = fixture.path("mirror/global_skills");
    init_source_repo(&source);
    let config = fixture.write_config(format!(
        r#"
[global]
canonical_path = "{}"
views = []

[subscriptions.ai-skills]
url = "{}"
ref = "main"
target = "{}"
delete_policy = "keep"
"#,
        target.display(),
        source.display(),
        target.display(),
    ));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("inside canonical scope"));
}
