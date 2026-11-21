use std::fs;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use serde_json::{json, Value};
use tempfile::tempdir;

#[test]
fn global_bundle_renders_templates_without_a_manifest() {
    let temp = tempdir().unwrap();
    let canonical = temp.path().join("canonical");
    let source = canonical.join("review");
    let view = temp.path().join("view");
    let data = temp.path().join("data");
    let config = temp.path().join("skillnet.toml");
    let catalog = temp.path().join("skillnet.catalog.toml");
    let bindings = temp.path().join("bindings.pkl");
    fs::create_dir_all(&source).unwrap();
    let frontmatter = "---\nname: review\ndescription: Review proposed changes.\n---\n";
    fs::write(
        source.join("SKILL.md.tmpl"),
        format!("{frontmatter}Review by {{{{seat_labels}}}}\n"),
    )
    .unwrap();
    fs::write(
        source.join("skillnet-template.toml"),
        "[seats]\nmin = 1\nmax = 1\n",
    )
    .unwrap();
    fs::write(source.join("notes.md"), "notes").unwrap();
    let settings = format!(
        "data_dir = {data:?}\n[global]\ncanonical_path = {canonical:?}\n[[global.views]]\nlabel = \"test\"\npath = {view:?}\n[templates]\nfleet = {bindings:?}\n",
    );
    fs::write(&config, settings).unwrap();
    fs::write(
        &catalog,
        "[[rules]]\nname = \"review\"\ncategory = \"agent-tools\"\n",
    )
    .unwrap();
    let command = || {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command
            .arg("--config")
            .arg(&config)
            .arg("--catalog-config")
            .arg(&catalog);
        command
    };
    command()
        .args(["catalog", "lint"])
        .assert()
        .success()
        .stdout(contains("passed for 1 skills"));
    command()
        .args(["skill", "list", "--scope", "global"])
        .assert()
        .success()
        .stdout("# global\nreview\n");
    let sync = || {
        let mut command = command();
        command.args(["view", "sync", "--all", "--allow-delete"]);
        command
    };
    sync().assert().failure().stderr(contains("bindings.pkl"));
    fs::write(&bindings, "skills {}").unwrap();
    sync()
        .assert()
        .failure()
        .stderr(contains("review").and(contains("bindings.pkl")));
    command()
        .args(["sync", "--scope", "global"])
        .assert()
        .failure()
        .stderr(contains("review").and(contains("bindings.pkl")));
    let missing = temp.path().join("missing.pkl");
    command()
        .args(["template", "render", "review", "--bindings"])
        .arg(&missing)
        .assert()
        .failure()
        .stderr(contains("missing.pkl"));
    fs::write(
        &bindings,
        r#"skills { ["review"] { seats { new { name = "reviewer"; route { provider = "opencode"; model = "Sol" } } } } }"#,
    )
    .unwrap();
    let rendered = format!("{frontmatter}Review by Sol\n");
    command()
        .args(["template", "render", "review"])
        .assert()
        .success()
        .stdout(rendered.clone());
    sync().assert().success();
    command()
        .args(["status", "--scope", "global", "--format", "json"])
        .assert()
        .success()
        .stdout(contains("\"skill_count\": 1"));
    let bundle = data.join("bundles/global/review");
    let generated = bundle.join("SKILL.md");
    assert!(generated.symlink_metadata().unwrap().file_type().is_file());
    assert_eq!(fs::read_to_string(&generated).unwrap(), rendered);
    assert!(!source.join("SKILL.md").exists());
    assert!(!bundle.join("SKILL.md.tmpl").exists());
    assert!(!bundle.join("skillnet-template.toml").exists());
    assert!(bundle.join("notes.md").is_symlink());
    assert_eq!(fs::read_link(view.join("review")).unwrap(), bundle);

    command()
        .args(["view", "status", "--all"])
        .assert()
        .success()
        .stdout(contains("clean"));
    fs::write(
        &bindings,
        fs::read_to_string(&bindings)
            .unwrap()
            .replace("Sol", "Terra"),
    )
    .unwrap();
    command()
        .args(["view", "status", "--all"])
        .assert()
        .success()
        .stdout(contains("generated entrypoint for `review` is stale"));
    command()
        .args(["doctor"])
        .assert()
        .failure()
        .stderr(contains("generated entrypoint for `review` is stale"));
    fs::write(
        &config,
        format!(
            "bundles_root = {canonical:?}\n{}",
            fs::read_to_string(&config).unwrap()
        ),
    )
    .unwrap();
    sync()
        .assert()
        .failure()
        .stderr(contains("overlaps canonical source"));
    assert!(!canonical.join("global").exists());
}

#[test]
fn models_json_uses_enabled_provider_snapshots() {
    let temp = tempdir().unwrap();
    let snapshot = json!({
        "displayName": "OpenCode", "enabled": true, "status": "ready",
        "models": [{"slug": "org/model", "name": "Model", "capabilities": {
            "optionDescriptors": [{"id": "variant", "label": "Reasoning", "type": "select",
                "options": [{"id": "high", "label": "High", "isDefault": true}] }]
        }}]
    });
    fs::write(temp.path().join("opencode.json"), snapshot.to_string()).unwrap();
    let mut disabled = snapshot;
    disabled["enabled"] = json!(false);
    fs::write(temp.path().join("disabled.json"), disabled.to_string()).unwrap();
    fs::write(temp.path().join("malformed.json"), "{").unwrap();
    fs::create_dir(temp.path().join("ignored.json")).unwrap();
    let assert = Command::cargo_bin("skillnet")
        .unwrap()
        .current_dir(temp.path())
        .env("XDG_CONFIG_HOME", temp.path())
        .env_remove("SKILLNET_CONFIG")
        .args([
            "models",
            "list",
            "--json",
            "--caches",
            temp.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(contains("malformed.json").and(contains("warning")));
    let providers: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(providers.as_array().unwrap().len(), 1);
    assert_eq!(providers[0]["instanceId"], "opencode");
    assert_eq!(
        providers[0]["models"][0]["capabilities"]["optionDescriptors"][0]["options"][0]["id"],
        "high"
    );
    let config = temp.path().join("skillnet.toml");
    let bindings = temp.path().join("bindings.pkl");
    fs::write(
        &config,
        format!(
            "[global]\nviews = []\n[templates]\nfleet = {bindings:?}\n[models]\nt3_caches = {:?}\n",
            temp.path()
        ),
    )
    .unwrap();
    let list = || {
        let mut command = Command::cargo_bin("skillnet").unwrap();
        command
            .arg("--config")
            .arg(&config)
            .args(["models", "list"]);
        command
    };
    fs::write(&bindings, r#"ui { providers = List("opencode") }"#).unwrap();
    list()
        .assert()
        .success()
        .stdout("opencode org/model variant=high\n");
    fs::write(&bindings, r#"ui { providers = List("disabled") }"#).unwrap();
    list().assert().success().stdout("");
}

#[test]
fn global_sync_without_templates_keeps_direct_views() {
    let temp = tempdir().unwrap();
    let canonical = temp.path().join("canonical");
    let view = temp.path().join("view");
    let config = temp.path().join("skillnet.toml");
    // Neither an unexpandable path nor a broken binding file should matter.
    let broken = temp.path().join("broken.pkl");
    fs::write(&broken, "skills {").unwrap();
    fs::create_dir_all(canonical.join("plain")).unwrap();
    fs::write(
        canonical.join("plain/SKILL.md"),
        "---\nname: plain\ndescription: Plain skill.\n---\n",
    )
    .unwrap();
    fs::write(
        &config,
        format!(
            "data_dir = {:?}\n[global]\ncanonical_path = {canonical:?}\n[[global.views]]\nlabel = \"test\"\npath = {view:?}\n[templates]\nfleet = \"${{SKILLNET_TEST_UNSET_VAR}}/x.pkl\"\nlocal = {broken:?}\n",
            temp.path().join("data"),
        ),
    )
    .unwrap();
    Command::cargo_bin("skillnet")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .env_remove("SKILLNET_TEST_UNSET_VAR")
        .args(["view", "sync", "--all", "--allow-delete"])
        .assert()
        .success();
    assert_eq!(
        fs::read_link(view.join("plain")).unwrap(),
        canonical.join("plain")
    );
}

#[test]
fn first_template_keeps_view_only_authored_skills() {
    let temp = tempdir().unwrap();
    let canonical = temp.path().join("canonical");
    let view = temp.path().join("view");
    let config = temp.path().join("skillnet.toml");
    let bindings = temp.path().join("bindings.pkl");
    fs::create_dir_all(canonical.join("review")).unwrap();
    fs::write(
        canonical.join("review/SKILL.md.tmpl"),
        "---\nname: review\ndescription: Review.\n---\n{{seat_labels}}\n",
    )
    .unwrap();
    fs::write(
        &bindings,
        r#"skills { ["review"] { seats { new { name = "r"; route { provider = "p"; model = "m" } } } } }"#,
    )
    .unwrap();
    // Authored directly in the view; the canonical store does not have it.
    fs::create_dir_all(view.join("authored")).unwrap();
    fs::write(view.join("authored/SKILL.md"), "---\nname: authored\n---\n").unwrap();
    fs::write(
        &config,
        format!(
            "data_dir = {:?}\n[global]\ncanonical_path = {canonical:?}\n[[global.views]]\nlabel = \"test\"\npath = {view:?}\n[templates]\nfleet = {bindings:?}\n",
            temp.path().join("data")
        ),
    )
    .unwrap();
    Command::cargo_bin("skillnet")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .args(["view", "sync", "--all", "--allow-delete"])
        .assert()
        .success();
    assert!(view.join("authored/SKILL.md").is_file());
    assert!(view.join("review").is_symlink());
}

#[test]
fn binding_files_inside_the_bundle_are_refused() {
    let temp = tempdir().unwrap();
    let canonical = temp.path().join("canonical");
    let data = temp.path().join("data");
    let config = temp.path().join("skillnet.toml");
    fs::create_dir_all(canonical.join("review")).unwrap();
    fs::write(
        canonical.join("review/SKILL.md.tmpl"),
        "---\nname: review\ndescription: Review.\n---\n{{seat_labels}}\n",
    )
    .unwrap();
    // Pruning the bundle would delete this override on the first sync.
    let inside = data.join("bundles/global/overrides.pkl");
    fs::create_dir_all(inside.parent().unwrap()).unwrap();
    fs::write(
        &inside,
        r#"skills { ["review"] { seats { new { name = "r"; route { provider = "p"; model = "m" } } } } }"#,
    )
    .unwrap();
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[global]\ncanonical_path = {canonical:?}\n[[global.views]]\nlabel = \"test\"\npath = {:?}\n[templates]\nlocal = {inside:?}\n",
            temp.path().join("view")
        ),
    )
    .unwrap();
    Command::cargo_bin("skillnet")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .args(["view", "sync", "--all"])
        .assert()
        .failure()
        .stderr(contains("inside bundle"));
    assert!(inside.is_file());

    // A link inside the bundle to a file outside it would be unlinked too.
    let outside = temp.path().join("outside.pkl");
    fs::rename(&inside, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &inside).unwrap();
    Command::cargo_bin("skillnet")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .args(["view", "sync", "--all"])
        .assert()
        .failure()
        .stderr(contains("inside bundle"));
    assert!(inside.is_symlink());

    // So would a linked folder on the way to it.
    fs::remove_file(&inside).unwrap();
    let outside_dir = temp.path().join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::rename(&outside, outside_dir.join("overrides.pkl")).unwrap();
    let hop = data.join("bundles/global/hop");
    std::os::unix::fs::symlink(&outside_dir, &hop).unwrap();
    let via_hop = hop.join("overrides.pkl");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace(&format!("{inside:?}"), &format!("{via_hop:?}")),
    )
    .unwrap();
    Command::cargo_bin("skillnet")
        .unwrap()
        .arg("--config")
        .arg(&config)
        .args(["view", "sync", "--all"])
        .assert()
        .failure()
        .stderr(contains("inside bundle"));
    assert!(hop.is_symlink());
}
