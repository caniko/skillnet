use std::fs;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};

pub fn mirror_skill_dirs(mirror: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    if !mirror.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(mirror)? {
        let entry = entry?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in mirror: {}", p.display()))?;
        if path.is_dir() && path.join("SKILL.md").is_file() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mirror_skill_dirs_returns_only_skill_directories() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("alpha")).unwrap();
        fs::write(root.join("alpha/SKILL.md"), "").unwrap();
        fs::create_dir_all(root.join("not-a-skill")).unwrap();
        fs::write(root.join("file"), "").unwrap();

        let dirs = mirror_skill_dirs(&root).unwrap();

        assert_eq!(dirs, vec![root.join("alpha")]);
    }
}
