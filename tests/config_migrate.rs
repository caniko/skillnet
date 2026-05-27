use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::{tempdir, TempDir};

struct Fixture {
    temp: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            temp: tempdir().unwrap(),
        }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn cwd(&self) -> std::path::PathBuf {
        self.root().join("cwd")
    }

    fn xdg_home(&self) -> std::path::PathBuf {
        self.root().join("xdg")
    }

    fn home(&self) -> std::path::PathBuf {
        self.root().join("home")
    }

    fn legacy(&self, file_name: &str) -> std::path::PathBuf {
        self.cwd().join(file_name)
    }

    fn xdg(&self, file_name: &str) -> std::path::PathBuf {
        self.xdg_home().join("skillnet").join(file_name)
    }

    fn breadcrumb(&self, file_name: &str) -> std::path::PathBuf {
        self.cwd().join(format!(".{file_name}.moved-to-xdg"))
    }

    fn write_legacy(&self, file_name: &str, body: &str) {
        fs::create_dir_all(self.cwd()).unwrap();
        fs::write(self.legacy(file_name), body).unwrap();
    }

    fn write_xdg(&self, file_name: &str, body: &str) {
        let path = self.xdg(file_name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn command(&self) -> Command {
        fs::create_dir_all(self.cwd()).unwrap();
        fs::create_dir_all(self.home()).unwrap();
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command
            .current_dir(self.cwd())
            .env("XDG_CONFIG_HOME", self.xdg_home())
            .env("HOME", self.home())
            .env_remove("SKILLNET_CONFIG")
            .env_remove("SKILLNET_CATALOG_CONFIG")
            .env_remove("SKILLNET_MIRROR_ROOT");
        command
    }
}

fn read_optional(path: &Path) -> Option<Vec<u8>> {
    path.exists().then(|| fs::read(path).unwrap())
}

fn assert_breadcrumb_points_to(fixture: &Fixture, file_name: &str) {
    let body = fs::read_to_string(fixture.breadcrumb(file_name)).unwrap();
    assert_eq!(body, format!("{}\n", fixture.xdg(file_name).display()));
}

fn minimal_config(root: &Path) -> String {
    format!(
        r#"
mirror_root = "{}"

[global]
views = []
"#,
        root.display()
    )
}

#[test]
fn migrate_noop_when_neither_present() {
    let fixture = Fixture::new();

    fixture
        .command()
        .args(["config", "migrate"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "skillnet.toml: no config to migrate",
        ))
        .stdout(predicate::str::contains(
            "skillnet.catalog.toml: no config to migrate",
        ));
}

#[test]
fn migrate_noop_when_only_xdg_present() {
    let fixture = Fixture::new();
    fixture.write_xdg("skillnet.toml", "legacy = false\n");
    fixture.write_xdg("skillnet.catalog.toml", "catalog = true\n");

    fixture
        .command()
        .args(["config", "migrate"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "skillnet.toml: already centralised",
        ))
        .stdout(predicate::str::contains(
            "skillnet.catalog.toml: already centralised",
        ));
}

#[test]
fn migrate_moves_legacy_to_xdg_when_xdg_absent() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", "legacy = true\n");

    fixture
        .command()
        .args(["config", "migrate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skillnet.toml: moved to"));

    assert!(!fixture.legacy("skillnet.toml").exists());
    assert_eq!(
        fs::read_to_string(fixture.xdg("skillnet.toml")).unwrap(),
        "legacy = true\n"
    );
    assert_breadcrumb_points_to(&fixture, "skillnet.toml");
}

#[test]
fn migrate_deletes_legacy_when_xdg_equal() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", "same = true\n");
    fixture.write_xdg("skillnet.toml", "same = true\n");

    fixture
        .command()
        .args(["config", "migrate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("XDG copy retained"));

    assert!(!fixture.legacy("skillnet.toml").exists());
    assert_eq!(
        fs::read_to_string(fixture.xdg("skillnet.toml")).unwrap(),
        "same = true\n"
    );
    assert_breadcrumb_points_to(&fixture, "skillnet.toml");
}

#[test]
fn migrate_refuses_when_xdg_differs_without_force() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", "legacy = true\n");
    fixture.write_xdg("skillnet.toml", "xdg = true\n");

    fixture
        .command()
        .args(["config", "migrate"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--force"));

    assert_eq!(
        fs::read_to_string(fixture.legacy("skillnet.toml")).unwrap(),
        "legacy = true\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.xdg("skillnet.toml")).unwrap(),
        "xdg = true\n"
    );
}

#[test]
fn migrate_overwrites_xdg_with_force() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", "legacy = true\n");
    fixture.write_xdg("skillnet.toml", "xdg = true\n");

    fixture
        .command()
        .args(["config", "migrate", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--force"));

    assert!(!fixture.legacy("skillnet.toml").exists());
    assert_eq!(
        fs::read_to_string(fixture.xdg("skillnet.toml")).unwrap(),
        "legacy = true\n"
    );
    assert_breadcrumb_points_to(&fixture, "skillnet.toml");
}

#[test]
fn migrate_dry_run_makes_no_changes() {
    for (legacy, xdg, force, success) in [
        (None, None, false, true),
        (None, Some("xdg = true\n"), false, true),
        (Some("legacy = true\n"), None, false, true),
        (Some("same = true\n"), Some("same = true\n"), false, true),
        (Some("legacy = true\n"), Some("xdg = true\n"), false, false),
        (Some("legacy = true\n"), Some("xdg = true\n"), true, true),
    ] {
        let fixture = Fixture::new();
        if let Some(body) = legacy {
            fixture.write_legacy("skillnet.toml", body);
        }
        if let Some(body) = xdg {
            fixture.write_xdg("skillnet.toml", body);
        }
        let legacy_before = read_optional(&fixture.legacy("skillnet.toml"));
        let xdg_before = read_optional(&fixture.xdg("skillnet.toml"));

        let mut command = fixture.command();
        command.args(["config", "migrate", "--dry-run"]);
        if force {
            command.arg("--force");
        }
        let assert = command.assert();
        if success {
            assert.success();
        } else {
            assert.code(1);
        }

        assert_eq!(
            read_optional(&fixture.legacy("skillnet.toml")),
            legacy_before
        );
        assert_eq!(read_optional(&fixture.xdg("skillnet.toml")), xdg_before);
        assert!(!fixture.breadcrumb("skillnet.toml").exists());
    }
}

#[test]
fn migrate_remove_breadcrumbs_deletes_them() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.cwd()).unwrap();
    fs::write(fixture.breadcrumb("skillnet.toml"), "one\n").unwrap();
    fs::write(fixture.breadcrumb("skillnet.catalog.toml"), "two\n").unwrap();

    fixture
        .command()
        .args(["config", "migrate", "--remove-breadcrumbs"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed breadcrumb"));

    assert!(!fixture.breadcrumb("skillnet.toml").exists());
    assert!(!fixture.breadcrumb("skillnet.catalog.toml").exists());
}

#[test]
fn deprecation_warning_fires_on_legacy_pickup() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", &minimal_config(fixture.root()));

    let assert = fixture.command().args(["scope", "list"]).assert().success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);

    assert_eq!(stderr.matches("deprecated").count(), 1);
    assert!(stderr.contains("0.7.0"));
}

#[test]
fn no_deprecation_warning_on_xdg_pickup() {
    let fixture = Fixture::new();
    fixture.write_legacy("skillnet.toml", &minimal_config(fixture.root()));
    fixture.write_xdg("skillnet.toml", &minimal_config(fixture.root()));

    fixture
        .command()
        .args(["scope", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("deprecated").not());
}
