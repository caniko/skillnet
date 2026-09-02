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

fn init_provider_repo(path: &Path) {
    fs::create_dir_all(path.join("skills/alpha")).unwrap();
    fs::write(path.join("skills/alpha/SKILL.md"), "alpha v1").unwrap();
    git(path, ["init", "-b", "main"]);
    git(path, ["config", "user.email", "skillnet@example.invalid"]);
    git(path, ["config", "user.name", "Skillnet Test"]);
    git(path, ["add", "."]);
    git(path, ["commit", "-m", "initial"]);
}

fn replace_provider_alpha_with_beta(path: &Path) {
    fs::remove_dir_all(path.join("skills/alpha")).unwrap();
    fs::create_dir_all(path.join("skills/beta")).unwrap();
    fs::write(path.join("skills/beta/SKILL.md"), "beta provider").unwrap();
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

fn provider_config(fixture: &Fixture, source: &Path) -> String {
    format!(
        r#"
[global]
canonical_path = "{}"
views = [{{ label = "test", path = "{}", scope = "global" }}]

[subscriptions.external]
url = "{}"
ref = "main"
source = "skills"
provider = true
"#,
        fixture.path("mirror/global").display(),
        fixture.path("view").display(),
        source.display(),
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
        .stderr(predicates::str::contains("overlaps canonical scope"));
}

#[test]
fn provider_subscription_updates_generated_views_and_prunes_removed_skills() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    fs::create_dir_all(fixture.path("mirror/global")).unwrap();
    let config = fixture.write_config(provider_config(&fixture, &source));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();
    assert!(fixture.path("view/alpha").is_symlink());
    assert_eq!(
        fs::read_to_string(fixture.path("view/alpha/SKILL.md")).unwrap(),
        "alpha v1"
    );
    assert!(!fixture.path("view/alpha/SKILL.md").is_symlink());
    fixture.command(&config).arg("doctor").assert().success();

    fs::create_dir_all(fixture.path("view/manual")).unwrap();
    fs::write(fixture.path("view/manual/SKILL.md"), "manual").unwrap();
    assert_eq!(
        fs::read_to_string(fixture.path("view/manual/SKILL.md")).unwrap(),
        "manual"
    );

    replace_provider_alpha_with_beta(&source);
    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();

    assert!(!fixture.path("view/alpha").exists());
    assert!(fixture.path("view/beta").is_symlink());
    assert_eq!(
        fs::read_to_string(fixture.path("view/beta/SKILL.md")).unwrap(),
        "beta provider"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("view/manual/SKILL.md")).unwrap(),
        "manual"
    );
}

#[test]
fn provider_sync_preserves_host_user_filtering_custom_bundles_and_discovery() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    let canonical = fixture.path("mirror/global");
    fs::create_dir_all(canonical.join("atlas-only")).unwrap();
    fs::write(canonical.join("atlas-only/SKILL.md"), "atlas adapter").unwrap();
    fs::write(
        canonical.join("Skillnet.pkl"),
        r#"
class Skill {
  users: Listing<String>? = null
  hosts: Listing<String>? = null
  dependencies: Listing<String> = new {}
}
schemaVersion = 3
skills: Mapping<String, Skill> = new {
  ["atlas-only"] = new {
    users = List("can")
    hosts = List("atlas")
    dependencies = List("alpha")
  }
}
"#,
    )
    .unwrap();
    let project = fixture.path("projects/owned/project");
    init_provider_repo(&project);
    fs::create_dir_all(project.join(".skills/project-skill")).unwrap();
    fs::write(project.join(".skills/project-skill/SKILL.md"), "project").unwrap();
    fs::write(project.join(".gitignore"), ".agents/\n.claude/\n").unwrap();
    git(&project, ["add", ".skills", ".gitignore"]);
    git(&project, ["commit", "-m", "add project skill"]);
    let tree = fixture.path("project-tree.json");
    fs::write(
        &tree,
        serde_json::to_string(&serde_json::json!({
            "schemaVersion": 1,
            "root": fixture.path("projects"),
            "layout": {"primary": {"owned": "owned"}}
        }))
        .unwrap(),
    )
    .unwrap();
    for (user, host) in [
        ("can", "atlas"),
        ("can", "nomad"),
        ("dejana", "atlas"),
        ("dejana", "nomad"),
    ] {
        let config = fixture.write_config(format!(
            r#"user = "{user}"
host = "{host}"
bundles_root = "{}"
{}
[project_discovery]
project_tree = "{}"
classes = ["owned"]
marker = ".skills"
"#,
            fixture.path("custom-bundles").display(),
            provider_config(&fixture, &source),
            tree.display(),
        ));
        fixture
            .command(&config)
            .args(["subscription", "sync", "--all"])
            .assert()
            .success();
        fixture
            .command(&config)
            .args(["project", "sync", "--all"])
            .assert()
            .success();
        assert_eq!(
            fs::read_to_string(fixture.path("view/alpha/SKILL.md")).unwrap(),
            "alpha v1"
        );
        let granted = user == "can" && host == "atlas";
        assert_eq!(fixture.path("view/atlas-only").exists(), granted);
        assert_eq!(fixture.path("custom-bundles/global/atlas-only").exists(), granted);
        if granted {
            assert_eq!(
                fs::read_to_string(fixture.path(
                    "custom-bundles/global/atlas-only/.skillnet/deps/alpha/SKILL.md"
                ))
                .unwrap(),
                "alpha v1"
            );
        }
        assert!(project.join(".agents/skills/project-skill/SKILL.md").is_file());
        assert!(!project.join(".agents/skills/alpha").exists());
        assert!(!fixture.path("data/bundles/global").exists());
    }
}

#[test]
fn provider_subscription_rejects_custom_bundle_overlapping_canonical_before_clone() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    fs::create_dir_all(fixture.path("mirror/global")).unwrap();
    let config = fixture.write_config(format!(
        "bundles_root = {:?}\n{}",
        fixture.path("mirror").to_str().unwrap(),
        provider_config(&fixture, &source),
    ));
    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("overlaps canonical scope"));
    assert!(!fixture.path("data/subscriptions/external").exists());
}

#[test]
fn provider_subscription_restores_last_good_checkout_after_collision() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    fs::create_dir_all(fixture.path("mirror/global/beta")).unwrap();
    fs::write(
        fixture.path("mirror/global/beta/SKILL.md"),
        "beta canonical",
    )
    .unwrap();
    let config = fixture.write_config(provider_config(&fixture, &source));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .success();
    replace_provider_alpha_with_beta(&source);

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "restored last-known-good checkout",
        ));

    assert_eq!(
        fs::read_to_string(fixture.path("view/alpha/SKILL.md")).unwrap(),
        "alpha v1"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("view/beta/SKILL.md")).unwrap(),
        "beta canonical"
    );
    assert!(fixture
        .path("data/subscriptions/external/repo/skills/alpha/SKILL.md")
        .is_file());
}

#[test]
fn provider_subscription_rejects_source_escape_before_clone() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    let config = fixture.write_config(
        provider_config(&fixture, &source).replace("source = \"skills\"", "source = \"../skills\""),
    );

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "source must stay within its checkout",
        ));
    assert!(!fixture.path("data/subscriptions/external").exists());
}

#[test]
fn subscription_rejects_path_component_name_before_clone() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    let config = fixture.write_config(provider_config(&fixture, &source).replace(
        "[subscriptions.external]",
        "[subscriptions.\"../external\"]",
    ));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "subscription name must be one path component",
        ));
    assert!(!fixture.path("data/subscriptions/external").exists());
}

#[test]
fn provider_subscription_rejects_symlinks_outside_source_root() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    fs::write(fixture.path("outside.txt"), "private").unwrap();
    std::os::unix::fs::symlink(
        fixture.path("outside.txt"),
        source.join("skills/alpha/outside.txt"),
    )
    .unwrap();
    git(&source, ["add", "."]);
    git(&source, ["commit", "-m", "add escaping symlink"]);
    let config = fixture.write_config(provider_config(&fixture, &source));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "provider symlink escapes its source root",
        ));
    assert!(!fixture.path("data/subscriptions/external").exists());
}

#[test]
fn provider_subscription_rejects_reserved_hidden_skill_names() {
    let fixture = Fixture::new();
    let source = fixture.path("source");
    init_provider_repo(&source);
    fs::rename(
        source.join("skills/alpha"),
        source.join("skills/.skillnet-tmp"),
    )
    .unwrap();
    git(&source, ["add", "-A"]);
    git(&source, ["commit", "-m", "use reserved skill name"]);
    let config = fixture.write_config(provider_config(&fixture, &source));

    fixture
        .command(&config)
        .args(["subscription", "sync", "--all"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "invalid provider skill name `.skillnet-tmp`",
        ));
    assert!(!fixture.path("data/subscriptions/external").exists());
}
