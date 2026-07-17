use std::collections::{BTreeMap, BTreeSet};
use std::fs;

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
        let implicit_disabled = implicit_invocation_disabled(entry);
        if entry.status == "routed" {
            if !implicit_disabled {
                errors.push(format!(
                    "{}: routed skill must set policy.allow_implicit_invocation = false",
                    entry.qualified_name
                ));
            }
            if entry.related_skills.is_empty() {
                errors.push(format!(
                    "{}: routed skill must name its router in related_skills",
                    entry.qualified_name
                ));
            }
        }
        if implicit_disabled && matches!(entry.status.as_str(), "active" | "experimental") {
            errors.push(format!(
                "{}: implicit-only skill has active status; classify it as routed, reference, internal, or retired",
                entry.qualified_name
            ));
        }
        if let Some(limit) = config.settings.metadata_description_char_limit {
            let budget_scope = entry.scope == "global" || entry.project.as_deref() == Some("canix");
            if budget_scope && !implicit_disabled && entry.description.chars().count() > limit {
                errors.push(format!(
                    "{}: description is {} characters (limit {limit})",
                    entry.qualified_name,
                    entry.description.chars().count()
                ));
            }
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
    errors.extend(validate_metadata_budget(entries, config));
    errors
}

fn implicit_invocation_disabled(entry: &SkillEntry) -> bool {
    let path = entry.path.join("agents/openai.yaml");
    fs::read_to_string(path).is_ok_and(|body| {
        body.lines()
            .any(|line| line.trim().replace(' ', "") == "allow_implicit_invocation:false")
    })
}

fn validate_metadata_budget(entries: &[SkillEntry], config: &CatalogConfig) -> Vec<String> {
    let settings = &config.settings;
    let (Some(context), Some(percent), Some(headroom), Some(reserved)) = (
        settings.metadata_context_window_tokens,
        settings.metadata_budget_percent,
        settings.metadata_headroom_percent,
        settings.metadata_reserved_tokens,
    ) else {
        return Vec::new();
    };
    if percent > 100 || headroom > 100 {
        return vec!["metadata budget percentages must be between 0 and 100".into()];
    }
    let budget = context.saturating_mul(percent) / 100;
    let headroom_ceiling = budget.saturating_mul(100usize.saturating_sub(headroom)) / 100;
    let ceiling = headroom_ceiling.min(budget.saturating_sub(reserved));
    let estimated = entries
        .iter()
        .filter(|entry| {
            (entry.scope == "global" || entry.project.as_deref() == Some("canix"))
                && matches!(entry.status.as_str(), "active" | "experimental")
                && !implicit_invocation_disabled(entry)
        })
        .map(estimated_metadata_tokens)
        .sum::<usize>();
    if estimated > ceiling {
        vec![format!(
            "implicit skill metadata estimate is {estimated} tokens, above effective ceiling {ceiling} ({}% of {context}, {}% headroom, {reserved} reserved)",
            percent, headroom
        )]
    } else {
        Vec::new()
    }
}

fn estimated_metadata_tokens(entry: &SkillEntry) -> usize {
    // Codex renders one compact line containing the qualified skill name and
    // description. Four UTF-8 bytes per token is intentionally conservative.
    (entry.qualified_name.len() + entry.description.len() + 8).div_ceil(4)
}

pub(super) fn duplicates_by_name(entries: &[SkillEntry]) -> BTreeMap<&str, Vec<&SkillEntry>> {
    let mut by_name = BTreeMap::<&str, Vec<&SkillEntry>>::new();
    for entry in entries {
        by_name.entry(entry.name.as_str()).or_default().push(entry);
    }
    by_name
}
