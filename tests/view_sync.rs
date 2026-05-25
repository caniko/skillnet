use std::{fs, os::unix::fs as unix_fs};

use camino::Utf8PathBuf;
use skillnet::{model::ViewTarget, view::materialize_view};
use tempfile::tempdir;

fn write_skill(root: &camino::Utf8Path, name: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), name).unwrap();
}

#[test]
fn materialize_view_creates_idempotent_symlinks_and_fixes_wrong_targets() {
    let tmp = tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
    let canonical = root.join("canonical");
    let view_path = root.join("view");
    write_skill(&canonical, "alpha");
    write_skill(&canonical, "beta");
    write_skill(&canonical, "gamma");
    let view = ViewTarget {
        label: "test".into(),
        path: view_path.clone(),
    };

    let first = materialize_view(&canonical, &view).unwrap();
    assert_eq!(first.created, 3);
    assert_eq!(first.updated, 0);
    for skill in ["alpha", "beta", "gamma"] {
        assert_eq!(
            fs::read_link(view_path.join(skill)).unwrap(),
            canonical.join(skill)
        );
    }

    let second = materialize_view(&canonical, &view).unwrap();
    assert_eq!(second.created, 0);
    assert_eq!(second.updated, 0);
    assert_eq!(second.unchanged, 3);

    fs::remove_file(view_path.join("beta")).unwrap();
    unix_fs::symlink(root.join("elsewhere"), view_path.join("beta")).unwrap();
    let third = materialize_view(&canonical, &view).unwrap();
    assert_eq!(third.updated, 1);
    assert_eq!(
        fs::read_link(view_path.join("beta")).unwrap(),
        canonical.join("beta")
    );
}
