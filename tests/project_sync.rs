use std::fs;

use camino::Utf8PathBuf;
use skillnet::{
    model::{Target, TargetScope, ViewTarget},
    view::{materialize_project, AggregatorStatus},
};
use tempfile::tempdir;

fn write_skill(root: &camino::Utf8Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), name).unwrap();
}

#[test]
fn materialize_project_creates_relative_views_and_absolute_aggregator() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let project = root.join("repo");
    let canonical = project.join(".skills");
    write_skill(&canonical, "alpha");
    write_skill(&canonical, "beta");
    write_skill(&canonical, "gamma");

    let target = Target {
        name: "demo".into(),
        scope: TargetScope::Project,
        canonical_path: canonical.clone(),
        views: vec![
            ViewTarget {
                label: "claude".into(),
                path: project.join(".claude/skills"),
            },
            ViewTarget {
                label: "agents".into(),
                path: project.join(".agents/skills"),
            },
        ],
        aggregator_path: Some(root.join("mirror/projects/demo")),
    };

    let summary = materialize_project(&target).unwrap();
    assert_eq!(summary.views.len(), 2);
    assert_eq!(summary.aggregator, Some(AggregatorStatus::Created));

    for view in &target.views {
        for skill in ["alpha", "beta", "gamma"] {
            let target = fs::read_link(view.path.join(skill)).unwrap();
            let rendered = target.to_string_lossy();
            assert!(
                rendered.starts_with("../../"),
                "{rendered} should be relative"
            );
            assert_eq!(rendered, format!("../../.skills/{skill}"));
        }
    }

    assert_eq!(
        fs::read_link(root.join("mirror/projects/demo")).unwrap(),
        canonical
    );

    let second = materialize_project(&target).unwrap();
    assert!(second
        .views
        .iter()
        .all(|view| view.summary.created == 0 && view.summary.updated == 0));
    assert_eq!(second.aggregator, Some(AggregatorStatus::Unchanged));
}
