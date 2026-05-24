use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub scope: TargetScope,
    pub canonical_path: Utf8PathBuf,
    pub views: Vec<ViewTarget>,
    pub aggregator_path: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ViewTarget {
    pub label: String,
    pub path: Utf8PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetScope {
    Global,
    Project,
}

#[allow(dead_code)]
#[deprecated(note = "P4 deletes reconcile.rs and removes pre-Option-B source arbitration")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub label: String,
    pub path: Utf8PathBuf,
    pub priority: i64,
}

#[allow(dead_code)]
#[deprecated(note = "P4 deletes reconcile.rs and removes pre-Option-B source arbitration")]
#[derive(Debug, Clone)]
pub struct Candidate {
    pub skill: String,
    pub source: String,
    pub priority: i64,
    pub path: Utf8PathBuf,
    pub newest_mtime_nanos: u128,
    pub content_signature: String,
}

#[allow(dead_code)]
#[deprecated(note = "P4 deletes reconcile.rs and removes pre-Option-B source arbitration")]
#[derive(Debug, Clone)]
pub struct Choice {
    pub skill: String,
    pub source: String,
    pub path: Utf8PathBuf,
    pub newest_mtime_nanos: u128,
    pub candidate_count: usize,
}
