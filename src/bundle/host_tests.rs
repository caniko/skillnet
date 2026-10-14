use super::*;

fn fixture() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
    let data = Utf8PathBuf::from_path_buf(tmp.path().join("data")).unwrap();
    for name in ["generic", "atlas-only"] {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(root.join(name).join("SKILL.md"), name).unwrap();
    }
    fs::write(
        root.join(manifest::MANIFEST_FILE),
        r#"
class Skill {
  role: String = "entrypoint"
  dependencies: Listing<String> = new {}
  users: Listing<String>? = null
  hosts: Listing<String>? = null
}
schemaVersion = 3
defaultUsers = List("can", "dejana")
skills: Mapping<String, Skill> = new {
  ["generic"] = new {}
  ["atlas-only"] = new {
    hosts = List("atlas")
    users = List("can")
    dependencies = List("generic")
  }
}
"#,
    )
    .unwrap();
    (tmp, root, data)
}

#[test]
fn host_bundle_intersects_user_and_host_and_removes_stale_dependencies() {
    let (_tmp, root, data) = fixture();
    let atlas = plan_for_access(&root, "global", &data, &[], &[], Some("can"), Some("atlas"))
        .unwrap()
        .unwrap();
    atlas.materialize().unwrap();
    assert!(atlas
        .bundle_root
        .join("atlas-only/.skillnet/deps/generic")
        .is_symlink());
    for (user, host) in [("can", "nomad"), ("dejana", "atlas")] {
        let other = plan_for_access(&root, "global", &data, &[], &[], Some(user), Some(host))
            .unwrap()
            .unwrap();
        other.materialize().unwrap();
        assert!(other.bundle_root.join("generic/SKILL.md").is_file());
        assert!(!other.bundle_root.join("atlas-only").exists());
    }
}

#[test]
fn host_bundle_requires_explicit_host_before_materialization() {
    let (_tmp, root, data) = fixture();
    let error = plan_for_access(&root, "global", &data, &[], &[], Some("can"), None).unwrap_err();
    assert!(error.to_string().contains("no Skillnet host"));
    assert!(!data.exists());
}

#[test]
fn host_bundle_rejects_a_denied_dependency() {
    let (_tmp, root, data) = fixture();
    let file = root.join(manifest::MANIFEST_FILE);
    let source = fs::read_to_string(&file)
        .unwrap()
        .replace(
            "[\"generic\"] = new {}",
            "[\"generic\"] = new { dependencies = List(\"atlas-only\") }",
        )
        .replace("dependencies = List(\"generic\")", "dependencies = List()");
    fs::write(file, source).unwrap();
    let error =
        plan_for_access(&root, "global", &data, &[], &[], Some("can"), Some("nomad")).unwrap_err();
    assert!(error.to_string().contains("dependency `atlas-only`"));
}

#[test]
fn host_bundle_inherits_defaults_and_empty_lists_deny_all() {
    let (_tmp, root, data) = fixture();
    let file = root.join(manifest::MANIFEST_FILE);
    let source = fs::read_to_string(&file).unwrap().replace(
        "defaultUsers = List(\"can\", \"dejana\")",
        "defaultUsers = List(\"can\", \"dejana\")\ndefaultHosts = List(\"atlas\")",
    );
    fs::write(&file, &source).unwrap();
    let nomad = plan_for_access(&root, "global", &data, &[], &[], Some("can"), Some("nomad"))
        .unwrap()
        .unwrap();
    assert!(nomad.expected_links().is_empty());
    let atlas = plan_for_access(&root, "global", &data, &[], &[], Some("can"), Some("atlas"))
        .unwrap()
        .unwrap();
    assert_eq!(atlas.expected_links().len(), 2);
    // Explicit empty skill lists override a granting default.
    fs::write(
        &file,
        source.replace("hosts = List(\"atlas\")", "hosts = List()"),
    )
    .unwrap();
    let atlas = plan_for_access(&root, "global", &data, &[], &[], Some("can"), Some("atlas"))
        .unwrap()
        .unwrap();
    assert_eq!(atlas.skill_names().collect::<Vec<_>>(), ["generic"]);
    fs::write(
        &file,
        "schemaVersion = 3\ndefaultHosts = List()\nskills = new Mapping {}",
    )
    .unwrap();
    let denied = plan_for_access(&root, "global", &data, &[], &[], None, Some("atlas"))
        .unwrap()
        .unwrap();
    assert!(denied.expected_links().is_empty());
}

#[test]
fn host_bundle_composes_external_host_skill_with_global_dependency() {
    let (_tmp, root, data) = fixture();
    fs::remove_dir_all(root.join("atlas-only")).unwrap();
    fs::write(
        root.join(manifest::MANIFEST_FILE),
        "schemaVersion = 2\ndefaultUsers = List(\"can\")\nskills = new Mapping {}",
    )
    .unwrap();
    let external = root.parent().unwrap().join("host-skills");
    fs::create_dir_all(external.join("adapter")).unwrap();
    fs::write(external.join("adapter/SKILL.md"), "host adapter").unwrap();
    let file = external.join(manifest::MANIFEST_FILE);
    fs::write(
        &file,
        r#"
class Skill {
  source: String
  hosts: Listing<String>? = null
  dependencies: Listing<String> = new {}
}
schemaVersion = 3
skills: Mapping<String, Skill> = new {
  ["atlas-only"] = new {
    source = "adapter"
    hosts = List("atlas")
    dependencies = List("generic")
  }
}
"#,
    )
    .unwrap();
    for host in ["atlas", "nomad"] {
        let plan = plan_for_access(
            &root,
            "global",
            &data,
            std::slice::from_ref(&file),
            &[],
            Some("can"),
            Some(host),
        )
        .unwrap()
        .unwrap();
        plan.materialize().unwrap();
        assert!(plan.bundle_root.join("generic/SKILL.md").is_file());
        assert_eq!(
            plan.expected_links().contains_key("atlas-only"),
            host == "atlas"
        );
        if host == "atlas" {
            assert_eq!(
                fs::read_to_string(
                    plan.bundle_root
                        .join("atlas-only/.skillnet/deps/generic/SKILL.md")
                )
                .unwrap(),
                "generic"
            );
        }
    }
    assert!(plan_for_access(&root, "global", &data, &[file], &[], Some("can"), None).is_err());
}

#[test]
fn host_bundle_preserves_schema_one_and_two_without_a_host_selector() {
    let (_tmp, root, data) = fixture();
    for version in [1, 2] {
        fs::write(
            root.join(manifest::MANIFEST_FILE),
            format!("schemaVersion = {version}\nskills = new Mapping {{}}"),
        )
        .unwrap();
        let plan = plan_for_access(&root, "global", &data, &[], &[], None, None)
            .unwrap()
            .unwrap();
        assert_eq!(plan.expected_links().len(), 2);
    }
}

#[test]
fn host_bundle_rejects_host_fields_in_legacy_schemas_and_invalid_host_lists() {
    let (_tmp, root, data) = fixture();
    let file = root.join(manifest::MANIFEST_FILE);
    for version in [1, 2] {
        fs::write(
            &file,
            format!("schemaVersion = {version}\ndefaultHosts = List(\"atlas\")"),
        )
        .unwrap();
        assert!(manifest::load(&root)
            .unwrap_err()
            .to_string()
            .contains("schemaVersion 3"));
    }
    for hosts in [
        "List(\"atlas\", \"atlas\")",
        "List(\"\")",
        "List(\"bad host\")",
        "List(\"../atlas\")",
    ] {
        fs::write(&file, format!("schemaVersion = 3\ndefaultHosts = {hosts}")).unwrap();
        assert!(manifest::load(&root).is_err(), "{hosts}");
    }
    fs::write(&file, "schemaVersion = 3").unwrap();
    assert!(plan_for_access(&root, "global", &data, &[], &[], None, Some("bad host")).is_err());
    for host in ["", "bad host", "atlas/nomad"] {
        let config = root.parent().unwrap().join("skillnet.toml");
        fs::write(&config, format!("host = {host:?}")).unwrap();
        assert!(crate::config::Config::load(&config).is_err());
    }
}
