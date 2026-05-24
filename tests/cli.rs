use std::{fs, path::Path, process::Command as StdCommand};

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

    fn write_catalog_config(&self, body: impl AsRef<str>) -> std::path::PathBuf {
        let path = self.path("skillnet.catalog.toml");
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

    fn command_with_catalog(&self, config: &Path, catalog_config: &Path) -> Command {
        let mut command = self.command(config);
        command.args(["--catalog-config", catalog_config.to_str().unwrap()]);
        command
    }

    fn raw_command(&self, config: &Path) -> Command {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command.args(["--config", config.to_str().unwrap()]);
        command
    }
}

fn write_skill(root: &Path, name: &str, body: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), body).unwrap();
}

fn minimal_config() -> &'static str {
    "[global]\nsources = []\nsync_paths = []\nstale_codex_skill_paths = []\n"
}

fn minimal_config_with_skills_root(root: &Path) -> String {
    format!(
        r#"skills_root = "{}"

[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []
"#,
        root.display()
    )
}

fn init_git_repo(path: &Path) {
    StdCommand::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
}

fn global_config(agents: &Path, claude: &Path, codex: &Path) -> String {
    format!(
        r#"
[global]
sources = [
  {{ label = "agents", path = "{}", priority = 3 }},
  {{ label = "claude", path = "{}", priority = 2 }},
  {{ label = "codex", path = "{}", priority = 1 }},
]
sync_paths = ["{}", "{}"]
stale_codex_skill_paths = ["{}"]
"#,
        agents.display(),
        claude.display(),
        codex.display(),
        agents.display(),
        claude.display(),
        codex.display()
    )
}

#[test]
fn sync_pull_only_writes_the_selected_mirror_scope() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .success();

    assert!(fixture.path("global/alpha/SKILL.md").is_file());
    assert!(codex.exists());
    assert!(fixture.path(".skillnet/cache.toml").is_file());
}

#[test]
fn sync_pull_then_push_writes_live_targets_and_removes_stale_codex_skills() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&codex, "alpha", "a");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();

    assert!(agents.join("alpha/SKILL.md").is_file());
    assert!(claude.join("alpha/SKILL.md").is_file());
    assert!(!codex.exists());
}

#[test]
fn sync_pull_then_push_does_not_push_after_a_pull_failure() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    let bad_config = fixture.write_config("not = [valid");

    fixture
        .command(&bad_config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse config file"));

    assert!(!agents.exists());
    assert!(!claude.exists());
    assert!(!codex.exists());
}

#[test]
fn project_scopes_are_pulled_with_all_and_empty_projects_get_manifests() {
    let fixture = Fixture::new();
    let project_a = fixture.path("repos/project-a");
    let project_b = fixture.path("repos/project-b");
    write_skill(&project_a.join(".agents/skills"), "alpha", "a");
    fs::create_dir_all(&project_b).unwrap();

    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[project_source_rules]]
label = "agents"
rel = ".agents/skills"
priority = 1

[[projects]]
name = "project-a"
path = "{}"

[[projects]]
name = "project-b"
path = "{}"
"#,
        project_a.display(),
        project_b.display()
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success();

    assert!(fixture.path("projects/project-a/alpha/SKILL.md").is_file());
    assert!(fixture
        .path("projects/project-b/RECONCILIATION.md")
        .is_file());
}

#[test]
fn skill_delete_honors_global_dry_run() {
    let fixture = Fixture::new();
    write_skill(&fixture.path("global"), "alpha", "a");
    let config = fixture.write_config(minimal_config());

    fixture
        .command(&config)
        .args(["--dry-run", "skill", "delete", "global/alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("delete"));

    assert!(fixture.path("global/alpha/SKILL.md").is_file());
}

#[test]
fn skill_rename_and_move_update_mirrored_skill_directories() {
    let fixture = Fixture::new();
    let project = fixture.path("repos/demo");
    fs::create_dir_all(&project).unwrap();
    write_skill(&fixture.path("global"), "alpha", "a");
    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[projects]]
name = "demo"
path = "{}"
"#,
        project.display()
    ));

    fixture
        .command(&config)
        .args(["skill", "rename", "global/alpha", "beta"])
        .assert()
        .success();
    assert!(!fixture.path("global/alpha").exists());
    assert!(fixture.path("global/beta/SKILL.md").is_file());

    fixture
        .command(&config)
        .args(["skill", "move", "global/beta", "demo"])
        .assert()
        .success();
    assert!(!fixture.path("global/beta").exists());
    assert!(fixture.path("projects/demo/beta/SKILL.md").is_file());

    fixture
        .command(&config)
        .args(["skill", "move", "demo/beta", "global"])
        .assert()
        .success();
    assert!(fixture.path("global/beta/SKILL.md").is_file());
    assert!(!fixture.path("projects/demo/beta").exists());
}

#[test]
fn skill_list_scope_list_and_scope_sources_report_configured_state() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&fixture.path("global"), "alpha", "a");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["skill", "list", "--scope", "global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# global"))
        .stdout(predicate::str::contains("alpha"));

    fixture
        .command(&config)
        .args(["scope", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global"));

    fixture
        .command(&config)
        .args(["scope", "sources", "--scope", "global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agents"))
        .stdout(predicate::str::contains("claude"))
        .stdout(predicate::str::contains("codex"));
}

#[test]
fn invalid_scope_reports_valid_configured_scopes() {
    let fixture = Fixture::new();
    let project = fixture.path("repos/demo");
    fs::create_dir_all(&project).unwrap();
    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[projects]]
name = "demo"
path = "{}"
"#,
        project.display()
    ));

    fixture
        .command(&config)
        .args(["sync", "status", "--scope", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown scope `nope`"))
        .stderr(predicate::str::contains("global, demo"));
}

#[test]
fn project_add_remove_and_list_update_config() {
    let fixture = Fixture::new();
    let project_root = fixture.path("repos/new-project");
    fs::create_dir_all(&project_root).unwrap();
    let config = fixture.write_config(minimal_config());

    fixture
        .command(&config)
        .args([
            "project",
            "add",
            "new-project",
            project_root.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("added project new-project"));

    let updated = fs::read_to_string(&config).unwrap();
    assert!(updated.contains("[[projects]]"));
    assert!(updated.contains("name = \"new-project\""));
    assert!(updated.contains(&format!("path = \"{}\"", project_root.display())));

    fixture
        .command(&config)
        .args(["project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("new-project"));

    fixture
        .command(&config)
        .args(["project", "remove", "new-project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed project new-project"));

    let updated = fs::read_to_string(&config).unwrap();
    assert!(!updated.contains("new-project"));
}

#[test]
fn project_add_dry_run_does_not_mutate_config() {
    let fixture = Fixture::new();
    let original = minimal_config();
    let config = fixture.write_config(original);

    fixture
        .command(&config)
        .args([
            "--dry-run",
            "project",
            "add",
            "future-project",
            "/tmp/future-project",
            "--allow-missing",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("add project future-project"));

    assert_eq!(fs::read_to_string(&config).unwrap(), original);
}

#[test]
fn catalog_generate_creates_docs_and_project_indexes() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());
    let catalog_config = fixture.write_catalog_config(
        r#"
[[rules]]
path_prefix = "global/"
scope = "global"
category = "agent-tools"
status = "active"

[[rules]]
path_prefix = "projects/"
scope = "project"
category = "domain-workflow"
status = "active"
"#,
    );
    write_skill(
        &fixture.path("global"),
        "alpha",
        "---\nname: alpha\ndescription: Alpha skill\n---\n",
    );
    write_skill(
        &fixture.path("projects/demo"),
        "beta",
        "---\nname: beta\ndescription: Beta skill\n---\n",
    );

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["catalog", "generate"])
        .assert()
        .success();

    assert!(fixture.path("CATALOG.md").is_file());
    assert!(fixture.path("ROUTING.md").is_file());
    assert!(fixture.path("SKILL_CONFLICTS.md").is_file());
    assert!(fixture.path("projects/demo/INDEX.md").is_file());
}

#[test]
fn catalog_lint_rejects_invalid_metadata() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());
    let catalog_config = fixture.write_catalog_config(
        r#"
[[rules]]
path_prefix = "global/"
scope = "global"
category = "wrong"
status = "active"
related_skills = ["missing-skill"]
"#,
    );
    write_skill(
        &fixture.path("global"),
        "alpha",
        "---\nname: alpha\ndescription: Alpha skill\n---\n",
    );

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["catalog", "lint"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown category"))
        .stderr(predicate::str::contains("related skill"));
}

#[test]
fn skill_show_and_catalog_search_use_effective_metadata() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());
    let catalog_config = fixture.write_catalog_config(
        r#"
[[rules]]
path_prefix = "global/"
scope = "global"
category = "ci-release"
status = "active"
tags = ["forgejo"]
"#,
    );
    write_skill(
        &fixture.path("global"),
        "forgejo-ci",
        "---\nname: forgejo-ci\ndescription: Forgejo CI skill\n---\n",
    );

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["skill", "show", "global/forgejo-ci"])
        .assert()
        .success()
        .stdout(predicate::str::contains("path:"))
        .stdout(predicate::str::contains("catalog entry:"))
        .stdout(predicate::str::contains("category:       ci-release"));

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["catalog", "search", "forgejo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global/forgejo-ci"));
}

#[test]
fn skill_show_missing_skill_reports_not_found() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());

    fixture
        .command(&config)
        .args(["skill", "show", "global/missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn no_args_runs_status() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());

    fixture
        .command(&config)
        .assert()
        .success()
        .stdout(predicate::str::contains("scopes:"))
        .stdout(predicate::str::contains("global"));
}

#[test]
fn status_reports_git_destination_from_skills_root() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let config = fixture.write_config(minimal_config_with_skills_root(fixture.root()));

    fixture
        .raw_command(&config)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination:"))
        .stdout(predicate::str::contains("git"));
}

#[test]
fn dirty_git_destination_blocks_mutating_mirror_commands() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    fs::write(fixture.path("untracked.txt"), "dirty").unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("dirty entries"));

    assert!(!fixture.path("global/alpha/SKILL.md").exists());
}

#[test]
fn allow_dirty_destination_bypasses_git_write_guard() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    fs::write(fixture.path("untracked.txt"), "dirty").unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args([
            "--allow-dirty-destination",
            "sync",
            "pull",
            "--scope",
            "global",
        ])
        .assert()
        .success();

    assert!(fixture.path("global/alpha/SKILL.md").is_file());
}

#[test]
fn cache_is_best_effort_for_status() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));
    let cache = fixture.path(".skillnet/cache.toml");

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .success();
    assert!(cache.is_file());

    fs::remove_file(&cache).unwrap();
    fixture
        .command(&config)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global"));

    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, "garbage").unwrap();
    fixture
        .command(&config)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global"));
}

#[test]
fn sync_status_reports_clean_and_then_diverged() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .success();

    fixture
        .command(&config)
        .args(["sync", "status", "--scope", "global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global  clean"));

    fs::write(agents.join("alpha/SKILL.md"), "changed").unwrap();
    fixture
        .command(&config)
        .args(["sync", "status", "--scope", "global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global  diverged"));
}

#[test]
fn global_dry_run_sync_push_plans_all_configured_scopes() {
    let fixture = Fixture::new();
    let project_root = fixture.path("repos/demo");
    fs::create_dir_all(&project_root).unwrap();
    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[projects]]
name = "demo"
path = "{}"
"#,
        project_root.display()
    ));

    fixture
        .command(&config)
        .args(["--dry-run", "sync", "push", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# sync global"))
        .stdout(predicate::str::contains("# sync demo"));
}
