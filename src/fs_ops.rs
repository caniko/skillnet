use std::{
    fs,
    io::Read,
    os::unix::fs as unix_fs,
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

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
        } else if metadata.file_type().is_file() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&path, &out)?;
            fs::set_permissions(
                &out,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            )?;
        }
    }
    Ok(())
}

pub fn replace_dir(src: &Utf8Path, dest: &Utf8Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        fs::remove_dir_all(dest).with_context(|| format!("failed to remove {dest}"))?;
    }
    fs::rename(src, dest).with_context(|| format!("failed to move {src} to {dest}"))
}

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
    use tempfile::tempdir;

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
}
