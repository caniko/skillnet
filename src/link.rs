use serde::Deserialize;

use crate::{
    config::{Config, ProjectConfig},
    model::TargetScope,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkStrategy {
    Symlink,
    Hardlink,
}

impl LinkStrategy {
    pub fn default_for(scope: TargetScope) -> Self {
        match scope {
            TargetScope::Global => Self::Symlink,
            TargetScope::Project => Self::Hardlink,
        }
    }
}

pub fn resolve_link_strategy(
    scope: TargetScope,
    config: &Config,
    project: Option<&ProjectConfig>,
    cli: Option<LinkStrategy>,
) -> LinkStrategy {
    cli.or_else(|| {
        (scope == TargetScope::Project)
            .then(|| project.and_then(|project| project.link_strategy))
            .flatten()
    })
    .or(config.link_strategy)
    .unwrap_or_else(|| LinkStrategy::default_for(scope))
}
