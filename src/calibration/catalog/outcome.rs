use super::thresholds::ThresholdSource;
use crate::calibration::sidecar::TriggerRecord;

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerOutcome {
    pub input_value: f64,
    pub threshold: f64,
    pub fired: bool,
    pub section_added: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NamedTriggerOutcome {
    pub name: String,
    pub input_value: f64,
    pub default_threshold: f64,
    pub threshold: f64,
    pub threshold_source: ThresholdSource,
    pub fired: bool,
    pub section_added: Option<String>,
}

impl From<NamedTriggerOutcome> for TriggerRecord {
    fn from(outcome: NamedTriggerOutcome) -> Self {
        Self {
            name: outcome.name,
            input_value: outcome.input_value,
            threshold: outcome.threshold,
            fired: outcome.fired,
            section_added: outcome.section_added,
        }
    }
}
