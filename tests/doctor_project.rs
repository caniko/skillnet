use std::{fs, os::unix::fs as unix_fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::{tempdir, TempDir};
use walkdir::WalkDir;

struct Fixture {
    tmp: TempDir,
}

struct ProjectFixture {
    config: std::path::PathBuf,
    project_root: std::path::PathBuf,
    canonical: std::path::PathBuf,
    aggregator: std::path::PathBuf,
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

    fn project(&self, project_config: &str) -> ProjectFixture {
        fs::create_dir_all(self.path("mirror/global")).unwrap();
        let project = self.path("repos/demo");
        let canonical = self.path("mirror/projects/demo");
        write_skill(&canonical, "alpha");
        write_skill(&canonical, "beta");

        let view = project.join(".claude/skills");
        fs::create_dir_all(&view).unwrap();
        unix_fs::symlink("../../.agents/skills/alpha", view.join("alpha")).unwrap();
        unix_fs::symlink("../../.agents/skills/beta", view.join("beta")).unwrap();

        let aggregator = project.join(".agents/skills");

        let config = self.write_config(format!(
            r#"
[global]
views = []

[[projects]]
name = "demo"
path = "{}"
views = [{{ rel = ".claude/skills", label = "claude" }}]
{project_config}
"#,
            project.display()
        ));

        ProjectFixture {
            config,
            project_root: project,
            canonical,
            aggregator,
        }
    }
}

fn write_skill(root: &Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), name).unwrap();
}

fn hardlink_aggregator(canonical: &Path, aggregator: &Path) {
    if aggregator.exists() {
        fs::remove_dir_all(aggregator).unwrap();
    }
    fs::create_dir_all(aggregator).unwrap();

    for entry in WalkDir::new(canonical).min_depth(1) {
        let entry = entry.unwrap();
        let source = entry.path();
        let rel = source.strip_prefix(canonical).unwrap();
        let dest = aggregator.join(rel);
        let metadata = fs::symlink_metadata(source).unwrap();

        if metadata.file_type().is_dir() {
            fs::create_dir_all(&dest).unwrap();
        } else if metadata.file_type().is_symlink() {
            unix_fs::symlink(fs::read_link(source).unwrap(), dest).unwrap();
        } else if metadata.file_type().is_file() {
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::hard_link(source, dest).unwrap();
        }
    }
}

#[test]
fn doctor_accepts_clean_hardlinked_project_aggregator() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    hardlink_aggregator(&project.canonical, &project.aggregator);

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor: no issues"));
}

#[test]
fn doctor_reports_legacy_symlink_aggregator_under_hardlink_strategy() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    fs::create_dir_all(project.aggregator.parent().unwrap()).unwrap();
    unix_fs::symlink(&project.canonical, &project.aggregator).unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "is a symlink but project uses hardlink strategy",
        ))
        .stderr(predicate::str::contains("run `skillnet project sync`"));
}

#[test]
fn doctor_reports_severed_identical_hardlink_aggregator_files_as_warning() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    hardlink_aggregator(&project.canonical, &project.aggregator);

    let canonical_file = project.canonical.join("alpha/SKILL.md");
    let aggregator_file = project.aggregator.join("alpha/SKILL.md");
    fs::remove_file(&aggregator_file).unwrap();
    fs::copy(&canonical_file, &aggregator_file).unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("warn: [demo]"))
        .stderr(predicate::str::contains("alpha/SKILL.md"))
        .stderr(predicate::str::contains("will re-link"));
}

#[test]
fn doctor_reports_diverged_hardlink_aggregator_files_as_error() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    hardlink_aggregator(&project.canonical, &project.aggregator);

    let aggregator_file = project.aggregator.join("alpha/SKILL.md");
    fs::remove_file(&aggregator_file).unwrap();
    fs::write(&aggregator_file, "aggregator edit").unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("error: [demo]"))
        .stderr(predicate::str::contains("alpha/SKILL.md"))
        .stderr(predicate::str::contains("--force"))
        .stderr(predicate::str::contains("--prefer canonical"));
}

#[test]
fn doctor_keeps_symlink_strategy_aggregator_checks() {
    let fixture = Fixture::new();
    let project = fixture.project(r#"link_strategy = "symlink""#);
    fs::create_dir_all(project.aggregator.parent().unwrap()).unwrap();
    unix_fs::symlink(&project.canonical, &project.aggregator).unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor: no issues"));

    fs::remove_file(&project.aggregator).unwrap();
    unix_fs::symlink(project.canonical.join("missing"), &project.aggregator).unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("project working-copy symlink"))
        .stderr(predicate::str::contains("expected"));
}

#[test]
fn doctor_reports_symlink_entries_inside_project_canonical() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    hardlink_aggregator(&project.canonical, &project.aggregator);
    fs::remove_dir_all(project.canonical.join("alpha")).unwrap();
    unix_fs::symlink("beta", project.canonical.join("alpha")).unwrap();

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("project canonical entry"))
        .stderr(predicate::str::contains("is a symlink"));
}

#[test]
fn doctor_reports_legacy_skills_as_info_only() {
    let fixture = Fixture::new();
    let project = fixture.project("");
    hardlink_aggregator(&project.canonical, &project.aggregator);
    write_skill(&project.project_root.join(".skills"), "alpha");

    fixture
        .command(&project.config)
        .arg("doctor")
        .assert()
        .success()
        .stderr(predicate::str::contains("legacy project skill store"));
}
