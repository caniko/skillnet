use super::*;

#[test]
fn exclusions_apply_before_duplicate_checks_across_all_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
    let external = root.parent().unwrap().join("external");
    let provider = root.parent().unwrap().join("provider");
    for directory in [&root, &external, &provider] {
        fs::create_dir_all(directory.join("retired")).unwrap();
        fs::write(directory.join("retired/SKILL.md"), "retired").unwrap();
    }
    fs::create_dir_all(root.join("kept")).unwrap();
    fs::write(root.join("kept/SKILL.md"), "kept").unwrap();
    let manifest = external.join(manifest::MANIFEST_FILE);
    fs::write(&manifest, "class Skill { source: String }\nschemaVersion = 1\nskills: Mapping<String, Skill> = new { [\"retired\"] = new { source = \"retired\" } }").unwrap();
    let excluded = vec!["retired".to_string()];
    let plan = plan_for_selection_at(
        &root,
        root.parent().unwrap().join("bundle"),
        &[manifest],
        &[provider],
        BundleSelection {
            excluded_skills: &excluded,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.skill_names().collect::<Vec<_>>(), ["kept"]);
    plan.materialize().unwrap();
    assert!(!plan.bundle_root.join("retired").exists());
    assert!(root.join("retired/SKILL.md").exists());
}

#[test]
fn an_excluded_dependency_refuses_materialization() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
    for name in ["entry", "reference"] {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(root.join(name).join("SKILL.md"), name).unwrap();
    }
    fs::write(root.join(manifest::MANIFEST_FILE), "class Skill { dependencies: Listing<String> = new {} }\nschemaVersion = 1\nskills: Mapping<String, Skill> = new { [\"entry\"] = new { dependencies = List(\"reference\") } }").unwrap();
    let bundle = root.parent().unwrap().join("bundle");
    let excluded = vec!["reference".to_string()];
    let error = plan_for_selection_at(
        &root,
        bundle.clone(),
        &[],
        &[],
        BundleSelection {
            excluded_skills: &excluded,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("depends on missing skill `reference`"));
    assert!(!bundle.exists());
}

#[test]
fn provider_exclusions_preserve_same_named_authored_skills() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
    let provider = root.parent().unwrap().join("provider");
    for directory in [&root, &provider] {
        fs::create_dir_all(directory.join("local-policy")).unwrap();
        fs::write(directory.join("local-policy/SKILL.md"), "local-policy").unwrap();
    }
    let excluded_provider = vec!["local-policy".to_string()];
    let plan = plan_for_selection_at(
        &root,
        root.parent().unwrap().join("bundle"),
        &[],
        &[provider],
        BundleSelection {
            excluded_provider_skills: &excluded_provider,
            ..Default::default()
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.skill_names().collect::<Vec<_>>(), ["local-policy"]);
    plan.materialize().unwrap();
    assert!(plan.bundle_root.join("local-policy/SKILL.md").exists());
}

#[test]
fn custom_bundle_roots_preserve_legacy_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().join("skills")).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join(manifest::MANIFEST_FILE),
        "schemaVersion = 1\nskills = new Mapping {}",
    )
    .unwrap();
    let bundle = root.parent().unwrap().join("custom");
    let plan = plan_for_user_at(&root, bundle.clone(), &[], None)
        .unwrap()
        .unwrap();
    assert_eq!(plan.bundle_root, bundle);
}
