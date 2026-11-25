use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    process::Command,
};

use anyhow::{ensure, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Fingerprint attributes are independent predicates in the JSON schema"
)]
pub(crate) struct RepoFingerprint {
    pub root: Utf8PathBuf,
    pub languages: Vec<String>,
    pub has_rust: bool,
    pub has_nix: bool,
    pub has_python: bool,
    pub has_typescript: bool,
    pub rust_files: u64,
    pub rust_loc: u64,
    pub rust_has_workspace: bool,
    pub forge: Forge,
    pub has_forgejo: bool,
    pub has_github_workflows: bool,
    pub has_book_toml: bool,
    pub has_simit_toml: bool,
    pub has_plinth: bool,
    pub has_chaosbox: bool,
    pub is_nixpkgs: bool,
    pub has_openpencil: bool,
    pub has_3d_assets: bool,
    pub recommended_packs: Vec<String>,
    pub projected_skills: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Forge {
    Codeberg,
    Github,
    Gitlab,
    Selfhosted,
    #[default]
    None,
}

pub(crate) fn repo_root(cwd: &Utf8Path) -> Result<Utf8PathBuf> {
    Ok(git_root(&resolve_cwd(cwd)?).to_owned())
}

fn resolve_cwd(cwd: &Utf8Path) -> Result<Utf8PathBuf> {
    let cwd = cwd
        .canonicalize_utf8()
        .with_context(|| format!("resolve {cwd}"))?;
    ensure!(cwd.is_dir(), "{cwd} is not a directory");
    Ok(cwd)
}

fn git_root(cwd: &Utf8Path) -> &Utf8Path {
    cwd.ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(cwd)
}

fn keep_entry(entry: &DirEntry) -> bool {
    !entry.file_type().is_symlink()
        && (entry.depth() == 0
            || !entry.file_type().is_dir()
            || (!entry.file_name().as_encoded_bytes().starts_with(b".")
                && !matches!(
                    entry.file_name().to_str(),
                    Some("target" | "node_modules" | "vendor" | "dist" | "build" | "result")
                )
                && entry.path().join(".git").symlink_metadata().is_err()))
}

fn cargo_root(cwd: &Utf8Path, root: &Utf8Path) -> Result<Option<Utf8PathBuf>> {
    let has_manifest = |path: &std::path::Path| {
        path.join("Cargo.toml")
            .symlink_metadata()
            .is_ok_and(|meta| meta.is_file())
    };
    if let Some(path) = cwd
        .ancestors()
        .take_while(|p| p.starts_with(root))
        .find(|p| has_manifest(p.as_std_path()))
    {
        return Ok(Some(path.to_owned()));
    }
    // A monorepo may keep its Cargo workspace one level below its Git root.
    for entry in WalkDir::new(root)
        .max_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(keep_entry)
    {
        let entry = entry?;
        if entry.depth() == 1 && entry.file_type().is_dir() && has_manifest(entry.path()) {
            return Ok(Some(Utf8PathBuf::try_from(entry.into_path())?));
        }
    }
    Ok(None)
}

impl RepoFingerprint {
    pub(crate) fn detect(cwd: &Utf8Path) -> Result<Self> {
        let cwd = resolve_cwd(cwd)?;
        let root = git_root(&cwd).to_owned();
        let cargo_root = cargo_root(&cwd, &root)?;
        let file = |name: &str| root.join(name).is_file();
        let dir = |name: &str| root.join(name).is_dir();
        let mut fp = Self {
            has_rust: cargo_root.is_some(),
            has_nix: file("flake.nix") || file("default.nix") || file("shell.nix"),
            has_python: file("pyproject.toml")
                || file("requirements.txt")
                || file("setup.py")
                || file("setup.cfg"),
            has_typescript: file("tsconfig.json"),
            forge: detect_forge(&root)?,
            has_forgejo: dir(".forgejo"),
            has_github_workflows: dir(".github/workflows"),
            has_book_toml: file("book.toml"),
            has_simit_toml: file("simit.toml"),
            has_plinth: file("website/plinth-project.toml"),
            has_chaosbox: file(".chaosbox.json"),
            is_nixpkgs: file("pkgs/top-level/all-packages.nix"),
            root,
            ..Self::default()
        };

        if let Some(cargo_root) = cargo_root {
            fp.scan_rust(&cargo_root)?;
        }

        for entry in WalkDir::new(&fp.root)
            .max_depth(3)
            .into_iter()
            .filter_entry(keep_entry)
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            match entry.path().extension().and_then(|ext| ext.to_str()) {
                Some("rs") => fp.has_rust = true,
                Some("nix") => fp.has_nix = true,
                Some("py") => fp.has_python = true,
                Some("ts" | "tsx" | "mts" | "cts") => fp.has_typescript = true,
                Some("fig") => fp.has_openpencil = true,
                Some("blend" | "glb") => fp.has_3d_assets = true,
                _ => {}
            }
        }

        for (detected, language) in [
            (fp.has_rust, "rust"),
            (fp.has_nix, "nix"),
            (fp.has_python, "python"),
            (fp.has_typescript, "typescript"),
        ] {
            if detected {
                fp.languages.push(language.to_owned());
            }
        }
        for (detected, pack) in [
            (fp.has_rust, "rust"),
            (fp.has_nix, "nix"),
            (
                fp.forge != Forge::None || fp.has_forgejo || fp.has_github_workflows,
                "forge",
            ),
            (true, "gates"),
        ] {
            if detected {
                fp.recommended_packs.push(pack.to_owned());
            }
        }
        for (detected, skills) in [
            (fp.is_nixpkgs, &["nixpkgs-pr", "nixpkgs-pr-review"][..]),
            (fp.has_plinth, &["plinth-project-visual-review"][..]),
            (
                fp.has_openpencil,
                &["openpencil-ui-parity", "openpencil-designer"][..],
            ),
            (fp.has_3d_assets, &["trellis-3d-asset"][..]),
            (fp.has_chaosbox, &["chaosbox"][..]),
        ] {
            if detected {
                fp.projected_skills
                    .extend(skills.iter().map(|s| (*s).to_owned()));
            }
        }
        Ok(fp)
    }

    fn scan_rust(&mut self, root: &Utf8Path) -> Result<()> {
        let workspace = fs::read_to_string(root.join("Cargo.toml"))
            .ok()
            .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
            .is_some_and(|manifest| manifest.get("workspace").is_some_and(toml::Value::is_table));
        self.rust_has_workspace = workspace;
        let walk = WalkDir::new(root);
        let walk = if workspace { walk.max_depth(8) } else { walk };
        let mut line = Vec::new();
        for entry in walk.into_iter().filter_entry(|entry| {
            let wanted = if workspace {
                entry.depth() == 0
                    || !entry.file_type().is_dir()
                    || !matches!(
                        entry.file_name().to_str(),
                        Some("tests" | "benches" | "examples")
                    )
            } else {
                entry.depth() != 1 || (entry.file_type().is_dir() && entry.file_name() == "src")
            };
            wanted && keep_entry(entry)
        }) {
            let entry = entry?;
            if !entry.file_type().is_file()
                || entry.path().extension().is_none_or(|ext| ext != "rs")
                || (workspace
                    && !entry
                        .path()
                        .ancestors()
                        .skip(1)
                        .take_while(|parent| *parent != root.as_std_path())
                        .any(|parent| parent.file_name().is_some_and(|name| name == "src")))
            {
                continue;
            }
            self.rust_files += 1;
            let mut reader = BufReader::new(File::open(entry.path())?);
            while reader.read_until(b'\n', &mut line)? != 0 {
                self.rust_loc += 1;
                line.clear();
            }
        }
        Ok(())
    }
}

fn detect_forge(root: &Utf8Path) -> Result<Forge> {
    let marker = root.join(".git");
    let git_dir = if marker.is_file() {
        let link = fs::read_to_string(&marker)?;
        let git_dir = link
            .trim()
            .strip_prefix("gitdir:")
            .context("invalid .git file")?;
        root.join(git_dir.trim())
    } else {
        marker
    };
    let common = git_dir.join("commondir");
    let config = if common.is_file() {
        git_dir
            .join(fs::read_to_string(common)?.trim())
            .join("config")
    } else {
        git_dir.join("config")
    };
    if !config.is_file() {
        return Ok(Forge::None);
    }
    let output = Command::new("git")
        .args([
            "config",
            "--no-includes",
            "--file",
            config.as_str(),
            "--null",
            "--get-regexp",
            r"^remote\..*\.url$",
        ])
        .output()
        .context("read Git remote URLs")?;
    if output.status.code() == Some(1) {
        return Ok(Forge::None);
    }
    ensure!(
        output.status.success(),
        "could not read Git remotes from {config}"
    );
    let urls = String::from_utf8_lossy(&output.stdout);
    Ok(urls
        .split('\0')
        .filter_map(|row| row.split_once('\n'))
        .filter_map(|(key, url)| remote_forge(url).map(|forge| (key != "remote.origin.url", forge)))
        .min_by_key(|&(not_origin, _)| not_origin)
        .map_or(Forge::None, |(_, forge)| forge))
}

fn remote_forge(url: &str) -> Option<Forge> {
    let authority = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        rest.split('/').next()?
    } else {
        let (host, _) = url.split_once(':')?;
        if host.contains(['/', '\\']) {
            return None;
        }
        host
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    match host.to_ascii_lowercase().as_str() {
        "" => None,
        "codeberg.org" => Some(Forge::Codeberg),
        "github.com" | "ssh.github.com" => Some(Forge::Github),
        "gitlab.com" | "altssh.gitlab.com" => Some(Forge::Gitlab),
        _ => Some(Forge::Selfhosted),
    }
}
