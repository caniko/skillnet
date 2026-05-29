use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    os::unix::fs as unix_fs,
    os::unix::fs::MetadataExt,
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use filetime::{set_file_times, set_symlink_file_times, FileTime};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

const EXDEV: i32 = 18;
const STAGING_DIR: &str = ".skillnet-tmp";

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardlinkStatus {
    Missing,
    Identical,
    Severed { files: Vec<Utf8PathBuf> },
    Diverged { files: Vec<Utf8PathBuf> },
    Foreign,
}

pub fn newest_mtime_nanos(path: &Utf8Path) -> Result<u128> {
    let mut newest = 0;
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            let modified = entry
                .metadata()?
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let nanos = modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            newest = newest.max(nanos);
        }
    }
    Ok(newest)
}

pub fn content_signature(path: &Utf8Path) -> Result<String> {
    let mut files = Vec::new();
    for entry in WalkDir::new(path).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            files.push(entry.into_path());
        }
    }
    files.sort();

    let mut hasher = Sha256::new();
    for file in files {
        let file = Utf8PathBuf::from_path_buf(file)
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        let rel = file.strip_prefix(path)?;
        hasher.update(rel.as_str().as_bytes());
        hasher.update([0]);
        let metadata = fs::symlink_metadata(&file)?;
        if metadata.file_type().is_symlink() {
            hasher.update(b"symlink");
            hasher.update(fs::read_link(&file)?.to_string_lossy().as_bytes());
        } else {
            hasher.update(b"file");
            let mut f = fs::File::open(&file)?;
            let mut buf = [0; 8192];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn copy_dir(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest).with_context(|| format!("failed to remove {dest}"))?;
    }
    fs::create_dir_all(dest).with_context(|| format!("failed to create {dest}"))?;

    for entry in WalkDir::new(src).follow_links(false).min_depth(1) {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        let rel = path.strip_prefix(src)?;
        let out = dest.join(rel);
        let metadata = fs::symlink_metadata(&path)?;

        if metadata.file_type().is_dir() {
            fs::create_dir_all(&out)?;
            fs::set_permissions(
                &out,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            )?;
        } else if metadata.file_type().is_symlink() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            unix_fs::symlink(fs::read_link(&path)?, &out)?;
            preserve_symlink_times(&path, &out)?;
        } else if metadata.file_type().is_file() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&path, &out)?;
            fs::set_permissions(
                &out,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            )?;
            preserve_file_times(&metadata, &out)?;
        }
    }
    preserve_dir_times(src, dest)?;
    Ok(())
}

#[allow(dead_code)]
pub fn hardlink_dir(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    let dest_parent = dest.parent().context("destination path has no parent")?;
    let name = dest
        .file_name()
        .context("destination path has no file name")?;
    let staging_parent = dest_parent.join(STAGING_DIR);
    let staging = staging_parent.join(format!("{name}-{}", std::process::id()));

    if path_exists_no_follow(&staging)? {
        remove_existing_path(&staging)
            .with_context(|| format!("failed to remove stale staging directory {staging}"))?;
    }
    fs::create_dir_all(&staging_parent)
        .with_context(|| format!("failed to create staging parent {staging_parent}"))?;

    let stage_result = build_hardlink_dir(src, &staging);
    if let Err(err) = stage_result {
        let _ = remove_existing_path(&staging);
        return Err(err).with_context(|| format!("failed to stage hardlinked skill from {src}"));
    }

    if path_exists_no_follow(dest)? {
        remove_existing_path(dest)
            .with_context(|| format!("failed to remove existing destination {dest}"))?;
    }
    fs::rename(&staging, dest)
        .with_context(|| format!("failed to replace hardlink destination {dest}"))?;
    Ok(())
}

fn build_hardlink_dir(src: &Utf8Path, staging: &Utf8Path) -> Result<()> {
    let src_metadata = fs::symlink_metadata(src)?;
    fs::create_dir_all(staging).with_context(|| format!("failed to create {staging}"))?;
    fs::set_permissions(
        staging,
        fs::Permissions::from_mode(src_metadata.permissions().mode()),
    )?;

    for entry in WalkDir::new(src).follow_links(false).min_depth(1) {
        let entry = entry?;
        let path = utf8_path(entry.path())?;
        let rel = path.strip_prefix(src)?;
        let out = staging.join(rel);
        let metadata = fs::symlink_metadata(&path)?;

        if metadata.file_type().is_dir() {
            fs::create_dir_all(&out)?;
            fs::set_permissions(
                &out,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            )?;
        } else if metadata.file_type().is_symlink() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            unix_fs::symlink(fs::read_link(&path)?, &out)?;
            preserve_symlink_times(&path, &out)?;
        } else if metadata.file_type().is_file() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::hard_link(&path, &out).map_err(|err| map_link_err(err, &path, &out))?;
        }
    }
    preserve_dir_times(src, staging)?;
    Ok(())
}

#[allow(dead_code)]
pub fn hardlink_dir_status(canonical: &Utf8Path, dest: &Utf8Path) -> Result<HardlinkStatus> {
    let dest_metadata = match fs::symlink_metadata(dest) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(HardlinkStatus::Missing),
        Err(err) => return Err(err.into()),
    };
    if !dest_metadata.file_type().is_dir() || dest_metadata.file_type().is_symlink() {
        return Ok(HardlinkStatus::Foreign);
    }

    let canonical_entries = relative_tree_entries(canonical)?;
    let dest_entries = relative_tree_entries(dest)?;
    if canonical_entries != dest_entries
        || canonical_entries
            .values()
            .any(|kind| matches!(kind, TreeEntryKind::Other))
    {
        return Ok(HardlinkStatus::Foreign);
    }

    let mut severed = Vec::new();
    let mut diverged = Vec::new();

    for (rel, kind) in canonical_entries {
        match kind {
            TreeEntryKind::Dir => {}
            TreeEntryKind::Symlink => {
                let canonical_target = fs::read_link(canonical.join(&rel))?;
                let dest_target = fs::read_link(dest.join(&rel))?;
                if canonical_target != dest_target {
                    diverged.push(rel);
                }
            }
            TreeEntryKind::File => {
                let canonical_file = canonical.join(&rel);
                let dest_file = dest.join(&rel);
                let canonical_metadata = fs::symlink_metadata(&canonical_file)?;
                let dest_metadata = fs::symlink_metadata(&dest_file)?;
                if canonical_metadata.dev() == dest_metadata.dev()
                    && canonical_metadata.ino() == dest_metadata.ino()
                {
                    continue;
                }

                if files_equal(&canonical_file, &dest_file)? {
                    severed.push(rel);
                } else {
                    diverged.push(rel);
                }
            }
            TreeEntryKind::Other => return Ok(HardlinkStatus::Foreign),
        }
    }

    if !diverged.is_empty() {
        Ok(HardlinkStatus::Diverged { files: diverged })
    } else if !severed.is_empty() {
        Ok(HardlinkStatus::Severed { files: severed })
    } else {
        Ok(HardlinkStatus::Identical)
    }
}

#[allow(dead_code)]
pub fn relink_files(canonical: &Utf8Path, dest: &Utf8Path, files: &[Utf8PathBuf]) -> Result<()> {
    for rel in files {
        let canonical_file = canonical.join(rel);
        let dest_file = dest.join(rel);
        let metadata = fs::symlink_metadata(&canonical_file)
            .with_context(|| format!("failed to stat canonical file {canonical_file}"))?;
        if !metadata.file_type().is_file() {
            bail!("{canonical_file} is not a regular file and cannot be hardlinked");
        }
        atomic_hardlink_file(&canonical_file, &dest_file)?;
    }
    Ok(())
}

fn atomic_hardlink_file(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    let parent = dest
        .parent()
        .context("hardlink destination has no parent")?;
    fs::create_dir_all(parent)?;
    let file_name = dest
        .file_name()
        .context("hardlink destination has no file name")?;
    let temp = parent.join(format!(".{file_name}.skillnet-tmp-{}", std::process::id()));
    if path_exists_no_follow(&temp)? {
        remove_existing_path(&temp)
            .with_context(|| format!("failed to remove stale temporary hardlink {temp}"))?;
    }

    if let Err(err) = fs::hard_link(src, &temp) {
        return Err(map_link_err(err, src, &temp));
    }
    if let Err(err) = fs::rename(&temp, dest) {
        let _ = fs::remove_file(&temp);
        return Err(err).with_context(|| format!("failed to replace hardlink {dest}"));
    }
    Ok(())
}

fn map_link_err(err: io::Error, src: &Utf8Path, dest: &Utf8Path) -> anyhow::Error {
    if err.raw_os_error() == Some(EXDEV) {
        anyhow::anyhow!(
            "cannot hardlink {src} -> {dest}: source and destination are on different filesystems (EXDEV); hardlinks require a shared mount"
        )
    } else {
        anyhow::Error::new(err).context(format!("cannot hardlink {src} -> {dest}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeEntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

fn relative_tree_entries(root: &Utf8Path) -> Result<BTreeMap<Utf8PathBuf, TreeEntryKind>> {
    let mut entries = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry?;
        let path = utf8_path(entry.path())?;
        if path
            .components()
            .any(|component| component.as_str() == STAGING_DIR)
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        let kind = if metadata.file_type().is_dir() {
            TreeEntryKind::Dir
        } else if metadata.file_type().is_symlink() {
            TreeEntryKind::Symlink
        } else if metadata.file_type().is_file() {
            TreeEntryKind::File
        } else {
            TreeEntryKind::Other
        };
        entries.insert(path.strip_prefix(root)?.to_path_buf(), kind);
    }
    Ok(entries)
}

fn files_equal(left: &Utf8Path, right: &Utf8Path) -> Result<bool> {
    let left_metadata = fs::symlink_metadata(left)?;
    let right_metadata = fs::symlink_metadata(right)?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }

    let mut left_file = fs::File::open(left)?;
    let mut right_file = fs::File::open(right)?;
    let mut left_buf = [0; 8192];
    let mut right_buf = [0; 8192];
    loop {
        let left_n = left_file.read(&mut left_buf)?;
        let right_n = right_file.read(&mut right_buf)?;
        if left_n != right_n {
            return Ok(false);
        }
        if left_n == 0 {
            return Ok(true);
        }
        if left_buf[..left_n] != right_buf[..right_n] {
            return Ok(false);
        }
    }
}

fn remove_existing_path(path: &Utf8Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("failed to remove {path}"))
}

fn path_exists_no_follow(path: &Utf8Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("failed to stat {path}")),
    }
}

fn utf8_path(path: &std::path::Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))
}

fn preserve_file_times(metadata: &fs::Metadata, dest: &Utf8Path) -> Result<()> {
    set_file_times(
        dest,
        FileTime::from_last_access_time(metadata),
        FileTime::from_last_modification_time(metadata),
    )
    .with_context(|| format!("failed to preserve file times for {dest}"))
}

fn preserve_symlink_times(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    let metadata = fs::symlink_metadata(src)?;
    set_symlink_file_times(
        dest,
        FileTime::from_last_access_time(&metadata),
        FileTime::from_last_modification_time(&metadata),
    )
    .with_context(|| format!("failed to preserve symlink times for {dest}"))
}

fn preserve_dir_times(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    let mut dirs = WalkDir::new(src)
        .follow_links(false)
        .contents_first(true)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    dirs.sort_by_key(|entry| std::cmp::Reverse(entry.depth()));
    for entry in dirs {
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path().to_path_buf())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in skill tree: {}", p.display()))?;
        let rel = path.strip_prefix(src)?;
        let out = dest.join(rel);
        preserve_file_times(&fs::symlink_metadata(&path)?, &out)?;
    }
    Ok(())
}

#[cfg(test)]
pub fn remove_codex_skills(skills_path: &Utf8Path) -> Result<()> {
    if skills_path.exists() {
        fs::remove_dir_all(skills_path)?;
    }
    if let Some(codex_dir) = skills_path.parent() {
        if codex_dir.is_dir() && fs::read_dir(codex_dir)?.next().is_none() {
            fs::remove_dir(codex_dir)?;
        }
    }
    Ok(())
}

pub fn ensure_skill_dir(path: &Utf8Path) -> Result<()> {
    if !path.join("SKILL.md").is_file() {
        bail!("{path} is not a skill directory: missing SKILL.md");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    use tempfile::tempdir;

    fn utf8_temp_path(tmp: &tempfile::TempDir) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap()
    }

    fn write_skill_file(path: &Utf8Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
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

    fn sample_skill(root: &Utf8Path) -> Utf8PathBuf {
        let src = root.join("canonical");
        write_skill_file(&src.join("SKILL.md"), "# Skill\n");
        write_skill_file(&src.join("nested/data.txt"), "shared\n");
        unix_fs::symlink("../SKILL.md", src.join("nested/link.md")).unwrap();
        src
    }

    #[test]
    fn removes_empty_codex_parent_only() {
        let tmp = tempdir().unwrap();
        let skills = Utf8PathBuf::from_path_buf(tmp.path().join(".codex/skills")).unwrap();
        fs::create_dir_all(&skills).unwrap();
        remove_codex_skills(&skills).unwrap();
        assert!(!skills.exists());
        assert!(!skills.parent().unwrap().exists());
    }

    #[test]
    fn preserves_non_empty_codex_parent() {
        let tmp = tempdir().unwrap();
        let codex = Utf8PathBuf::from_path_buf(tmp.path().join(".codex")).unwrap();
        let skills = codex.join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(codex.join("config.toml"), "").unwrap();
        remove_codex_skills(&skills).unwrap();
        assert!(!skills.exists());
        assert!(codex.exists());
    }

    #[test]
    fn preserves_file_codex_parent() {
        let tmp = tempdir().unwrap();
        let codex = Utf8PathBuf::from_path_buf(tmp.path().join(".codex")).unwrap();
        let skills = codex.join("skills");
        fs::write(&codex, "").unwrap();
        remove_codex_skills(&skills).unwrap();
        assert!(codex.is_file());
    }

    #[test]
    fn hardlink_dir_links_regular_files_and_recreates_dirs_and_symlinks() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = sample_skill(&root);
        let dest = root.join("dest");

        hardlink_dir(&src, &dest).unwrap();

        assert!(dest.join("nested").is_dir());
        assert_same_inode(&src.join("SKILL.md"), &dest.join("SKILL.md"));
        assert_same_inode(&src.join("nested/data.txt"), &dest.join("nested/data.txt"));

        let dest_link = dest.join("nested/link.md");
        let link_metadata = fs::symlink_metadata(&dest_link).unwrap();
        assert!(link_metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_link(dest_link).unwrap(),
            std::path::Path::new("../SKILL.md")
        );
    }

    #[test]
    fn write_through_dest_hardlink_is_visible_from_source() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = sample_skill(&root);
        let dest = root.join("dest");
        hardlink_dir(&src, &dest).unwrap();

        fs::write(dest.join("nested/data.txt"), "edited through dest\n").unwrap();

        assert_eq!(
            fs::read_to_string(src.join("nested/data.txt")).unwrap(),
            "edited through dest\n"
        );
    }

    #[test]
    fn hardlink_dir_status_classifies_missing_identical_severed_and_diverged() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = sample_skill(&root);
        let dest = root.join("dest");

        assert_eq!(
            hardlink_dir_status(&src, &root.join("missing")).unwrap(),
            HardlinkStatus::Missing
        );

        hardlink_dir(&src, &dest).unwrap();
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Identical
        );

        fs::remove_file(dest.join("nested/data.txt")).unwrap();
        fs::copy(src.join("nested/data.txt"), dest.join("nested/data.txt")).unwrap();
        assert_different_inode(&src.join("nested/data.txt"), &dest.join("nested/data.txt"));
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Severed {
                files: vec![Utf8PathBuf::from("nested/data.txt")]
            }
        );

        fs::write(dest.join("nested/data.txt"), "different\n").unwrap();
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Diverged {
                files: vec![Utf8PathBuf::from("nested/data.txt")]
            }
        );
    }

    #[test]
    fn hardlink_dir_status_classifies_foreign_destinations() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = sample_skill(&root);
        let dest = root.join("dest");

        fs::write(&dest, "not a directory").unwrap();
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Foreign
        );

        fs::remove_file(&dest).unwrap();
        hardlink_dir(&src, &dest).unwrap();
        fs::write(dest.join("extra.txt"), "extra").unwrap();
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Foreign
        );

        fs::remove_file(dest.join("extra.txt")).unwrap();
        fs::remove_file(dest.join("SKILL.md")).unwrap();
        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Foreign
        );
    }

    #[test]
    fn relink_files_restores_severed_files_to_identical() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = sample_skill(&root);
        let dest = root.join("dest");
        hardlink_dir(&src, &dest).unwrap();

        fs::remove_file(dest.join("nested/data.txt")).unwrap();
        fs::copy(src.join("nested/data.txt"), dest.join("nested/data.txt")).unwrap();
        let status = hardlink_dir_status(&src, &dest).unwrap();
        let HardlinkStatus::Severed { files } = status else {
            panic!("expected severed status, got {status:?}");
        };

        relink_files(&src, &dest, &files).unwrap();

        assert_eq!(
            hardlink_dir_status(&src, &dest).unwrap(),
            HardlinkStatus::Identical
        );
        assert_same_inode(&src.join("nested/data.txt"), &dest.join("nested/data.txt"));
    }

    #[test]
    fn exdev_mapping_names_paths_and_different_filesystems() {
        // Creating a real EXDEV in a unit test would require provisioning a
        // second filesystem, so this verifies the errno mapping directly.
        let err = map_link_err(
            io::Error::from_raw_os_error(EXDEV),
            Utf8Path::new("/src/file"),
            Utf8Path::new("/dest/file"),
        );
        let message = err.to_string();

        assert!(message.contains("/src/file"));
        assert!(message.contains("/dest/file"));
        assert!(message.contains("different filesystems"));
    }

    #[test]
    fn hardlink_dir_failure_cleans_staging_without_partial_dest() {
        let tmp = tempdir().unwrap();
        let root = utf8_temp_path(&tmp);
        let src = root.join("canonical");
        write_skill_file(&src.join("ok.txt"), "ok");

        let invalid_name = OsString::from_vec(vec![0xff, b'b', b'a', b'd']);
        fs::write(src.as_std_path().join(invalid_name), "bad").unwrap();

        let dest = root.join("dest");
        let err = hardlink_dir(&src, &dest).unwrap_err();

        assert!(err.to_string().contains("failed to stage hardlinked skill"));
        assert!(!dest.exists());
        assert!(!root
            .join(STAGING_DIR)
            .join(format!("dest-{}", std::process::id()))
            .exists());
    }
}
