use std::{
    fs,
    os::unix::fs::{self as unix_fs, MetadataExt},
};

use camino::{Utf8Path, Utf8PathBuf};
use skillnet::{
    link::LinkStrategy,
    model::{Target, TargetScope, ViewTarget},
    view::{materialize_project, project_diff, project_status, AggregatorStatus},
};
use tempfile::tempdir;

fn write_skill(root: &Utf8Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), name).unwrap();
}

fn project_target(root: &Utf8Path, strategy: LinkStrategy) -> Target {
    let project = root.join("repo");
    Target {
        name: "demo".into(),
        scope: TargetScope::Project,
        link_strategy: strategy,
        canonical_path: root.join("mirror/projects/demo"),
        views: vec![ViewTarget {
            label: "claude".into(),
            path: project.join(".claude/skills"),
        }],
        aggregator_path: Some(project.join(".agents/skills")),
        project_root: Some(project),
        canonical_rel: Some(".agents/skills".into()),
        origin: None,
    }
}

fn assert_relative_project_views(target: &Target) {
    for view in &target.views {
        for skill in ["alpha", "beta", "gamma"] {
            let target = fs::read_link(view.path.join(skill)).unwrap();
            let rendered = target.to_string_lossy();
            assert!(
                rendered.starts_with("../../"),
                "{rendered} should be relative"
            );
            assert_eq!(rendered, format!("../../.agents/skills/{skill}"));
        }
    }
}

fn assert_same_inode(left: &Utf8Path, right: &Utf8Path) {
    let left_metadata = fs::symlink_metadata(left).unwrap();
    let right_metadata = fs::symlink_metadata(right).unwrap();
    assert_eq!(left_metadata.dev(), right_metadata.dev());
    assert_eq!(left_metadata.ino(), right_metadata.ino());
}

fn assert_different_inode(left: &Utf8Path, right: &Utf8Path) {
    let left_metadata = fs::symlink_metadata(left).unwrap();
    let right_metadata = fs::symlink_metadata(right).unwrap();
    assert_ne!(
        (left_metadata.dev(), left_metadata.ino()),
        (right_metadata.dev(), right_metadata.ino())
    );
}

#[test]
fn materialize_project_preserves_symlink_aggregator_for_symlink_strategy() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let target = project_target(&root, LinkStrategy::Symlink);
    for skill in ["alpha", "beta", "gamma"] {
        write_skill(&target.canonical_path, skill);
    }

    let summary = materialize_project(&target).unwrap();
    assert_eq!(summary.views.len(), 1);
    assert_eq!(summary.aggregator, Some(AggregatorStatus::Created));
    assert!(summary.aggregator_pending.is_empty());
    assert_relative_project_views(&target);

    let aggregator = target.aggregator_path.as_ref().unwrap();
    let metadata = fs::symlink_metadata(aggregator).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(aggregator).unwrap(), target.canonical_path);

    let second = materialize_project(&target).unwrap();
    assert!(second
        .views
        .iter()
        .all(|view| view.summary.created == 0 && view.summary.updated == 0));
    assert_eq!(second.aggregator, Some(AggregatorStatus::Unchanged));
    assert!(second.aggregator_pending.is_empty());
}

#[test]
fn materialize_project_hardlinks_aggregator_and_repairs_severed_files() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let target = project_target(&root, LinkStrategy::Hardlink);
    for skill in ["alpha", "beta", "gamma"] {
        write_skill(&target.canonical_path, skill);
    }

    let summary = materialize_project(&target).unwrap();
    assert_eq!(summary.views.len(), 1);
    assert_eq!(summary.aggregator, Some(AggregatorStatus::Created));
    assert!(summary.aggregator_pending.is_empty());
    assert_relative_project_views(&target);

    let aggregator = target.aggregator_path.as_ref().unwrap();
    let aggregator_skill = aggregator.join("alpha/SKILL.md");
    let canonical_skill = target.canonical_path.join("alpha/SKILL.md");
    let metadata = fs::symlink_metadata(&aggregator_skill).unwrap();
    assert!(metadata.file_type().is_file());
    assert!(!metadata.file_type().is_symlink());
    assert_same_inode(&canonical_skill, &aggregator_skill);

    let second = materialize_project(&target).unwrap();
    assert!(second
        .views
        .iter()
        .all(|view| view.summary.created == 0 && view.summary.updated == 0));
    assert_eq!(second.aggregator, Some(AggregatorStatus::Unchanged));
    assert!(second.aggregator_pending.is_empty());

    fs::remove_file(&aggregator_skill).unwrap();
    fs::copy(&canonical_skill, &aggregator_skill).unwrap();
    assert_different_inode(&canonical_skill, &aggregator_skill);

    let repaired = materialize_project(&target).unwrap();
    assert_eq!(repaired.aggregator, Some(AggregatorStatus::Updated));
    assert!(repaired.aggregator_pending.is_empty());
    assert_same_inode(&canonical_skill, &aggregator_skill);
}

#[test]
fn materialize_project_repairs_diverged_hardlink_working_copy_files() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let target = project_target(&root, LinkStrategy::Hardlink);
    write_skill(&target.canonical_path, "alpha");

    materialize_project(&target).unwrap();
    let aggregator_skill = target
        .aggregator_path
        .as_ref()
        .unwrap()
        .join("alpha/SKILL.md");
    let canonical_skill = target.canonical_path.join("alpha/SKILL.md");

    fs::remove_file(&aggregator_skill).unwrap();
    fs::copy(&canonical_skill, &aggregator_skill).unwrap();
    fs::write(&aggregator_skill, "edited").unwrap();

    let repaired = materialize_project(&target).unwrap();
    assert_eq!(repaired.aggregator, Some(AggregatorStatus::Updated));
    assert!(repaired.aggregator_pending.is_empty());
    assert_eq!(fs::read_to_string(&aggregator_skill).unwrap(), "alpha");
    assert_eq!(fs::read_to_string(&canonical_skill).unwrap(), "alpha");
    assert_same_inode(&canonical_skill, &aggregator_skill);

    let drift = project_status(&target).unwrap();
    assert!(drift.is_empty());
    let diff = project_diff(&target).unwrap();
    assert!(diff.is_empty());
}

#[test]
fn materialize_project_repairs_symlink_only_canonical_from_legacy_skills() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let target = project_target(&root, LinkStrategy::Hardlink);
    let project_root = target.project_root.as_ref().unwrap();
    let legacy = project_root.join(".skills");

    for skill in ["alpha", "beta", "gamma"] {
        write_skill(&legacy, skill);
        fs::create_dir_all(&target.canonical_path).unwrap();
        unix_fs::symlink(
            format!("../../.skills/{skill}"),
            target.canonical_path.join(skill),
        )
        .unwrap();
        fs::create_dir_all(target.aggregator_path.as_ref().unwrap()).unwrap();
        unix_fs::symlink(
            format!("../../.skills/{skill}"),
            target.aggregator_path.as_ref().unwrap().join(skill),
        )
        .unwrap();
    }

    let summary = materialize_project(&target).unwrap();

    assert_eq!(summary.aggregator, Some(AggregatorStatus::Updated));
    assert!(summary.aggregator_pending.is_empty());
    assert_relative_project_views(&target);
    assert!(legacy.join("alpha/SKILL.md").is_file());
    assert!(!fs::symlink_metadata(target.canonical_path.join("alpha"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_same_inode(
        &target.canonical_path.join("alpha/SKILL.md"),
        &target
            .aggregator_path
            .as_ref()
            .unwrap()
            .join("alpha/SKILL.md"),
    );
}

#[test]
fn materialize_project_refuses_divergent_real_canonical_during_legacy_repair() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let target = project_target(&root, LinkStrategy::Hardlink);
    let project_root = target.project_root.as_ref().unwrap();
    let legacy = project_root.join(".skills");

    write_skill(&legacy, "alpha");
    fs::write(legacy.join("alpha/SKILL.md"), "legacy").unwrap();
    write_skill(&target.canonical_path, "alpha");
    fs::write(target.canonical_path.join("alpha/SKILL.md"), "canonical").unwrap();
    fs::create_dir_all(target.aggregator_path.as_ref().unwrap()).unwrap();
    unix_fs::symlink(
        "../../.skills/alpha",
        target.aggregator_path.as_ref().unwrap().join("alpha"),
    )
    .unwrap();

    let err = materialize_project(&target).unwrap_err();

    assert!(err
        .to_string()
        .contains("already contains real content that diverges"));
    assert_eq!(
        fs::read_to_string(target.canonical_path.join("alpha/SKILL.md")).unwrap(),
        "canonical"
    );
}
