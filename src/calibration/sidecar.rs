use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

const SIDECAR_FILE: &str = ".calibration.json";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize, Serialize, Debug)]
pub struct Sidecar {
    pub schema_version: u32,
    pub plan: PlanRecord,
    pub triggers: Vec<TriggerRecord>,
    pub phases: Vec<PhaseRecord>,
    pub meta_heuristics_fired: Vec<String>,
    pub tags: BTreeMap<String, String>,
    #[serde(default)]
    pub verify: Option<VerifyRecord>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct PlanRecord {
    pub id: String,
    pub name: String,
    pub flavor: String,
    pub worktype: Option<String>,
    pub created_at: i64,
    pub phase_count: u32,
    pub wave_count: u32,
    pub max_chain_depth: u32,
    pub repo_spread: u32,
    pub routing_dist: BTreeMap<String, u32>,
    pub shape_hash: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct TriggerRecord {
    pub name: String,
    pub input_value: f64,
    pub threshold: f64,
    pub fired: bool,
    pub section_added: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct PhaseRecord {
    pub ordinal: u32,
    pub slug: String,
    pub routing_tier: String,
    pub files: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct VerifyRecord {
    pub verified_at: i64,
    pub elapsed_seconds: Option<i64>,
    pub outcome: String,
    pub phase_outcomes: BTreeMap<String, String>,
    pub emergency_changes: Option<serde_json::Value>,
    pub surprises: Option<String>,
}

impl Sidecar {
    pub fn load(plan_dir: &Path) -> anyhow::Result<Sidecar> {
        let path = plan_dir.join(SIDECAR_FILE);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("missing calibration sidecar {}", path.display()))?;
        let sidecar: Sidecar = serde_json::from_str(&raw)
            .with_context(|| format!("malformed calibration sidecar {}", path.display()))?;

        if sidecar.schema_version != SUPPORTED_SCHEMA_VERSION {
            bail!(
                "unsupported calibration sidecar schema_version {}; expected {}",
                sidecar.schema_version,
                SUPPORTED_SCHEMA_VERSION
            );
        }

        Ok(sidecar)
    }
}
