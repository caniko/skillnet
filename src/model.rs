use camino::Utf8PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub label: String,
    pub path: Utf8PathBuf,
    pub priority: i64,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub mirror_path: Utf8PathBuf,
    pub sources: Vec<Source>,
    pub sync_paths: Vec<Utf8PathBuf>,
    pub stale_codex_skill_paths: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub skill: String,
    pub source: String,
    pub priority: i64,
    pub path: Utf8PathBuf,
    pub newest_mtime_nanos: u128,
    pub content_signature: String,
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub skill: String,
    pub source: String,
    pub path: Utf8PathBuf,
    pub newest_mtime_nanos: u128,
    pub candidate_count: usize,
}
