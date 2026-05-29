use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Read,
    os::unix::fs as unix_fs,
    path::Path,
    process::Command as StdCommand,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use filetime::{set_file_mtime, set_file_times, FileTime};
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::{tempdir, TempDir};
use walkdir::WalkDir;

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

fn read_skill(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join(name).join("SKILL.md")).unwrap()
}

fn wait_for_mtime_tick() {
    thread::sleep(Duration::from_millis(20));
}

fn set_skill_file_mtime(root: &Path, name: &str, time: FileTime) {
    set_file_mtime(root.join(name).join("SKILL.md"), time).unwrap();
}

fn write_skill_with_files(root: &Path, name: &str, files: &[(&str, &str)]) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    for (rel, body) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }
}

fn symlink_file(target: &str, link: &Path) {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    unix_fs::symlink(target, link).unwrap();
}

fn sync_paths_config(
    sources: &[(&str, &Path, u32)],
    sync_paths: &[&Path],
    stale: &[&Path],
) -> String {
    let source_lines = sources
        .iter()
        .map(|(label, path, priority)| {
            format!(
                r#"  {{ label = "{label}", path = "{}", priority = {priority} }}"#,
                path.display()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let sync_paths = sync_paths
        .iter()
        .map(|path| format!(r#""{}""#, path.display()))
        .collect::<Vec<_>>()
        .join(", ");
    let stale = stale
        .iter()
        .map(|path| format!(r#""{}""#, path.display()))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"
[global]
sources = [
{source_lines}
]
sync_paths = [{sync_paths}]
stale_codex_skill_paths = [{stale}]
"#
    )
}

fn assert_sync_paths_parity(paths: &[&Path]) -> BTreeMap<String, String> {
    assert!(
        !paths.is_empty(),
        "assert_sync_paths_parity requires at least one path"
    );
    let maps = paths
        .iter()
        .map(|path| ((*path).to_path_buf(), sync_path_entries(path)))
        .collect::<Vec<_>>();
    let expected = &maps[0].1;

    let mismatches = maps
        .iter()
        .skip(1)
        .filter(|(_, entries)| entries != expected)
        .collect::<Vec<_>>();
    if !mismatches.is_empty() {
        let mut message = String::from("sync path parity mismatch\n");
        for (path, entries) in &maps {
            message.push_str(&format!(
                "\n{}:\n{}",
                path.display(),
                parity_difference(expected, entries)
            ));
        }
        panic!("{message}");
    }

    expected.clone()
}

fn sync_path_entries(path: &Path) -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    for entry in WalkDir::new(path).follow_links(false).min_depth(1) {
        let entry = entry.unwrap();
        let metadata = fs::symlink_metadata(entry.path()).unwrap();
        if !(metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(path)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        entries.insert(rel, sync_path_entry_hash(entry.path()));
    }
    entries
}

fn sync_path_entry_hash(path: &Path) -> String {
    let metadata = fs::symlink_metadata(path).unwrap();
    let mut hasher = Sha256::new();
    if metadata.file_type().is_symlink() {
        hasher.update(b"symlink");
        hasher.update(fs::read_link(path).unwrap().to_string_lossy().as_bytes());
    } else {
        hasher.update(b"file");
        let mut file = fs::File::open(path).unwrap();
        let mut buf = [0; 8192];
        loop {
            let n = file.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    format!("{:x}", hasher.finalize())
}

fn parity_difference(
    expected: &BTreeMap<String, String>,
    actual: &BTreeMap<String, String>,
) -> String {
    let expected_keys = expected.keys().collect::<BTreeSet<_>>();
    let actual_keys = actual.keys().collect::<BTreeSet<_>>();
    let mut lines = Vec::new();

    for key in expected_keys.difference(&actual_keys) {
        lines.push(format!("  missing: {key}"));
    }
    for key in actual_keys.difference(&expected_keys) {
        lines.push(format!("  extra: {key}"));
    }
    for key in expected_keys.intersection(&actual_keys) {
        if expected[*key] != actual[*key] {
            lines.push(format!(
                "  changed: {key}\n    expected: {}\n    actual:   {}",
                expected[*key], actual[*key]
            ));
        }
    }

    if lines.is_empty() {
        "  matches baseline\n".to_string()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn minimal_config() -> &'static str {
    "[global]\nviews = []\n"
}

fn minimal_config_with_skills_root(root: &Path) -> String {
    format!(
        r#"skills_root = "{}"

[global]
views = []
"#,
        root.display()
    )
}

fn project_config(project_name: &str, project_root: &Path) -> String {
    format!(
        r#"
[global]
views = []

[[projects]]
name = "{project_name}"
path = "{}"
canonical_rel = ".skills"
"#,
        project_root.display()
    )
}

fn assert_rfc3339_utc(value: &str) {
    assert_eq!(value.len(), 20, "{value} is not YYYY-MM-DDTHH:MM:SSZ");
    assert_eq!(&value[4..5], "-");
    assert_eq!(&value[7..8], "-");
    assert_eq!(&value[10..11], "T");
    assert_eq!(&value[13..14], ":");
    assert_eq!(&value[16..17], ":");
    assert_eq!(&value[19..20], "Z");
    assert!(value[..4].parse::<u16>().is_ok());
    assert!((1..=12).contains(&value[5..7].parse::<u8>().unwrap()));
    assert!((1..=31).contains(&value[8..10].parse::<u8>().unwrap()));
    assert!(value[11..13].parse::<u8>().unwrap() < 24);
    assert!(value[14..16].parse::<u8>().unwrap() < 60);
    assert!(value[17..19].parse::<u8>().unwrap() < 60);
}

fn init_git_repo(path: &Path) {
    StdCommand::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
}

fn commit_all(path: &Path, message: &str) {
    StdCommand::new("git")
        .args(["add", "-A"])
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args([
            "-c",
            "user.name=skillnet-test",
            "-c",
            "user.email=skillnet@example.invalid",
            "commit",
            "-m",
            message,
        ])
        .current_dir(path)
        .output()
        .unwrap();
}

fn commit_paths(path: &Path, paths: &[&str], message: &str) {
    StdCommand::new("git")
        .arg("add")
        .args(paths)
        .current_dir(path)
        .output()
        .unwrap();
    StdCommand::new("git")
        .args([
            "-c",
            "user.name=skillnet-test",
            "-c",
            "user.email=skillnet@example.invalid",
            "commit",
            "-m",
            message,
        ])
        .current_dir(path)
        .output()
        .unwrap();
}

#[allow(dead_code)]
fn auto_commit_config(
    agents: &Path,
    claude: &Path,
    codex: &Path,
    enabled: bool,
    model: &str,
    effort: &str,
) -> String {
    format!(
        r#"
[sync]
auto_commit_dirty_destination = {enabled}
codex_model = "{model}"
codex_reasoning_effort = "{effort}"

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

#[allow(dead_code)]
fn install_codex_stub(
    fixture: &Fixture,
    script_body: &str,
    log_name: &str,
) -> (std::path::PathBuf, String) {
    let bin_dir = tempfile::tempdir().unwrap().keep();
    fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("codex");
    let log = fixture.path(log_name);
    let bash = env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("bash"))
        .find(|candidate| candidate.is_file())
        .expect("bash must be available on PATH to run codex stub");
    let rewritten = script_body
        .replace("#!/usr/bin/env bash", &format!("#!{}", bash.display()))
        .replace("__LOG__", &log.display().to_string());
    fs::write(&script, rewritten).unwrap();
    let mut perms = fs::metadata(&script).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }
    (log.clone(), prepend_path(&bin_dir))
}

#[allow(dead_code)]
fn prepend_path(bin_dir: &Path) -> String {
    let current = env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin_dir.to_path_buf()];
    paths.extend(env::split_paths(&current));
    env::join_paths(paths).unwrap().into_string().unwrap()
}

#[allow(dead_code)]
fn path_without_codex() -> String {
    let current = env::var_os("PATH").unwrap_or_default();
    let filtered = env::split_paths(&current)
        .filter(|dir| !dir.join("codex").exists())
        .collect::<Vec<_>>();
    env::join_paths(filtered).unwrap().into_string().unwrap()
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

struct SyncFixture {
    fixture: Fixture,
    config: std::path::PathBuf,
    catalog_config: std::path::PathBuf,
    global_view: std::path::PathBuf,
    first_project: std::path::PathBuf,
    second_project: std::path::PathBuf,
}

impl SyncFixture {
    fn new() -> Self {
        let fixture = Fixture::new();
        let global_view = fixture.path("views/global/agents");
        let first_project = fixture.path("work/first");
        let second_project = fixture.path("work/second");

        write_skill(&fixture.path("global"), "alpha", "global alpha");
        write_skill(&fixture.path("global"), "beta", "global beta");
        write_skill(
            &first_project.join(".agents/skills"),
            "alpha",
            "first alpha",
        );
        write_skill(&first_project.join(".agents/skills"), "beta", "first beta");
        write_skill(
            &second_project.join(".agents/skills"),
            "alpha",
            "second alpha",
        );
        write_skill(
            &second_project.join(".agents/skills"),
            "beta",
            "second beta",
        );

        init_git_repo(&first_project);
        commit_all(&first_project, "initial first project");
        init_git_repo(&second_project);
        commit_all(&second_project, "initial second project");

        let config = fixture.write_config(format!(
            r#"
[global]
views = [
  {{ label = "agents", path = "{}" }},
]

[[projects]]
name = "first"
path = "{}"

[[projects]]
name = "second"
path = "{}"
"#,
            global_view.display(),
            first_project.display(),
            second_project.display()
        ));
        let catalog_config = fixture.write_catalog_config("");

        Self {
            fixture,
            config,
            catalog_config,
            global_view,
            first_project,
            second_project,
        }
    }

    fn command(&self) -> Command {
        let command = self
            .fixture
            .command_with_catalog(&self.config, &self.catalog_config);
        assert!(
            command.get_program().to_string_lossy().contains("target/"),
            "assert_cmd did not resolve the test-profile binary: {}",
            command.get_program().to_string_lossy()
        );
        command
    }

    fn project_view(&self, project: &Path) -> std::path::PathBuf {
        project.join(".claude/skills")
    }

    fn aggregator(&self, project: &str) -> std::path::PathBuf {
        self.fixture.path(&format!("projects/{project}"))
    }
}

struct PromotionFixture {
    fixture: Fixture,
    config: std::path::PathBuf,
    catalog_config: std::path::PathBuf,
    canonical: std::path::PathBuf,
    view: std::path::PathBuf,
    view_mtime: FileTime,
    canonical_mtime: FileTime,
}

impl PromotionFixture {
    fn global_view_newer() -> Self {
        let fixture = Fixture::new();
        let canonical = fixture.path("global");
        let view = fixture.path("views/global/agents");
        write_skill(&canonical, "alpha", "canonical old");
        write_skill(&view, "alpha", "view new");
        let canonical_mtime = FileTime::from_unix_time(1_700_000_000, 0);
        let view_mtime = FileTime::from_unix_time(1_700_003_600, 0);
        set_skill_file_times(&canonical, "alpha", canonical_mtime);
        set_skill_file_times(&view, "alpha", view_mtime);
        let config = fixture.write_config(format!(
            r#"
[global]
views = [
  {{ label = "agents", path = "{}" }},
]
"#,
            view.display()
        ));
        let catalog_config = fixture.write_catalog_config("");

        Self {
            fixture,
            config,
            catalog_config,
            canonical,
            view,
            view_mtime,
            canonical_mtime,
        }
    }

    fn command(&self) -> Command {
        self.fixture
            .command_with_catalog(&self.config, &self.catalog_config)
    }
}

fn set_skill_file_times(root: &Path, name: &str, time: FileTime) {
    let path = root.join(name).join("SKILL.md");
    set_file_times(&path, time, time).unwrap();
}

fn file_mtime(path: &Path) -> FileTime {
    FileTime::from_last_modification_time(&fs::metadata(path).unwrap())
}

fn assert_symlink_points_to(link: &Path, target: &Path) {
    assert!(
        fs::symlink_metadata(link).unwrap().file_type().is_symlink(),
        "{} should be a symlink",
        link.display()
    );
    assert_eq!(fs::read_link(link).unwrap(), target);
}

fn assert_relative_symlink_points_to(link: &Path, target: &str) {
    assert!(
        fs::symlink_metadata(link).unwrap().file_type().is_symlink(),
        "{} should be a symlink",
        link.display()
    );
    assert_eq!(fs::read_link(link).unwrap(), Path::new(target));
}

fn tree_digest(root: &Path) -> String {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.unwrap();
        let path = entry.path();
        let metadata = fs::symlink_metadata(path).unwrap();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy();
        let mut hasher = Sha256::new();
        hasher.update(rel.as_bytes());
        if metadata.file_type().is_symlink() {
            hasher.update(b"\0symlink\0");
            hasher.update(fs::read_link(path).unwrap().to_string_lossy().as_bytes());
        } else if metadata.is_dir() {
            hasher.update(b"\0dir");
        } else if metadata.is_file() {
            hasher.update(b"\0file\0");
            let mut file = fs::File::open(path).unwrap();
            let mut buf = [0; 8192];
            loop {
                let n = file.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        entries.push(format!("{:x}", hasher.finalize()));
    }
    entries.sort();

    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(entry.as_bytes());
        digest.update(b"\n");
    }
    format!("{:x}", digest.finalize())
}

#[test]
fn sync_creates_global_and_project_symlinks() {
    let sync = SyncFixture::new();

    sync.command().arg("sync").assert().success().stdout(
        predicate::str::contains("agents  ")
            .and(predicate::str::contains("first:claude"))
            .and(predicate::str::contains("second:claude")),
    );

    assert_symlink_points_to(
        &sync.global_view.join("alpha"),
        &sync.fixture.path("global/alpha"),
    );
    assert_symlink_points_to(
        &sync.global_view.join("beta"),
        &sync.fixture.path("global/beta"),
    );
    for (name, project) in [
        ("first", &sync.first_project),
        ("second", &sync.second_project),
    ] {
        let view = sync.project_view(project);
        assert_relative_symlink_points_to(&view.join("alpha"), "../../.agents/skills/alpha");
        assert_relative_symlink_points_to(&view.join("beta"), "../../.agents/skills/beta");
        assert!(sync.aggregator(name).exists());
    }
}

#[test]
fn sync_scope_projects_materializes_only_project_targets() {
    let sync = SyncFixture::new();

    sync.command()
        .args(["sync", "--scope", "projects"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("first:claude")
                .and(predicate::str::contains("second:claude"))
                .and(predicate::str::contains("global:agents").not()),
        );

    assert!(!sync.global_view.exists());
    for (name, project) in [
        ("first", &sync.first_project),
        ("second", &sync.second_project),
    ] {
        let view = sync.project_view(project);
        assert_relative_symlink_points_to(&view.join("alpha"), "../../.agents/skills/alpha");
        assert!(sync.aggregator(name).exists());
    }
}

#[test]
fn sync_scope_global_materializes_only_global_target() {
    let sync = SyncFixture::new();

    sync.command()
        .args(["sync", "--scope", "global"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("global:agents")
                .and(predicate::str::contains("first:claude").not())
                .and(predicate::str::contains("second:claude").not()),
        );

    assert_symlink_points_to(
        &sync.global_view.join("alpha"),
        &sync.fixture.path("global/alpha"),
    );
    assert!(!sync.project_view(&sync.first_project).exists());
    assert!(!sync.project_view(&sync.second_project).exists());
    assert!(!sync.aggregator("first").exists());
    assert!(!sync.aggregator("second").exists());
}

#[test]
fn status_accepts_scope_projects_and_all_selectors() {
    let sync = SyncFixture::new();

    sync.command()
        .args(["status", "--scope", "projects"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("scopes: 2 (first, second)")
                .and(predicate::str::contains("global  ").not()),
        );

    sync.command()
        .args(["status", "--scope", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "scopes: 3 (global, first, second)",
        ));
}

#[test]
fn status_warns_once_for_legacy_default_project_layout() {
    let fixture = Fixture::new();
    let project = fixture.path("work/demo");
    write_skill(&project.join(".skills"), "alpha", "legacy alpha");
    let config = fixture.write_config(format!(
        r#"
[global]
views = []

[[projects]]
name = "demo"
path = "{}"
"#,
        project.display()
    ));

    fixture
        .command(&config)
        .args(["status", "--scope", "demo"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("legacy '.skills' canonical detected")
                .count(1)
                .and(predicate::str::contains(
                    "Set canonical_rel = \".skills\" to keep the old layout",
                )),
        );

    assert!(project.join(".skills/alpha/SKILL.md").is_file());
    assert!(!project.join(".agents/skills").exists());
}

#[test]
fn sync_forwards_allow_delete_and_force() {
    let sync = SyncFixture::new();
    sync.command().arg("sync").assert().success();

    symlink_file("/missing/global", &sync.global_view.join("stale"));
    symlink_file(
        "/missing/first",
        &sync.project_view(&sync.first_project).join("stale"),
    );

    sync.command().arg("sync").assert().success();
    assert!(fs::symlink_metadata(sync.global_view.join("stale")).is_ok());
    assert!(fs::symlink_metadata(sync.project_view(&sync.first_project).join("stale")).is_ok());

    sync.command()
        .args(["sync", "--allow-delete"])
        .assert()
        .success();
    assert!(fs::symlink_metadata(sync.global_view.join("stale")).is_err());
    assert!(fs::symlink_metadata(sync.project_view(&sync.first_project).join("stale")).is_err());

    let sync = SyncFixture::new();
    fs::create_dir_all(&sync.global_view).unwrap();
    fs::write(sync.global_view.join("alpha"), "not a symlink").unwrap();
    fs::create_dir_all(sync.project_view(&sync.first_project)).unwrap();
    fs::write(
        sync.project_view(&sync.first_project).join("alpha"),
        "not a symlink",
    )
    .unwrap();

    sync.command()
        .args(["sync", "--no-promote"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("exists and is not a symlink")
                .and(predicate::str::contains("pass --force")),
        );

    fs::remove_file(sync.global_view.join("alpha")).unwrap();
    fs::remove_file(sync.project_view(&sync.first_project).join("alpha")).unwrap();
    write_skill(&sync.global_view, "alpha", "old view alpha");
    write_skill(
        &sync.project_view(&sync.first_project),
        "alpha",
        "old view alpha",
    );
    set_skill_file_times(
        &sync.global_view,
        "alpha",
        FileTime::from_unix_time(1_700_000_000, 0),
    );
    set_skill_file_times(
        &sync.fixture.path("global"),
        "alpha",
        FileTime::from_unix_time(1_700_003_600, 0),
    );
    set_skill_file_times(
        &sync.project_view(&sync.first_project),
        "alpha",
        FileTime::from_unix_time(1_700_000_000, 0),
    );
    set_skill_file_times(
        &sync.first_project.join(".agents/skills"),
        "alpha",
        FileTime::from_unix_time(1_700_003_600, 0),
    );

    sync.command().args(["sync", "--force"]).assert().success();
    assert_symlink_points_to(
        &sync.global_view.join("alpha"),
        &sync.fixture.path("global/alpha"),
    );
    assert_relative_symlink_points_to(
        &sync.project_view(&sync.first_project).join("alpha"),
        "../../.agents/skills/alpha",
    );
}

#[test]
fn sync_dry_run_does_not_mutate() {
    let sync = SyncFixture::new();
    let before = tree_digest(sync.fixture.root());

    sync.command()
        .args(["--dry-run", "sync"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("# view sync global")
                .and(predicate::str::contains("# project sync first"))
                .and(predicate::str::contains("# project sync second"))
                .and(predicate::str::contains("allow_delete: false"))
                .and(predicate::str::contains("force: false")),
        );

    assert_eq!(tree_digest(sync.fixture.root()), before);
    assert!(!sync.global_view.exists());
    assert!(!sync.project_view(&sync.first_project).exists());
    assert!(!sync.aggregator("first").exists());
}

#[test]
fn view_sync_adopts_loose_dir_instead_of_wiping() {
    let sync = SyncFixture::new();
    // An authored skill that lives only in the view, not yet in canonical.
    write_skill(&sync.global_view, "orphan", "orphan body");

    // `--allow-delete` used to WIPE loose view dirs; it must now adopt them.
    sync.command()
        .args(["view", "sync", "--scope", "global", "--allow-delete"])
        .assert()
        .success();

    // Adopted into canonical with content preserved (not wiped).
    assert_eq!(
        fs::read_to_string(sync.fixture.path("global/orphan/SKILL.md")).unwrap(),
        "orphan body"
    );
    // The loose view entry survived the sync.
    assert!(fs::symlink_metadata(sync.global_view.join("orphan")).is_ok());
    // Canonical skills were materialised as symlinks in the view.
    assert!(fs::symlink_metadata(sync.global_view.join("alpha"))
        .unwrap()
        .file_type()
        .is_symlink());

    // A second pass demotes the adopted dir to a symlink (two-phase convergence).
    sync.command()
        .args(["view", "sync", "--scope", "global"])
        .assert()
        .success();
    assert!(fs::symlink_metadata(sync.global_view.join("orphan"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn sync_short_circuits_on_view_failure() {
    let sync = SyncFixture::new();
    fs::create_dir_all(&sync.global_view).unwrap();
    fs::write(sync.global_view.join("alpha"), "not a symlink").unwrap();
    let first_project_before = tree_digest(&sync.first_project);
    let second_project_before = tree_digest(&sync.second_project);

    sync.command()
        .args(["sync", "--no-promote"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("failed to sync skill `alpha`")
                .and(predicate::str::contains("exists and is not a symlink"))
                .and(predicate::str::contains("pass --force"))
                .and(predicate::str::contains("# first").not())
                .and(predicate::str::contains("# second").not()),
        );

    assert_eq!(tree_digest(&sync.first_project), first_project_before);
    assert_eq!(tree_digest(&sync.second_project), second_project_before);
    assert!(!sync.project_view(&sync.first_project).exists());
    assert!(!sync.project_view(&sync.second_project).exists());
    assert!(!sync.aggregator("first").exists());
    assert!(!sync.aggregator("second").exists());
}

#[test]
fn sync_help_lists_command() {
    let mut command = Command::cargo_bin("skillnet").unwrap();
    command.arg("--help").assert().success().stdout(
        predicate::str::contains("sync")
            .and(predicate::str::contains("global view and project view")),
    );

    let mut command = Command::cargo_bin("skillnet").unwrap();
    command.args(["sync", "--help"]).assert().success().stdout(
        predicate::str::contains("--allow-delete")
            .and(predicate::str::contains("--force"))
            .and(predicate::str::contains("--apply-promote"))
            .and(predicate::str::contains("--no-promote"))
            .and(predicate::str::contains("--prefer"))
            .and(predicate::str::contains("--adopt-new"))
            .and(predicate::str::contains("--scope"))
            .and(predicate::str::contains("--dry-run"))
            .and(predicate::str::contains("--mirror-root")),
    );
}

#[test]
fn sync_help_output_matches_snapshot() {
    // clap inlines `[env: VAR=<value>]` in --help; isolate the env so the
    // snapshot is stable across shells that export skillnet's session vars
    // (the HM module sets SKILLNET_MIRROR_ROOT, for example).
    let output = Command::cargo_bin("skillnet")
        .unwrap()
        .env_remove("SKILLNET_CONFIG")
        .env_remove("SKILLNET_CATALOG_CONFIG")
        .env_remove("SKILLNET_MIRROR_ROOT")
        .env_remove("SKILLNET_DATABASE_URL")
        .env_remove("SKILLNET_DB_URL")
        .env_remove("DATABASE_URL")
        .args(["sync", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(output).unwrap();
    insta::assert_snapshot!(help);
}

#[test]
fn sync_default_on_view_newer_fixture_exits_2_and_does_not_mutate() {
    let promotion = PromotionFixture::global_view_newer();

    promotion
        .command()
        .arg("sync")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("would promote"));

    assert!(fs::symlink_metadata(promotion.view.join("alpha"))
        .unwrap()
        .is_dir());
    assert_eq!(read_skill(&promotion.canonical, "alpha"), "canonical old");
    assert_eq!(
        file_mtime(&promotion.canonical.join("alpha/SKILL.md")),
        promotion.canonical_mtime
    );
}

#[test]
fn sync_apply_promote_on_view_newer_fixture_succeeds() {
    let promotion = PromotionFixture::global_view_newer();

    promotion
        .command()
        .args(["sync", "--apply-promote"])
        .assert()
        .success();

    assert_symlink_points_to(
        &promotion.view.join("alpha"),
        &promotion.canonical.join("alpha"),
    );
    assert_eq!(read_skill(&promotion.canonical, "alpha"), "view new");
    assert_eq!(
        file_mtime(&promotion.canonical.join("alpha/SKILL.md")),
        promotion.view_mtime
    );
}

#[test]
fn sync_no_promote_on_view_newer_fixture_errors() {
    let promotion = PromotionFixture::global_view_newer();

    promotion
        .command()
        .args(["sync", "--no-promote"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("exists and is not a symlink"));

    assert_eq!(read_skill(&promotion.canonical, "alpha"), "canonical old");
    assert_eq!(
        file_mtime(&promotion.canonical.join("alpha/SKILL.md")),
        promotion.canonical_mtime
    );
}

#[test]
fn sync_dry_run_collapses_code_2_to_code_0() {
    let promotion = PromotionFixture::global_view_newer();

    promotion
        .command()
        .args(["--dry-run", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("would promote"));

    assert!(fs::symlink_metadata(promotion.view.join("alpha"))
        .unwrap()
        .is_dir());
    assert_eq!(read_skill(&promotion.canonical, "alpha"), "canonical old");
}

#[test]
fn sync_per_project_dirty_gate_refuses_promotion() {
    let fixture = Fixture::new();
    let project = fixture.path("work/demo");
    write_skill(&project.join(".skills"), "alpha", "canonical old");
    write_skill(&project.join(".claude/skills"), "alpha", "view new");
    set_skill_file_times(
        &project.join(".skills"),
        "alpha",
        FileTime::from_unix_time(1_700_000_000, 0),
    );
    set_skill_file_times(
        &project.join(".claude/skills"),
        "alpha",
        FileTime::from_unix_time(1_700_003_600, 0),
    );
    init_git_repo(&project);
    commit_all(&project, "baseline");
    fs::write(project.join("dirty"), "dirty").unwrap();
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&project)
        .output()
        .unwrap();
    let config = fixture.write_config(project_config("demo", &project));
    let catalog_config = fixture.write_catalog_config("");

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["sync", "--apply-promote"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(project.to_string_lossy().as_ref()));
}

#[test]
fn sync_with_allow_dirty_destination_bypasses_per_project_gate() {
    let fixture = Fixture::new();
    let project = fixture.path("work/demo");
    write_skill(&project.join(".skills"), "alpha", "canonical old");
    write_skill(&project.join(".claude/skills"), "alpha", "view new");
    set_skill_file_times(
        &project.join(".skills"),
        "alpha",
        FileTime::from_unix_time(1_700_000_000, 0),
    );
    set_skill_file_times(
        &project.join(".claude/skills"),
        "alpha",
        FileTime::from_unix_time(1_700_003_600, 0),
    );
    init_git_repo(&project);
    commit_all(&project, "baseline");
    fs::write(project.join("dirty"), "dirty").unwrap();
    StdCommand::new("git")
        .args(["add", "."])
        .current_dir(&project)
        .output()
        .unwrap();
    let config = fixture.write_config(project_config("demo", &project));
    let catalog_config = fixture.write_catalog_config("");

    fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["--allow-dirty-destination", "sync", "--apply-promote"])
        .assert()
        .success();
}

#[test]
fn status_json_emits_would_promote_and_tie_break_counts() {
    let fixture = Fixture::new();
    let canonical = fixture.path("global");
    let view = fixture.path("views/global/agents");
    write_skill(&canonical, "alpha", "canonical alpha");
    write_skill(&view, "alpha", "view alpha");
    set_skill_file_times(
        &canonical,
        "alpha",
        FileTime::from_unix_time(1_700_000_000, 0),
    );
    set_skill_file_times(&view, "alpha", FileTime::from_unix_time(1_700_003_600, 0));
    write_skill(&canonical, "beta", "canonical beta");
    write_skill(&view, "beta", "view beta");
    let same_time = FileTime::from_unix_time(1_700_010_000, 0);
    set_skill_file_times(&canonical, "beta", same_time);
    set_skill_file_times(&view, "beta", same_time);
    let config = fixture.write_config(format!(
        r#"
[global]
views = [
  {{ label = "agents", path = "{}" }},
]
"#,
        view.display()
    ));
    let catalog_config = fixture.write_catalog_config("");

    let output = fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["status", "--all", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let global = value
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["scope"] == "global")
        .unwrap();

    assert_eq!(global["would_promote"], 1);
    assert_eq!(global["needs_tie_break"], 1);
    let drift = global["drift"].as_array().unwrap();
    let alpha = drift
        .iter()
        .find(|entry| entry["skill"] == "alpha")
        .unwrap();
    assert!(alpha["view_mtime_nanos"].as_u64().is_some());
    assert!(alpha["canonical_mtime_nanos"].as_u64().is_some());
    let beta = drift.iter().find(|entry| entry["skill"] == "beta").unwrap();
    assert!(beta["view_sha"].as_str().unwrap().len() >= 8);
    assert!(beta["canonical_sha"].as_str().unwrap().len() >= 8);
}

#[test]
fn status_json_leaves_legacy_fields_unchanged() {
    let fixture = Fixture::new();
    let canonical = fixture.path("global");
    let view = fixture.path("views/global/agents");
    write_skill(&canonical, "missing", "missing");
    write_skill(&canonical, "linked", "linked");
    fs::create_dir_all(&view).unwrap();
    symlink_file("/missing/target", &view.join("linked"));
    write_skill(&view, "stale", "stale");
    let config = fixture.write_config(format!(
        r#"
[global]
views = [
  {{ label = "agents", path = "{}" }},
]
"#,
        view.display()
    ));
    let catalog_config = fixture.write_catalog_config("");

    let output = fixture
        .command_with_catalog(&config, &catalog_config)
        .args(["status", "--all", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let global = value
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["scope"] == "global")
        .unwrap();

    assert_eq!(global["would_promote"], 0);
    assert_eq!(global["needs_tie_break"], 0);
    for entry in global["drift"].as_array().unwrap() {
        assert!(entry["view_mtime_nanos"].is_null());
        assert!(entry["canonical_mtime_nanos"].is_null());
        assert!(entry["view_sha"].is_null());
        assert!(entry["canonical_sha"].is_null());
    }
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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
        .args([
            "sync",
            "pull",
            "--scope",
            "global",
            "--then-push",
            "--allow-delete",
        ])
        .assert()
        .success();

    assert!(agents.join("alpha/SKILL.md").is_file());
    assert!(claude.join("alpha/SKILL.md").is_file());
    assert!(!codex.exists());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_skips_older_source_by_default() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "source old");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all", "--allow-delete"])
        .assert()
        .success();
    wait_for_mtime_tick();
    write_skill(&fixture.path("global"), "alpha", "mirror new");
    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped 1 older"));

    assert_eq!(read_skill(&fixture.path("global"), "alpha"), "mirror new");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_overwrites_older_mirror_when_source_is_newer() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "source old");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success();
    wait_for_mtime_tick();
    write_skill(&source, "alpha", "source new");
    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success();

    assert_eq!(read_skill(&fixture.path("global"), "alpha"), "source new");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_equal_mtime_conflict_fails_without_allow_older() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "source");
    write_skill(&fixture.path("global"), "alpha", "mirror");
    let same_time = FileTime::from_unix_time(1_700_000_000, 0);
    set_skill_file_mtime(&source, "alpha", same_time);
    set_skill_file_mtime(&fixture.path("global"), "alpha", same_time);
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("equal-mtime conflicting skill"));
    fixture
        .command(&config)
        .args(["sync", "pull", "--all", "--allow-older"])
        .assert()
        .success();

    assert_eq!(read_skill(&fixture.path("global"), "alpha"), "source");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_preserves_mirror_only_skill_by_default() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    fs::create_dir_all(&source).unwrap();
    write_skill(&fixture.path("global"), "alpha", "mirror only");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preserved 1 missing"));

    assert_eq!(read_skill(&fixture.path("global"), "alpha"), "mirror only");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn roundtrip_command_matches_pull_then_push() {
    let roundtrip = Fixture::new();
    let pull_then_push = Fixture::new();
    let rt_agents = roundtrip.path("home/.agents/skills");
    let rt_claude = roundtrip.path("home/.claude/skills");
    let rt_source = roundtrip.path("home/source/skills");
    let ptp_agents = pull_then_push.path("home/.agents/skills");
    let ptp_claude = pull_then_push.path("home/.claude/skills");
    let ptp_source = pull_then_push.path("home/source/skills");
    for source in [&rt_source, &ptp_source] {
        write_skill_with_files(
            source,
            "alpha",
            &[
                ("SKILL.md", "alpha"),
                ("examples/example.md", "alpha example"),
            ],
        );
        write_skill(source, "beta", "beta");
    }
    for destination in [&rt_agents, &rt_claude, &ptp_agents, &ptp_claude] {
        fs::create_dir_all(destination).unwrap();
    }
    let rt_config = roundtrip.write_config(sync_paths_config(
        &[("source", &rt_source, 1)],
        &[&rt_agents, &rt_claude],
        &[],
    ));
    let ptp_config = pull_then_push.write_config(sync_paths_config(
        &[("source", &ptp_source, 1)],
        &[&ptp_agents, &ptp_claude],
        &[],
    ));

    roundtrip
        .command(&rt_config)
        .args(["sync", "roundtrip", "--all"])
        .assert()
        .success();
    pull_then_push
        .command(&ptp_config)
        .args(["sync", "pull", "--all", "--then-push"])
        .assert()
        .success();

    let mut roundtrip_mirror = sync_path_entries(&roundtrip.path("global"));
    let mut pull_then_push_mirror = sync_path_entries(&pull_then_push.path("global"));
    roundtrip_mirror.remove("RECONCILIATION.md");
    pull_then_push_mirror.remove("RECONCILIATION.md");
    assert_eq!(roundtrip_mirror, pull_then_push_mirror);
    assert_sync_paths_parity(&[&rt_agents, &rt_claude, &ptp_agents, &ptp_claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn roundtrip_check_exits_zero_when_destinations_are_in_sync() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all"])
        .assert()
        .success();
    let before = assert_sync_paths_parity(&[&agents, &claude]);
    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all", "--check", "--allow-older"])
        .assert()
        .success();
    let after = assert_sync_paths_parity(&[&agents, &claude]);

    assert_eq!(before, after);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn roundtrip_check_exits_nonzero_when_destinations_would_change() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all"])
        .assert()
        .success();
    fs::write(agents.join("alpha/SKILL.md"), "changed").unwrap();
    let before = sync_path_entries(&agents);
    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all", "--check", "--allow-older"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("would change"));
    let after = sync_path_entries(&agents);

    assert_eq!(before, after);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn roundtrip_on_pull_failure_does_not_push() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let bad_source = fixture.path("home/source-file");
    fs::create_dir_all(bad_source.parent().unwrap()).unwrap();
    fs::write(&bad_source, "not a directory").unwrap();
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &bad_source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all"])
        .assert()
        .code(2);

    assert!(sync_path_entries(&agents).is_empty());
    assert!(sync_path_entries(&claude).is_empty());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_emits_deprecation_note() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all", "--then-push"])
        .assert()
        .success()
        .stderr(predicate::str::contains("deprecated"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn push_summary_lists_every_destination() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all", "--allow-delete"])
        .assert()
        .success();
    fixture
        .command(&config)
        .args(["sync", "push", "--all", "--allow-delete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("synced global →"))
        .stdout(predicate::str::contains(agents.display().to_string()))
        .stdout(predicate::str::contains(claude.display().to_string()))
        .stdout(predicate::str::contains("(+"))
        .stdout(predicate::str::contains("~"))
        .stdout(predicate::str::contains("-"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn push_summary_counts_match_actual_changes() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha v1");
    write_skill(&source, "beta", "beta");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "roundtrip", "--all"])
        .assert()
        .success();
    fs::write(source.join("alpha/SKILL.md"), "alpha v2").unwrap();
    fs::remove_dir_all(source.join("beta")).unwrap();
    write_skill(&source, "gamma", "gamma");
    fixture
        .command(&config)
        .args(["sync", "pull", "--all", "--allow-delete"])
        .assert()
        .success();

    fixture
        .command(&config)
        .args(["sync", "push", "--all", "--allow-delete"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "{} (+1 ~1 -1)",
            agents.display()
        )))
        .stdout(predicate::str::contains(format!(
            "{} (+1 ~1 -1)",
            claude.display()
        )));

    assert_sync_paths_parity(&[&agents, &claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn push_summary_is_omitted_in_dry_run() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let source = fixture.path("home/source/skills");
    write_skill(&source, "alpha", "alpha");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--all"])
        .assert()
        .success();
    fixture
        .command(&config)
        .args(["--dry-run", "sync", "push", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("# sync global"))
        .stdout(predicate::str::contains("from:"))
        .stdout(predicate::str::contains("to:"))
        .stdout(predicate::str::contains("(+").not());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_push_skips_older_mirror_by_default() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    write_skill(&fixture.path("global"), "alpha", "mirror old");
    fs::create_dir_all(&claude).unwrap();
    wait_for_mtime_tick();
    write_skill(&agents, "alpha", "destination new");
    let config = fixture.write_config(sync_paths_config(&[], &[&agents, &claude], &[]));

    fixture
        .command(&config)
        .args(["sync", "push", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped 1 older"));

    assert_eq!(read_skill(&agents, "alpha"), "destination new");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_push_overwrites_older_destination_when_mirror_is_newer() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    write_skill(&agents, "alpha", "destination old");
    fs::create_dir_all(&claude).unwrap();
    wait_for_mtime_tick();
    write_skill(&fixture.path("global"), "alpha", "mirror new");
    let config = fixture.write_config(sync_paths_config(&[], &[&agents, &claude], &[]));

    fixture
        .command(&config)
        .args(["sync", "push", "--all"])
        .assert()
        .success();

    assert_eq!(read_skill(&agents, "alpha"), "mirror new");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_push_preserves_destination_only_skill_by_default() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    fs::create_dir_all(fixture.path("global")).unwrap();
    write_skill(&agents, "alpha", "destination only");
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(&[], &[&agents, &claude], &[]));

    fixture
        .command(&config)
        .args(["sync", "push", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("preserved 1 missing"));

    assert_eq!(read_skill(&agents, "alpha"), "destination only");
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_push_allow_delete_prunes_destination_only_skill() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    fs::create_dir_all(fixture.path("global")).unwrap();
    write_skill(&agents, "alpha", "destination only");
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(&[], &[&agents, &claude], &[]));

    fixture
        .command(&config)
        .args(["sync", "push", "--all", "--allow-delete"])
        .assert()
        .success();

    assert!(!agents.join("alpha").exists());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_leaves_agents_and_claude_byte_identical() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill_with_files(
        &codex,
        "alpha",
        &[
            ("SKILL.md", "alpha"),
            ("examples/example.md", "alpha example"),
        ],
    );
    write_skill_with_files(
        &codex,
        "beta",
        &[
            ("SKILL.md", "beta"),
            ("examples/example.md", "beta example"),
        ],
    );
    symlink_file("SKILL.md", &codex.join("alpha/skill-link.md"));
    symlink_file("SKILL.md", &codex.join("beta/skill-link.md"));
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &codex, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args([
            "sync",
            "pull",
            "--scope",
            "global",
            "--then-push",
            "--allow-delete",
        ])
        .assert()
        .success();

    assert_sync_paths_parity(&[&agents, &claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn removing_a_skill_from_all_sources_removes_it_from_both_destinations() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&codex, "alpha", "a");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &codex, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args([
            "sync",
            "pull",
            "--scope",
            "global",
            "--then-push",
            "--allow-delete",
        ])
        .assert()
        .success();
    fs::remove_dir_all(&codex).unwrap();
    fixture
        .command(&config)
        .args([
            "sync",
            "pull",
            "--scope",
            "global",
            "--then-push",
            "--allow-delete",
        ])
        .assert()
        .success();

    assert!(!agents.join("alpha").exists());
    assert!(!claude.join("alpha").exists());
    assert_sync_paths_parity(&[&agents, &claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_is_idempotent_in_content() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill_with_files(
        &codex,
        "alpha",
        &[
            ("SKILL.md", "alpha"),
            ("examples/example.md", "alpha example"),
        ],
    );
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();
    let first = assert_sync_paths_parity(&[&agents, &claude]);
    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();
    let second = assert_sync_paths_parity(&[&agents, &claude]);

    assert_eq!(first, second);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_with_three_sync_paths_keeps_all_three_in_parity() {
    let fixture = Fixture::new();
    let a = fixture.path("home/a/skills");
    let b = fixture.path("home/b/skills");
    let c = fixture.path("home/c/skills");
    let source = fixture.path("home/source/skills");
    write_skill_with_files(
        &source,
        "alpha",
        &[
            ("SKILL.md", "alpha"),
            ("examples/example.md", "alpha example"),
        ],
    );
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&c).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("source", &source, 1)],
        &[&a, &b, &c],
        &[],
    ));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();

    assert_sync_paths_parity(&[&a, &b, &c]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_handles_skill_with_nested_subdirs() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill_with_files(
        &codex,
        "alpha",
        &[
            ("SKILL.md", "alpha"),
            ("subdir/SKILL.md", "nested skill-shaped file"),
            ("subdir/deeper/example.md", "nested example"),
        ],
    );
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();

    assert!(agents.join("alpha/subdir/SKILL.md").is_file());
    assert!(claude.join("alpha/subdir/SKILL.md").is_file());
    assert_sync_paths_parity(&[&agents, &claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_then_push_with_symlinked_file_keeps_symlink_in_both_destinations() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&codex, "alpha", "a");
    symlink_file("SKILL.md", &codex.join("alpha/linked.md"));
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));

    fixture
        .command(&config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .success();

    assert!(fs::symlink_metadata(agents.join("alpha/linked.md"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(claude.join("alpha/linked.md"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_sync_paths_parity(&[&agents, &claude]);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn pull_failure_does_not_perturb_existing_destination_content() {
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
    let before = assert_sync_paths_parity(&[&agents, &claude]);
    let bad_config = fixture.write_config("not = [valid");
    fixture
        .command(&bad_config)
        .args(["sync", "pull", "--scope", "global", "--then-push"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to parse config file"));
    let after = assert_sync_paths_parity(&[&agents, &claude]);

    assert_eq!(before, after);
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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
fn project_add_refuses_when_config_under_nix_store() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());
    let Ok(output) = StdCommand::new("nix-store")
        .args(["--add-fixed", "sha256", config.to_str().unwrap()])
        .output()
    else {
        eprintln!("skipping: nix-store unavailable");
        return;
    };
    if !output.status.success() {
        eprintln!(
            "skipping: nix-store --add-fixed failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let store_config = String::from_utf8(output.stdout).unwrap();
    let store_config = store_config.trim();
    assert!(
        store_config.starts_with("/nix/store/"),
        "unexpected nix-store path: {store_config}"
    );

    fixture
        .command(Path::new(store_config))
        .args([
            "project",
            "add",
            "future-project",
            "/tmp/future-project",
            "--allow-missing",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("skillnet.toml at /nix/store/"))
        .stderr(predicate::str::contains(
            "is managed by Home Manager (read-only)",
        ))
        .stderr(predicate::str::contains(
            "hint: edit programs.skillnet.settings in your Home Manager configuration",
        ))
        .stderr(predicate::str::contains("then run `home-manager switch`."));
}

#[test]
fn project_add_rejects_selector_tokens_as_reserved_names() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());

    for name in ["all", "projects"] {
        fixture
            .command(&config)
            .args(["project", "add", name, "/tmp/x", "--allow-missing"])
            .assert()
            .failure()
            .stderr(predicate::str::contains(format!(
                "`{name}` is reserved and cannot be used as a project name"
            )));
    }
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn no_args_requires_scope_outside_project() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());

    fixture
        .command(&config)
        .assert()
        .failure()
        .stderr(predicate::str::contains("must pass --scope or --all"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn status_reports_git_destination_from_skills_root() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let config = fixture.write_config(minimal_config_with_skills_root(fixture.root()));

    fixture
        .raw_command(&config)
        .args(["status", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("destination:"))
        .stdout(predicate::str::contains("git"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn auto_detects_scope_from_cwd_inside_registered_project() {
    let fixture = Fixture::new();
    let project_root = fixture.path("repos/demo");
    fs::create_dir_all(&project_root).unwrap();
    let config = fixture.write_config(project_config("demo", &project_root));

    fixture
        .command(&config)
        .current_dir(&project_root)
        .args(["sync", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("demo"))
        .stdout(predicate::str::contains("global"))
        .stderr(predicate::str::contains(
            "note: defaulting to --scope demo --scope global (detected from cwd)",
        ));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn auto_detect_picks_deepest_project_on_nested_match() {
    let fixture = Fixture::new();
    let outer = fixture.path("repos/outer");
    let inner = outer.join("inner");
    fs::create_dir_all(&inner).unwrap();
    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[projects]]
name = "outer"
path = "{}"

[[projects]]
name = "inner"
path = "{}"
"#,
        outer.display(),
        inner.display()
    ));

    fixture
        .command(&config)
        .current_dir(&inner)
        .args(["sync", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("inner"))
        .stdout(predicate::str::contains("outer").not())
        .stderr(predicate::str::contains(
            "note: defaulting to --scope inner --scope global (detected from cwd)",
        ));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn auto_detect_falls_back_to_error_outside_any_project() {
    let fixture = Fixture::new();
    let project_root = fixture.path("repos/demo");
    let outside = fixture.path("outside");
    fs::create_dir_all(&project_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let config = fixture.write_config(project_config("demo", &project_root));

    fixture
        .command(&config)
        .current_dir(&outside)
        .args(["sync", "status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("must pass --scope or --all"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commits_selected_scope_dirty_paths_before_pulling() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(format!(
        r#"
[sync]
auto_commit_dirty_destination = true

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
    ));
    let (log, path) = install_codex_stub(
        &fixture,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "__LOG__"
repo=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-C" ]; then
    repo="$2"
    shift 2
    continue
  fi
  shift
done
git -C "$repo" add -A
git -C "$repo" -c user.name=skillnet-test -c user.email=skillnet@example.invalid commit -m "codex auto commit" >/dev/null
"#,
        "codex-default.log",
    );
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    commit_paths(
        fixture.root(),
        &["home", "skillnet.toml", "stub-bin"],
        "test baseline",
    );

    fixture
        .command(&config)
        .env("PATH", path)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .success();

    assert!(fixture.path("global/alpha/SKILL.md").is_file());
    let logged = fs::read_to_string(log).unwrap();
    assert!(logged.contains("gpt-5.4-mini"));
    assert!(logged.contains(r#"model_reasoning_effort="medium""#));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commit_cli_overrides_model_and_effort() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(global_config(&agents, &claude, &codex));
    let (log, path) = install_codex_stub(
        &fixture,
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "__LOG__"
repo=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-C" ]; then
    repo="$2"
    shift 2
    continue
  fi
  shift
done
git -C "$repo" add -A
git -C "$repo" -c user.name=skillnet-test -c user.email=skillnet@example.invalid commit -m "codex auto commit" >/dev/null
"#,
        "codex-override.log",
    );
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    commit_paths(
        fixture.root(),
        &["home", "skillnet.toml", "stub-bin"],
        "test baseline",
    );

    fixture
        .command(&config)
        .env("PATH", path)
        .args([
            "sync",
            "pull",
            "--scope",
            "global",
            "--auto-commit-dirty-destination",
            "--codex-model",
            "gpt-5.3-codex",
            "--codex-reasoning-effort",
            "high",
        ])
        .assert()
        .success();

    let logged = fs::read_to_string(log).unwrap();
    assert!(logged.contains("gpt-5.3-codex"));
    assert!(logged.contains(r#"model_reasoning_effort="high""#));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commit_rejects_unrelated_dirty_paths() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(auto_commit_config(
        &agents,
        &claude,
        &codex,
        true,
        "gpt-5.4-mini",
        "medium",
    ));
    let (log, path) = install_codex_stub(
        &fixture,
        "#!/usr/bin/env bash\nexit 0\n",
        "codex-blocked.log",
    );
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    fs::write(fixture.path("unrelated.txt"), "nope").unwrap();

    fixture
        .command(&config)
        .env("PATH", path)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "dirty entries outside the selected skillnet-managed paths",
        ))
        .stderr(predicate::str::contains("unrelated.txt"));

    assert!(!log.exists());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commit_requires_codex_on_path() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(auto_commit_config(
        &agents,
        &claude,
        &codex,
        true,
        "gpt-5.4-mini",
        "medium",
    ));
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    commit_paths(fixture.root(), &["home", "skillnet.toml"], "test baseline");

    fixture
        .command(&config)
        .env("PATH", path_without_codex())
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires `codex` on PATH"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commit_bubbles_codex_failure() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(auto_commit_config(
        &agents,
        &claude,
        &codex,
        true,
        "gpt-5.4-mini",
        "medium",
    ));
    let (_log, path) = install_codex_stub(
        &fixture,
        "#!/usr/bin/env bash\necho stub failure >&2\nexit 17\n",
        "codex-fail.log",
    );
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    commit_paths(
        fixture.root(),
        &["home", "skillnet.toml", "stub-bin"],
        "test baseline",
    );

    fixture
        .command(&config)
        .env("PATH", path)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Codex auto-commit failed"))
        .stderr(predicate::str::contains("stub failure"));

    assert!(!fixture.path("global/alpha/SKILL.md").exists());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn sync_pull_auto_commit_requires_new_commit_and_clean_repo() {
    let fixture = Fixture::new();
    init_git_repo(fixture.root());
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let codex = fixture.path("home/.codex/skills");
    write_skill(&agents, "alpha", "a");
    fs::create_dir_all(&claude).unwrap();
    fs::create_dir_all(&codex).unwrap();
    let config = fixture.write_config(auto_commit_config(
        &agents,
        &claude,
        &codex,
        true,
        "gpt-5.4-mini",
        "medium",
    ));
    let (_log, path) =
        install_codex_stub(&fixture, "#!/usr/bin/env bash\nexit 0\n", "codex-noop.log");
    commit_all(fixture.root(), "baseline");
    fs::create_dir_all(fixture.path("global")).unwrap();
    fs::write(fixture.path("global/local.txt"), "pending").unwrap();
    commit_paths(
        fixture.root(),
        &["home", "skillnet.toml", "stub-bin"],
        "test baseline",
    );

    fixture
        .command(&config)
        .env("PATH", path)
        .args(["sync", "pull", "--scope", "global"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("did not create a new commit"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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
        .args(["status", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global"));

    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, "garbage").unwrap();
    fixture
        .command(&config)
        .args(["status", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("global"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn status_json_emits_schema_v1_with_all_fields() {
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

    let output = fixture
        .command(&config)
        .args(["status", "--scope", "global", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.trim_start().starts_with("{\n  \"schema\""));
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["schema"], "skillnet.status.v1");
    let scope = &value["scopes"].as_array().unwrap()[0];
    assert_eq!(scope["name"], "global");
    assert!(matches!(
        scope["state"].as_str().unwrap(),
        "clean" | "diverged"
    ));
    assert!(scope["diverged_files"].as_u64().is_some());
    assert!(matches!(
        scope["cache_state"].as_str().unwrap(),
        "fresh" | "stale" | "missing"
    ));
    assert_rfc3339_utc(scope["last_pulled_at"].as_str().unwrap());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn status_json_omits_last_pulled_at_when_never_pulled() {
    let fixture = Fixture::new();
    let config = fixture.write_config(minimal_config());

    let output = fixture
        .command(&config)
        .args(["status", "--scope", "global", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["scopes"][0]["last_pulled_at"].is_null());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
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

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_is_listed_in_top_level_help() {
    let mut command = Command::cargo_bin("skillnet").unwrap();

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("doctor"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_clean_config_exits_zero_with_no_findings() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("agents", &agents, 2), ("claude", &claude, 1)],
        &[&agents, &claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_warns_on_asymmetric_fanout() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("agents", &agents, 2), ("claude", &claude, 1)],
        &[&claude],
        &[],
    ));

    fixture
        .command(&config)
        .args(["doctor"])
        .assert()
        .code(4)
        .stdout(predicate::str::contains("asymmetric fan-out"))
        .stdout(predicate::str::contains(".agents/skills"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_warns_on_missing_sync_path_parent() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    let claude = fixture.path("home/.claude/skills");
    let missing = fixture.path("missing-parent/skills");
    fs::create_dir_all(&agents).unwrap();
    fs::create_dir_all(&claude).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("agents", &agents, 2), ("claude", &claude, 1)],
        &[&agents, &claude, &missing],
        &[],
    ));

    fixture
        .command(&config)
        .args(["doctor"])
        .assert()
        .code(4)
        .stdout(predicate::str::contains("missing sync path"))
        .stdout(predicate::str::contains("parent does not exist"));
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_does_not_warn_on_project_scopes() {
    let fixture = Fixture::new();
    let project = fixture.path("repos/demo");
    fs::create_dir_all(&project).unwrap();
    let config = fixture.write_config(format!(
        r#"
[global]
sources = []
sync_paths = []
stale_codex_skill_paths = []

[[project_source_rules]]
label = "agents"
rel = ".agents/skills"
priority = 2

[[project_source_rules]]
label = "claude"
rel = ".claude/skills"
priority = 1

[[projects]]
name = "demo"
path = "{}"
"#,
        project.display()
    ));

    fixture
        .command(&config)
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// removed by P4: pre-Option-B reconcile CLI coverage
#[ignore = "removed by P4: pre-Option-B reconcile CLI coverage"]
#[test]
fn doctor_does_not_warn_when_only_one_agent_source() {
    let fixture = Fixture::new();
    let agents = fixture.path("home/.agents/skills");
    fs::create_dir_all(&agents).unwrap();
    let config = fixture.write_config(sync_paths_config(
        &[("agents", &agents, 1)],
        &[&agents],
        &[],
    ));

    fixture
        .command(&config)
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}
