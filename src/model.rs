use camino::Utf8PathBuf;

use crate::link::LinkStrategy;

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub scope: TargetScope,
    pub link_strategy: LinkStrategy,
    pub canonical_path: Utf8PathBuf,
    pub views: Vec<ViewTarget>,
    pub aggregator_path: Option<Utf8PathBuf>,
    pub project_root: Option<Utf8PathBuf>,
    pub canonical_rel: Option<String>,
    pub origin: Option<String>,
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
