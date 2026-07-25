#![allow(dead_code)]

use std::{env, fmt, path::PathBuf, str::FromStr};

use anyhow::{anyhow, bail, Result};
use clap::builder::PossibleValuesParser;

use crate::config::{expand_path, Config};

const SCOPE_GLOBAL: &str = "global";
const SCOPE_PROJECTS: &str = "projects";
const SCOPE_ALL: &str = "all";

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
        if input == SCOPE_GLOBAL {
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
    let values = std::iter::once(SCOPE_GLOBAL.to_string())
        .chain([SCOPE_PROJECTS.to_string(), SCOPE_ALL.to_string()])
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

    if all || scope_args.is_empty() || scope_args.iter().any(|raw| raw == SCOPE_ALL) {
        return Ok(configured_scopes(config));
    }

    let has_projects_selector = scope_args.iter().any(|raw| raw == SCOPE_PROJECTS);
    let has_global_scope = scope_args.iter().any(|raw| raw == SCOPE_GLOBAL);
    if has_projects_selector && has_global_scope {
        return Ok(configured_scopes(config));
    }

    let mut scopes = Vec::with_capacity(scope_args.len());
    for raw in scope_args {
        if raw == SCOPE_PROJECTS {
            for project in &config.projects {
                let scope = Scope::Project(project.name.clone());
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
            continue;
        }

        let scope = resolve_scope(config, raw)?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}

pub fn detect_from_cwd(config: &Config) -> Option<Scope> {
    let cwd = env::current_dir().ok()?.canonicalize().ok()?;
    let mut match_candidate: Option<(&str, PathBuf)> = None;

    for project in &config.projects {
        let Ok(path) = expand_path(&project.path) else {
            continue;
        };
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !cwd.starts_with(&path) {
            continue;
        }

        match &match_candidate {
            None => match_candidate = Some((&project.name, path)),
            Some((_, current_path))
                if path.components().count() > current_path.components().count() =>
            {
                match_candidate = Some((&project.name, path));
            }
            Some((_, current_path)) if path == *current_path => {
                eprintln!(
                    "warning: multiple projects share path {}; cannot auto-detect scope",
                    path.display()
                );
                return None;
            }
            _ => {}
        }
    }

    match_candidate.map(|(name, _)| Scope::Project(name.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DatabaseConfig, GlobalConfig};

    fn test_config(project_names: &[&str]) -> Config {
        Config {
            global: GlobalConfig {
                canonical_path: None,
                views: Vec::new(),
            },
            user: None,
            data_dir: None,
            skills_root: None,
            mirror_root: None,
            database: DatabaseConfig::default(),
            subscriptions: Default::default(),
            external_manifests: Default::default(),
            link_strategy: None,
            projects: project_names
                .iter()
                .map(|name| crate::config::ProjectConfig {
                    name: (*name).to_string(),
                    path: format!("/tmp/{name}"),
                    origin: None,
                    link_strategy: None,
                    canonical_rel: ".skills".to_string(),
                    views: Vec::new(),
                })
                .collect(),
        }
    }

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

    #[test]
    fn resolves_projects_selector_to_project_scopes_only() {
        let config = test_config(&["first", "second"]);

        let scopes = resolve_scopes(&config, &["projects".to_string()], false).unwrap();

        assert_eq!(
            scopes,
            vec![
                Scope::Project("first".to_string()),
                Scope::Project("second".to_string())
            ]
        );
    }

    #[test]
    fn resolves_all_selector_like_all_flag() {
        let config = test_config(&["first", "second"]);

        let selector = resolve_scopes(&config, &["all".to_string()], false).unwrap();
        let flag = resolve_scopes(&config, &[], true).unwrap();

        assert_eq!(selector, flag);
        assert_eq!(
            selector,
            vec![
                Scope::Global,
                Scope::Project("first".to_string()),
                Scope::Project("second".to_string())
            ]
        );
    }

    #[test]
    fn resolves_projects_selector_with_global_as_all_in_configured_order() {
        let config = test_config(&["first", "second"]);

        let scopes = resolve_scopes(
            &config,
            &["projects".to_string(), "global".to_string()],
            false,
        )
        .unwrap();

        assert_eq!(
            scopes,
            resolve_scopes(&config, &["all".to_string()], false).unwrap()
        );
    }

    #[test]
    fn resolves_projects_selector_without_repeating_named_project() {
        let config = test_config(&["first", "second"]);

        let scopes = resolve_scopes(
            &config,
            &["projects".to_string(), "first".to_string()],
            false,
        )
        .unwrap();

        assert_eq!(
            scopes,
            vec![
                Scope::Project("first".to_string()),
                Scope::Project("second".to_string())
            ]
        );
    }

    #[test]
    fn scope_value_parser_accepts_selector_tokens_and_rejects_unknowns() {
        let config = test_config(&["first"]);
        let command = || {
            clap::Command::new("skillnet").arg(
                clap::Arg::new("scope")
                    .long("scope")
                    .action(clap::ArgAction::Append)
                    .value_parser(scope_value_parser(&config)),
            )
        };

        assert!(command()
            .try_get_matches_from(["skillnet", "--scope", "projects"])
            .is_ok());
        assert!(command()
            .try_get_matches_from(["skillnet", "--scope", "all"])
            .is_ok());
        assert!(command()
            .try_get_matches_from(["skillnet", "--scope", "nope"])
            .is_err());
    }
}
