use std::{
    fs,
    os::unix::fs as unix_fs,
    os::unix::fs::MetadataExt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};
use filetime::{set_file_mtime, FileTime};
use skillnet::{
    link::LinkStrategy,
    model::ViewTarget,
    view::{
        compare_view_entry, materialize_view, materialize_view_with_options,
        materialize_view_with_promotion, promote_view_to_canonical, DriftEntry, DriftKind,
        Preference, PromotionOptions, ReconcileOutcome,
    },
};
use tempfile::{tempdir, TempDir};

struct Fixture {
    _tmp: TempDir,
    canonical: Utf8PathBuf,
    view_path: Utf8PathBuf,
    view: ViewTarget,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let canonical = root.join("canonical");
        let view_path = root.join("view");
        let view = ViewTarget {
            label: "test".into(),
            path: view_path.clone(),
        };
        Self {
            _tmp: tmp,
            canonical,
            view_path,
            view,
        }
    }
}

fn write_skill(root: &Utf8Path, name: &str, body: &str, mtime_secs: i64) -> Utf8PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    write_skill_file(&dir, "SKILL.md", body, mtime_secs);
    dir
}

fn write_skill_file(skill: &Utf8Path, rel: &str, body: &str, mtime_secs: i64) {
    let path = skill.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, body).unwrap();
    set_file_mtime(path, FileTime::from_unix_time(mtime_secs, 0)).unwrap();
}

fn default_summary(fixture: &Fixture) -> skillnet::view::PromotionSummary {
    materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions::default(),
        |_| Ok(()),
    )
    .unwrap()
}

fn is_symlink(path: &Utf8Path) -> bool {
    fs::symlink_metadata(path).unwrap().file_type().is_symlink()
}

fn file_mtime_nanos(path: &Utf8Path) -> u128 {
    fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[test]
fn materialize_view_creates_idempotent_symlinks_and_fixes_wrong_targets() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "alpha", "alpha", 100);
    write_skill(&fixture.canonical, "beta", "beta", 100);
    write_skill(&fixture.canonical, "gamma", "gamma", 100);

    let first = materialize_view(&fixture.canonical, &fixture.view).unwrap();
    assert_eq!(first.created, 3);
    assert_eq!(first.updated, 0);
    for skill in ["alpha", "beta", "gamma"] {
        assert_eq!(
            fs::read_link(fixture.view_path.join(skill)).unwrap(),
            fixture.canonical.join(skill)
        );
    }

    let second = materialize_view(&fixture.canonical, &fixture.view).unwrap();
    assert_eq!(second.created, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(second.unchanged, 3);

    fs::remove_file(fixture.view_path.join("beta")).unwrap();
    unix_fs::symlink(
        fixture.canonical.parent().unwrap().join("elsewhere"),
        fixture.view_path.join("beta"),
    )
    .unwrap();
    let third = materialize_view(&fixture.canonical, &fixture.view).unwrap();
    assert_eq!(third.updated, 1);
    assert_eq!(
        fs::read_link(fixture.view_path.join("beta")).unwrap(),
        fixture.canonical.join("beta")
    );
}

#[test]
fn materialize_view_supports_hardlinked_global_views() {
    let fixture = Fixture::new();
    let canonical = write_skill(&fixture.canonical, "hardlinked", "same", 100);
    let first = materialize_view_with_options(
        &fixture.canonical,
        &fixture.view,
        skillnet::view::ViewSyncOptions {
            link_strategy: LinkStrategy::Hardlink,
            ..skillnet::view::ViewSyncOptions::default()
        },
    )
    .unwrap();
    assert_eq!(first.created, 1);
    assert!(!is_symlink(&fixture.view_path.join("hardlinked")));
    let view_file = fixture.view_path.join("hardlinked/SKILL.md");
    let canonical_file = canonical.join("SKILL.md");
    assert_eq!(
        fs::metadata(view_file).unwrap().ino(),
        fs::metadata(canonical_file).unwrap().ino()
    );

    let second = materialize_view_with_options(
        &fixture.canonical,
        &fixture.view,
        skillnet::view::ViewSyncOptions {
            link_strategy: LinkStrategy::Hardlink,
            ..skillnet::view::ViewSyncOptions::default()
        },
    )
    .unwrap();
    assert_eq!(second.unchanged, 1);
}

#[test]
fn identical_demotes_to_symlink() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "identical", "same", 100);
    write_skill(&fixture.view_path, "identical", "same", 100);

    let summary = default_summary(&fixture);

    assert!(summary.promoted.is_empty());
    assert_eq!(summary.view.updated, 1);
    assert!(is_symlink(&fixture.view_path.join("identical")));
}

#[test]
fn view_newer_default_records_would_promote() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "view-newer", "canonical", 100);
    write_skill(&fixture.view_path, "view-newer", "view", 200);

    let summary = default_summary(&fixture);

    assert_eq!(summary.would_promote.len(), 1);
    assert_eq!(summary.would_promote[0].skill, "view-newer");
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("view-newer/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(!is_symlink(&fixture.view_path.join("view-newer")));
}

#[test]
fn view_newer_apply_promote_pulls_and_demotes() {
    let fixture = Fixture::new();
    let canonical = write_skill(&fixture.canonical, "apply-promote", "canonical", 100);
    let view = write_skill(&fixture.view_path, "apply-promote", "view", 200);
    let view_mtime = skillnet::view::compare_view_entry(&canonical, &view).unwrap();

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            apply_promote: true,
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(summary.promoted, vec!["apply-promote"]);
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("apply-promote/SKILL.md")).unwrap(),
        "view"
    );
    assert!(is_symlink(&fixture.view_path.join("apply-promote")));
    let ReconcileOutcome::ViewNewer { view_mtime, .. } = view_mtime else {
        panic!("expected view-newer outcome before promotion");
    };
    assert_eq!(
        file_mtime_nanos(&fixture.canonical.join("apply-promote/SKILL.md")),
        view_mtime
    );
}

#[test]
fn canonical_newer_default_records_would_demote_destructive() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "canonical-newer", "canonical", 200);
    write_skill(&fixture.view_path, "canonical-newer", "view", 100);

    let summary = default_summary(&fixture);

    assert_eq!(summary.would_demote_destructive.len(), 1);
    assert_eq!(summary.would_demote_destructive[0].skill, "canonical-newer");
    assert!(!is_symlink(&fixture.view_path.join("canonical-newer")));
}

#[test]
fn canonical_newer_force_demote_destroys_view_content() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "force-demote", "canonical", 200);
    write_skill(&fixture.view_path, "force-demote", "view", 100);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            force_demote: true,
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(summary.demoted_destructive, vec!["force-demote"]);
    assert!(is_symlink(&fixture.view_path.join("force-demote")));
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("force-demote/SKILL.md")).unwrap(),
        "canonical"
    );
}

#[test]
fn equal_mtime_different_content_needs_tie_break() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "tie", "canonical", 100);
    write_skill(&fixture.view_path, "tie", "view", 100);

    let summary = default_summary(&fixture);

    assert_eq!(summary.needs_tie_break.len(), 1);
    assert!(matches!(
        summary.needs_tie_break[0].outcome,
        ReconcileOutcome::EqualMtimeDifferentContent { .. }
    ));
}

#[test]
fn equal_mtime_with_prefer_view_promotes() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "prefer-view", "canonical", 100);
    write_skill(&fixture.view_path, "prefer-view", "view", 100);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            apply_promote: true,
            prefer: Some(Preference::View),
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(summary.promoted, vec!["prefer-view"]);
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("prefer-view/SKILL.md")).unwrap(),
        "view"
    );
    assert!(is_symlink(&fixture.view_path.join("prefer-view")));
}

#[test]
fn equal_mtime_with_prefer_canonical_demotes() {
    let fixture = Fixture::new();
    write_skill(&fixture.canonical, "prefer-canonical", "canonical", 100);
    write_skill(&fixture.view_path, "prefer-canonical", "view", 100);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            force_demote: true,
            prefer: Some(Preference::Canonical),
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(summary.demoted_destructive, vec!["prefer-canonical"]);
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("prefer-canonical/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(is_symlink(&fixture.view_path.join("prefer-canonical")));
}

#[test]
fn both_advanced_default_needs_tie_break() {
    let fixture = Fixture::new();
    let canonical = write_skill(&fixture.canonical, "both", "canonical", 100);
    write_skill_file(&canonical, "canonical-only.md", "canonical-only", 100);
    let view = write_skill(&fixture.view_path, "both", "view", 200);
    write_skill_file(&view, "view-only.md", "view-only", 200);

    let summary = default_summary(&fixture);

    assert_eq!(summary.needs_tie_break.len(), 1);
    assert!(matches!(
        summary.needs_tie_break[0].outcome,
        ReconcileOutcome::BothAdvanced { .. }
    ));
}

#[test]
fn adopt_candidate_default_noops() {
    let fixture = Fixture::new();
    write_skill(&fixture.view_path, "orphan", "view", 100);

    let summary = default_summary(&fixture);

    assert!(summary.adopted.is_empty());
    assert!(!fixture.canonical.join("orphan").exists());
    assert!(!is_symlink(&fixture.view_path.join("orphan")));
    let status = skillnet::view::view_status(&fixture.canonical, &fixture.view).unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].kind, DriftKind::Stale);
}

#[test]
fn adopt_candidate_with_adopt_new_promotes() {
    let fixture = Fixture::new();
    write_skill(&fixture.view_path, "adopt", "view", 100);

    let summary = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            apply_promote: true,
            adopt_new: true,
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap();

    assert_eq!(summary.adopted, vec!["adopt"]);
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("adopt/SKILL.md")).unwrap(),
        "view"
    );
    assert!(!is_symlink(&fixture.view_path.join("adopt")));
}

#[test]
fn future_mtime_downgrades_to_tie_break() {
    let fixture = Fixture::new();
    let canonical = write_skill(&fixture.canonical, "future", "canonical", 100);
    let view = write_skill(&fixture.view_path, "future", "view", 100);
    let future = FileTime::from_system_time(SystemTime::now() + Duration::from_secs(3600));
    set_file_mtime(view.join("SKILL.md"), future).unwrap();

    let outcome = compare_view_entry(&canonical, &view).unwrap();

    assert!(matches!(
        outcome,
        ReconcileOutcome::EqualMtimeDifferentContent { .. }
    ));
}

#[test]
fn promote_preserves_mtimes() {
    let fixture = Fixture::new();
    let canonical = write_skill(&fixture.canonical, "mtime", "canonical", 100);
    let view = write_skill(&fixture.view_path, "mtime", "view", 300);
    let ReconcileOutcome::ViewNewer { view_mtime, .. } =
        compare_view_entry(&canonical, &view).unwrap()
    else {
        panic!("expected view-newer outcome");
    };

    promote_view_to_canonical(&view, &canonical).unwrap();

    assert_eq!(file_mtime_nanos(&canonical.join("SKILL.md")), view_mtime);
}

#[test]
fn partial_failure_leaves_per_skill_atomicity() {
    let fixture = Fixture::new();
    for skill in ["atomic-a", "atomic-b", "atomic-c"] {
        write_skill(&fixture.canonical, skill, "canonical", 100);
        write_skill(&fixture.view_path, skill, "view", 200);
    }
    let staging_parent = fixture.canonical.join(".skillnet-tmp");
    fs::create_dir_all(&staging_parent).unwrap();
    fs::write(
        staging_parent.join(format!("atomic-b-{}", std::process::id())),
        "not a directory",
    )
    .unwrap();

    let err = materialize_view_with_promotion(
        &fixture.canonical,
        &fixture.view,
        PromotionOptions {
            apply_promote: true,
            ..PromotionOptions::default()
        },
        |_| Ok(()),
    )
    .unwrap_err();

    assert!(err.to_string().contains("atomic-b"));
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("atomic-a/SKILL.md")).unwrap(),
        "view"
    );
    assert!(is_symlink(&fixture.view_path.join("atomic-a")));
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("atomic-b/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(!is_symlink(&fixture.view_path.join("atomic-b")));
    assert_eq!(
        fs::read_to_string(fixture.canonical.join("atomic-c/SKILL.md")).unwrap(),
        "canonical"
    );
    assert!(!is_symlink(&fixture.view_path.join("atomic-c")));
}

#[test]
fn drift_entry_missing_serializes_reconcile_fields_as_null() {
    let json = serde_json::to_value(DriftEntry {
        skill: "missing".into(),
        kind: DriftKind::Missing,
        expected: None,
        actual: None,
        view_mtime_nanos: None,
        canonical_mtime_nanos: None,
        view_sha: None,
        canonical_sha: None,
        reconcile_outcome: None,
    })
    .unwrap();

    assert!(json["view_mtime_nanos"].is_null());
    assert!(json["canonical_mtime_nanos"].is_null());
    assert!(json["view_sha"].is_null());
    assert!(json["canonical_sha"].is_null());
}
