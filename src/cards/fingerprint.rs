use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
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
    pub rust_local_traits: u64,
    pub rust_single_impl_traits: u64,
    pub rust_dyn_sites: u64,
    pub rust_mock_traits: u64,
    pub rust_allow_attrs: u64,
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
        let mut buf = Vec::new();
        let mut census = RustCensus::default();
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
            buf.clear();
            File::open(entry.path())?.read_to_end(&mut buf)?;
            self.rust_loc += census.scan_file(&buf);
        }
        census.finish(self);
        Ok(())
    }
}

#[derive(Default)]
struct TraitInfo {
    defs: u64,
    is_mock: bool,
}

#[derive(Default)]
struct RustCensus {
    local_traits: HashMap<String, TraitInfo>,
    impl_counts: HashMap<String, u64>,
    mock_impl_traits: HashSet<String>,
    dyn_sites: u64,
    allow_attrs: u64,
}

impl RustCensus {
    fn scan_file(&mut self, bytes: &[u8]) -> u64 {
        if bytes.is_empty() {
            return 0;
        }
        let mut state = LexState::default();
        let mut code = Vec::with_capacity(256);
        let mut pending_automock = false;
        let mut impl_buf = String::new();
        let mut impl_lines = 0_u8;
        let mut loc = 0_u64;

        let slice = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        for raw_line in slice.split(|&b| b == b'\n') {
            loc += 1;
            code.clear();
            state.strip_line(raw_line, &mut code);
            let Ok(line) = std::str::from_utf8(&code) else {
                continue;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            self.allow_attrs += count_allow_attrs(trimmed);
            self.dyn_sites += count_dyn_sites(trimmed);

            if trimmed.starts_with('#') {
                if contains_word(trimmed, "automock") {
                    pending_automock = true;
                }
                continue;
            }

            if let Some(name) = parse_trait_def(trimmed) {
                let info = self.local_traits.entry(name.to_owned()).or_default();
                info.defs += 1;
                if pending_automock || name.starts_with("Mock") || name.ends_with("Mock") {
                    info.is_mock = true;
                }
                pending_automock = false;
                continue;
            }
            pending_automock = false;

            if !impl_buf.is_empty() {
                impl_buf.push(' ');
                impl_buf.push_str(trimmed);
                impl_lines += 1;
                if trimmed.contains(['{', ';']) || impl_lines >= 8 {
                    self.record_impl(&impl_buf);
                    impl_buf.clear();
                    impl_lines = 0;
                }
            } else if starts_impl_header(trimmed) {
                if trimmed.contains(['{', ';']) {
                    self.record_impl(trimmed);
                } else {
                    impl_buf.push_str(trimmed);
                    impl_lines = 1;
                }
            }
        }
        if !impl_buf.is_empty() {
            self.record_impl(&impl_buf);
        }
        loc
    }

    fn record_impl(&mut self, header: &str) {
        if let Some((trait_name, target_name)) = parse_trait_impl(header) {
            *self.impl_counts.entry(trait_name.to_owned()).or_default() += 1;
            if target_name.is_some_and(is_mock_target) {
                self.mock_impl_traits.insert(trait_name.to_owned());
            }
        }
    }

    fn finish(self, fp: &mut RepoFingerprint) {
        fp.rust_local_traits = self.local_traits.values().map(|info| info.defs).sum();
        fp.rust_single_impl_traits = self
            .local_traits
            .keys()
            .filter(|name| self.impl_counts.get(*name).copied().unwrap_or(0) == 1)
            .count() as u64;
        fp.rust_mock_traits = self
            .local_traits
            .iter()
            .filter(|(name, info)| info.is_mock || self.mock_impl_traits.contains(*name))
            .count() as u64;
        fp.rust_dyn_sites = self.dyn_sites;
        fp.rust_allow_attrs = self.allow_attrs;
    }
}

#[derive(Default)]
struct LexState {
    block_comment: usize,
    in_string: bool,
    raw_hashes: Option<usize>,
}

impl LexState {
    fn strip_line(&mut self, line: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        let len = line.len();
        while i < len {
            if self.block_comment > 0 {
                if i + 1 < len && line[i] == b'/' && line[i + 1] == b'*' {
                    self.block_comment += 1;
                    i += 2;
                } else if i + 1 < len && line[i] == b'*' && line[i + 1] == b'/' {
                    self.block_comment -= 1;
                    out.push(b' ');
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some(hashes) = self.raw_hashes {
                if line[i] == b'"'
                    && i + hashes < len
                    && line[i + 1..=i + hashes].iter().all(|&b| b == b'#')
                {
                    self.raw_hashes = None;
                    out.push(b' ');
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
                continue;
            }
            if self.in_string {
                if line[i] == b'\\' {
                    i += 2;
                } else if line[i] == b'"' {
                    self.in_string = false;
                    out.push(b' ');
                    i += 1;
                } else {
                    i += 1;
                }
                continue;
            }

            let b = line[i];
            if b == b'/' && i + 1 < len {
                if line[i + 1] == b'/' {
                    break;
                }
                if line[i + 1] == b'*' {
                    self.block_comment = 1;
                    i += 2;
                    continue;
                }
            }
            if b == b'r' || (b == b'b' && i + 1 < len && line[i + 1] == b'r') {
                let prev_ident = i > 0 && is_ident_continue(line[i - 1]);
                if !prev_ident {
                    let r_pos = if b == b'b' { i + 1 } else { i };
                    let mut j = r_pos + 1;
                    while j < len && line[j] == b'#' {
                        j += 1;
                    }
                    if j < len && line[j] == b'"' {
                        self.raw_hashes = Some(j - (r_pos + 1));
                        out.push(b' ');
                        i = j + 1;
                        continue;
                    }
                }
            }
            if b == b'"' {
                self.in_string = true;
                out.push(b' ');
                i += 1;
                continue;
            }
            if b == b'\'' {
                if let Some(end) = skip_char_literal(line, i) {
                    out.push(b' ');
                    i = end;
                    continue;
                }
            }
            out.push(b);
            i += 1;
        }
    }
}

fn skip_char_literal(line: &[u8], start: usize) -> Option<usize> {
    let rest = line.get(start + 1..)?;
    if rest.first() == Some(&b'\\') {
        let mut j = start + 2;
        while j < line.len() {
            if line[j] == b'\'' {
                return Some(j + 1);
            }
            if line[j] == b'\\' {
                j += 2;
            } else {
                j += 1;
            }
        }
        None
    } else {
        let ch_len = std::str::from_utf8(rest).ok()?.chars().next()?.len_utf8();
        if line.get(start + 1 + ch_len) == Some(&b'\'') {
            Some(start + 2 + ch_len)
        } else {
            None
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn contains_word(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let w = word.as_bytes();
    if bytes.len() < w.len() {
        return false;
    }
    let mut i = 0;
    while i + w.len() <= bytes.len() {
        if &bytes[i..i + w.len()] == w {
            let before_ok = i == 0 || !is_ident_continue(bytes[i - 1]);
            let after_ok = i + w.len() == bytes.len() || !is_ident_continue(bytes[i + w.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn count_allow_attrs(line: &str) -> u64 {
    let bytes = line.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b'!' {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'[' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if bytes[j..].starts_with(b"allow") {
                    let mut k = j + 5;
                    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k] == b'(' {
                        count += 1;
                        i = k + 1;
                        continue;
                    }
                } else if bytes[j..].starts_with(b"cfg_attr") {
                    if let Some(close) = bytes[j..].iter().position(|&b| b == b']') {
                        let attr = &line[j..j + close];
                        if contains_word(attr, "allow") && attr.contains("allow(") {
                            count += 1;
                        }
                        i = j + close + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    count
}

fn count_dyn_sites(line: &str) -> u64 {
    let bytes = line.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 3] == b"dyn"
            && (i == 0 || !is_ident_continue(bytes[i - 1]))
            && matches!(bytes[i + 3], b' ' | b'\t')
        {
            let mut j = i + 4;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
                j += 1;
            }
            if j < bytes.len() && (is_ident_start(bytes[j]) || bytes[j] == b':' || bytes[j] == b'(')
            {
                count += 1;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    count
}

fn strip_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    let first = rest.as_bytes().first().copied()?;
    if first.is_ascii_whitespace() {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn parse_trait_def(line: &str) -> Option<&str> {
    let mut s = line;
    if let Some(after_pub) = s.strip_prefix("pub") {
        let after_pub = after_pub.trim_start();
        if let Some(rest) = after_pub.strip_prefix('(') {
            let end = rest.find(')')?;
            s = rest[end + 1..].trim_start();
        } else if s.as_bytes().get(3).is_some_and(u8::is_ascii_whitespace) {
            s = after_pub;
        }
    }
    if let Some(rest) = strip_keyword(s, "unsafe") {
        s = rest;
    }
    if let Some(rest) = strip_keyword(s, "auto") {
        s = rest;
    }
    let rest = strip_keyword(s, "trait")?;
    take_ident(rest)
}

fn starts_impl_header(line: &str) -> bool {
    let s = strip_keyword(line, "unsafe").unwrap_or(line);
    s.strip_prefix("impl").is_some_and(|rest| {
        rest.as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b'<')
    })
}

fn parse_trait_impl(header: &str) -> Option<(&str, Option<&str>)> {
    let s = strip_keyword(header, "unsafe").unwrap_or(header);
    let mut s = s.strip_prefix("impl")?.trim_start();
    if s.starts_with('<') {
        s = skip_balanced(s, b'<', b'>')?.trim_start();
    }
    if let Some(rest) = s.strip_prefix('!') {
        s = rest.trim_start();
    }
    let (trait_part, target_part) = split_top_level_for(s)?;
    let trait_name = last_path_ident(trait_part)?;
    let target_name = extract_target_ident(target_part);
    Some((trait_name, target_name))
}

fn skip_balanced(s: &str, open: u8, close: u8) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&open) {
        return None;
    }
    let mut depth = 0_usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(&s[i + 1..]);
            }
        }
    }
    None
}

fn split_top_level_for(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut angle = 0_usize;
    let mut paren = 0_usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => angle += 1,
            b'>' => angle = angle.saturating_sub(1),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'{' | b';' if angle == 0 && paren == 0 => return None,
            b'f' if angle == 0 && paren == 0 && bytes[i..].starts_with(b"for") => {
                let before_ok = i > 0 && !is_ident_continue(bytes[i - 1]);
                let after_ok = i + 3 < bytes.len() && bytes[i + 3].is_ascii_whitespace();
                if before_ok && after_ok {
                    return Some((s[..i].trim(), s[i + 3..].trim()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn last_path_ident(path: &str) -> Option<&str> {
    let head = path.split('<').next()?.trim();
    let last = head.rsplit("::").next()?.trim();
    take_ident(last)
}

fn extract_target_ident(mut target: &str) -> Option<&str> {
    loop {
        target = target.trim_start();
        if let Some(rest) = target.strip_prefix('&') {
            target = rest.trim_start();
            if target.starts_with('\'') {
                let end = target
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '\'')
                    .unwrap_or(target.len());
                target = &target[end..];
            }
            if let Some(rest) = strip_keyword(target.trim_start(), "mut") {
                target = rest;
            }
        } else if let Some(rest) = target.strip_prefix('*') {
            let rest = rest.trim_start();
            target = strip_keyword(rest, "const")
                .or_else(|| strip_keyword(rest, "mut"))
                .unwrap_or(rest);
        } else if let Some(rest) = strip_keyword(target, "dyn") {
            target = rest;
        } else {
            break;
        }
    }
    let head = target.split(['<', '{', ';', ' ', '\t']).next()?.trim();
    let last = head.rsplit("::").next()?.trim();
    take_ident(last)
}

fn take_ident(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if !bytes.first().copied().is_some_and(is_ident_start) {
        return None;
    }
    let end = bytes
        .iter()
        .position(|&b| !is_ident_continue(b))
        .unwrap_or(bytes.len());
    Some(&s[..end])
}

fn is_mock_target(name: &str) -> bool {
    name.starts_with("Mock") || name.starts_with("Fake") || name.starts_with("Stub")
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
