use std::collections::{BTreeMap, BTreeSet};

use super::config::{
    CatalogConfig, VALID_GLOBAL_CATEGORIES, VALID_PROJECT_CATEGORIES, VALID_SCOPES, VALID_STATUSES,
};
use super::entry::SkillEntry;

pub(super) fn validate_entries(entries: &[SkillEntry], config: &CatalogConfig) -> Vec<String> {
    let mut errors = Vec::new();
    let categories = VALID_GLOBAL_CATEGORIES
        .iter()
        .chain(VALID_PROJECT_CATEGORIES.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    let qualified_names = entries
        .iter()
        .map(|entry| entry.qualified_name.as_str())
        .collect::<BTreeSet<_>>();
    let names = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();

    for entry in entries {
        if entry.description.is_empty() {
            errors.push(format!("{}: missing description", entry.qualified_name));
        }
        match &entry.category {
            Some(category) if categories.contains(category.as_str()) => {}
            Some(category) => errors.push(format!(
                "{}: unknown category `{category}`",
                entry.qualified_name
            )),
            None => errors.push(format!("{}: missing category", entry.qualified_name)),
        }
        if !VALID_SCOPES.contains(&entry.scope.as_str()) {
            errors.push(format!(
                "{}: unknown scope `{}`",
                entry.qualified_name, entry.scope
            ));
        }
        if !VALID_STATUSES.contains(&entry.status.as_str()) {
            errors.push(format!(
                "{}: unknown status `{}`",
                entry.qualified_name, entry.status
            ));
        }
        for related in &entry.related_skills {
            if !qualified_names.contains(related.as_str()) && !names.contains(related.as_str()) {
                errors.push(format!(
                    "{}: related skill `{related}` does not exist",
                    entry.qualified_name
                ));
            }
        }
        if entry.line_count > config.settings.large_skill_line_threshold
            && !entry.tags.iter().any(|tag| tag == "large-skill")
        {
            errors.push(format!(
                "{}: large skill has {} lines but is not tagged `large-skill`",
                entry.qualified_name, entry.line_count
            ));
        }
    }

    for (name, duplicates) in duplicates_by_name(entries) {
        if duplicates.len() > 1
            && duplicates
                .iter()
                .all(|entry| entry.collision_note.is_none())
        {
            errors.push(format!(
                "duplicate skill `{name}` lacks collision notes: {}",
                duplicates
                    .iter()
                    .map(|entry| entry.qualified_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    errors
}

pub(super) fn duplicates_by_name(entries: &[SkillEntry]) -> BTreeMap<&str, Vec<&SkillEntry>> {
    let mut by_name = BTreeMap::<&str, Vec<&SkillEntry>>::new();
    for entry in entries {
        by_name.entry(entry.name.as_str()).or_default().push(entry);
    }
    by_name
}
