use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn configured_exclusions_remove_stale_entries_and_survive_refresh() {
    let temp = tempdir().unwrap();
    let canonical = temp.path().join("canonical");
    let view = temp.path().join("view");
    for name in ["kept", "retired"] {
        fs::create_dir_all(canonical.join(name)).unwrap();
        fs::write(canonical.join(name).join("SKILL.md"), name).unwrap();
    }
    let config = temp.path().join("skillnet.toml");
    let settings = format!(
        "data_dir = {:?}\nexcluded_skills = []\n[database]\nbackend = \"sqlite\"\npath = {:?}\n[global]\ncanonical_path = {:?}\n[[global.views]]\nlabel = \"test\"\npath = {:?}\n",
        temp.path().join("data"),
        temp.path().join("db.sqlite"),
        canonical,
        view,
    );
    fs::write(&config, &settings).unwrap();
    let sync = || {
        Command::cargo_bin("skillnet")
            .unwrap()
            .args([
                "--config",
                config.to_str().unwrap(),
                "view",
                "sync",
                "--all",
                "--allow-delete",
            ])
            .assert()
            .success();
    };
    sync();
    assert!(view.join("retired/SKILL.md").is_file());
    fs::write(
        &config,
        settings.replace("excluded_skills = []", "excluded_skills = [\"retired\"]"),
    )
    .unwrap();
    sync();
    assert!(view.join("kept/SKILL.md").is_file());
    assert!(!view.join("retired").exists());
    // Refreshing the source cannot recreate an excluded destination entry.
    fs::write(canonical.join("retired/SKILL.md"), "refreshed").unwrap();
    sync();
    assert!(!view.join("retired").exists());
    assert_eq!(
        fs::read_to_string(canonical.join("retired/SKILL.md")).unwrap(),
        "refreshed"
    );
}
