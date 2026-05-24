use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanInputs {
    pub path: PathBuf,
    pub name: String,
    pub flavor: String,
    pub worktype: Option<String>,
    pub phase_count: u32,
    pub wave_count: u32,
    pub max_chain_depth: u32,
    pub repo_spread: u32,
    pub routing_dist: BTreeMap<String, u32>,
    pub phases: Vec<PhaseInputs>,
    pub waves: Vec<Vec<u32>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhaseInputs {
    pub ordinal: u32,
    pub slug: String,
    pub routing_tier: String,
    pub files: Vec<String>,
    pub working_tree: Option<String>,
    pub depends_on: Vec<u32>,
}
