#![allow(dead_code)]

use std::{fmt, str::FromStr};

use anyhow::{anyhow, bail, Result};
use clap::builder::PossibleValuesParser;

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Project(String),
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Global => f.write_str("global"),
            Self::Project(project) => f.write_str(project),
        }
    }
}

impl FromStr for Scope {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.is_empty() {
            bail!("scope cannot be empty");
        }
        if input == "global" {
            Ok(Self::Global)
        } else {
            Ok(Self::Project(input.to_string()))
        }
    }
}

/// Build the runtime `--scope` value parser after `Config::load`.
///
/// Project names are configuration-derived, so Phase 02 should attach this
/// parser after loading config instead of trying to express it in clap derive
/// attributes.
pub fn scope_value_parser(config: &Config) -> PossibleValuesParser {
    let values = std::iter::once("global".to_string())
        .chain(config.projects.iter().map(|project| project.name.clone()))
        .map(|value| Box::leak(value.into_boxed_str()) as &'static str)
        .collect::<Vec<_>>();
    PossibleValuesParser::new(values)
}

pub fn configured_scopes(config: &Config) -> Vec<Scope> {
    std::iter::once(Scope::Global)
        .chain(
            config
                .projects
                .iter()
                .map(|project| Scope::Project(project.name.clone())),
        )
        .collect()
}

pub fn resolve_scope(config: &Config, raw: &str) -> Result<Scope> {
    let scope = Scope::from_str(raw)?;
    let valid_scopes = configured_scopes(config);
    if valid_scopes.contains(&scope) {
        return Ok(scope);
    }

    bail!(
        "unknown scope `{raw}`; valid scopes are: {}",
        valid_scopes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
}

pub fn resolve_scopes(config: &Config, scope_args: &[String], all: bool) -> Result<Vec<Scope>> {
    if all && !scope_args.is_empty() {
        bail!("use either --all or --scope, not both");
    }

    if all || scope_args.is_empty() {
        return Ok(configured_scopes(config));
    }

    let mut scopes = Vec::with_capacity(scope_args.len());
    for raw in scope_args {
        let scope = resolve_scope(config, raw)?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPath {
    pub scope: Scope,
    pub skill: String,
}

impl SkillPath {
    pub fn parse(input: &str, valid_scopes: &[Scope]) -> Result<Self> {
        if input.contains('\\') {
            bail!("skill path `{input}` must use `/` as the scope separator");
        }

        let (scope_raw, skill) = input
            .split_once('/')
            .ok_or_else(|| anyhow!("skill path `{input}` is missing `/` separator"))?;

        if skill.is_empty() {
            bail!("skill path `{input}` has an empty skill name");
        }

        let scope = Scope::from_str(scope_raw)?;
        if !valid_scopes.contains(&scope) {
            bail!("unknown scope `{scope_raw}` in skill path `{input}`");
        }

        Ok(Self {
            scope,
            skill: skill.to_string(),
        })
    }
}

// TODO: The old `--target project` selector meant every project and no global.
// If that behavior is missed in the new surface, prefer repeated
// `--scope <project>` values or a future `--projects` flag.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_global_skill_path() {
        let parsed = SkillPath::parse("global/foo", &[Scope::Global]).unwrap();
        assert_eq!(
            parsed,
            SkillPath {
                scope: Scope::Global,
                skill: "foo".to_string(),
            }
        );
    }

    #[test]
    fn rejects_unknown_scope() {
        let err = SkillPath::parse("nope/foo", &[Scope::Global]).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn rejects_empty_skill() {
        let err = SkillPath::parse("global/", &[Scope::Global]).unwrap_err();
        assert!(err.to_string().contains("empty skill"));
    }

    #[test]
    fn rejects_missing_separator() {
        let err = SkillPath::parse("global", &[Scope::Global]).unwrap_err();
        assert!(err.to_string().contains("missing `/`"));
    }
}
