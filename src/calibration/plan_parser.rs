use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};

use super::catalog::{PhaseInputs, PlanInputs};

#[derive(Clone, Debug)]
struct PhaseRow {
    ordinal: u32,
    slug: String,
    file: PathBuf,
    depends_on: Vec<u32>,
    touched_files: Vec<String>,
}

pub fn parse(plan_dir: &Path) -> anyhow::Result<PlanInputs> {
    let readme_path = plan_dir.join("README.md");
    let readme = fs::read_to_string(&readme_path)
        .with_context(|| format!("failed to read plan README {}", readme_path.display()))?;
    let rows = parse_phase_rows(&readme).with_context(|| {
        format!(
            "unrecognized README format in {}; expected phase table with Phase/File/Depends on/Touches columns",
            readme_path.display()
        )
    })?;

    if rows.is_empty() {
        bail!(
            "unrecognized README format in {}; expected at least one phase row",
            readme_path.display()
        );
    }

    let name = readme
        .lines()
        .find_map(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("multi-phase plan")
        .to_string();

    let mut phases = Vec::new();
    for row in rows {
        let phase_path = plan_dir.join(&row.file);
        let raw = fs::read_to_string(&phase_path)
            .with_context(|| format!("failed to read phase file {}", phase_path.display()))?;
        let routing_tier = extract_routing_tier(&raw).unwrap_or_else(|| "medium".to_string());
        let working_tree = extract_section(&raw, "Working tree")
            .and_then(|section| first_meaningful_line(section).map(str::to_string))
            .filter(|value| !is_primary_worktree(value));
        let mut files = extract_files_likely_touched(&raw);
        if files.is_empty() {
            files = row.touched_files;
        }
        files.sort();
        files.dedup();
        phases.push(PhaseInputs {
            ordinal: row.ordinal,
            slug: row.slug,
            routing_tier,
            files,
            working_tree,
            depends_on: row.depends_on,
        });
    }

    phases.sort_by_key(|phase| phase.ordinal);
    let routing_dist = routing_dist(&phases);
    let waves = waves(&phases)?;
    let repo_spread = repo_spread(&phases);
    let max_chain_depth = max_chain_depth(&phases)?;
    let wave_count = waves.len() as u32;
    let phase_count = phases.len() as u32;

    Ok(PlanInputs {
        path: plan_dir.to_path_buf(),
        name,
        flavor: flavor(&readme),
        worktype: None,
        phase_count,
        wave_count,
        max_chain_depth,
        repo_spread,
        routing_dist,
        phases,
        waves,
    })
}

fn parse_phase_rows(readme: &str) -> anyhow::Result<Vec<PhaseRow>> {
    let lines = readme.lines().collect::<Vec<_>>();
    for window_start in 0..lines.len().saturating_sub(1) {
        let header = lines[window_start].trim();
        let separator = lines[window_start + 1].trim();
        if !is_table_row(header) || !is_separator_row(separator) {
            continue;
        }
        let headers = split_table_row(header)
            .into_iter()
            .map(normalize_header)
            .collect::<Vec<_>>();
        let Some(phase_idx) = find_col(&headers, &["phase"]) else {
            continue;
        };
        let Some(file_idx) = find_col(&headers, &["file"]) else {
            continue;
        };
        let Some(depends_idx) = find_col(&headers, &["dependson", "depends"]) else {
            continue;
        };
        let touches_idx = find_col(&headers, &["touches", "files", "filelikelytouched"]);

        let mut rows = Vec::new();
        for line in lines.iter().skip(window_start + 2) {
            if !is_table_row(line.trim()) {
                break;
            }
            let cells = split_table_row(line.trim());
            if cells.len() <= phase_idx.max(file_idx).max(depends_idx) {
                continue;
            }
            let file = clean_cell(&cells[file_idx]);
            if file.is_empty() {
                continue;
            }
            let ordinal = parse_first_u32(&cells[phase_idx])
                .or_else(|| parse_first_u32(&file))
                .with_context(|| format!("failed to determine phase ordinal for row {line}"))?;
            let slug = Path::new(&file)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(strip_ordinal_prefix)
                .unwrap_or_else(|| format!("phase-{ordinal}"));
            let touched_files = touches_idx
                .and_then(|index| cells.get(index))
                .map(|cell| parse_file_list(cell))
                .unwrap_or_default();
            rows.push(PhaseRow {
                ordinal,
                slug,
                file: PathBuf::from(file),
                depends_on: parse_phase_refs(&cells[depends_idx]),
                touched_files,
            });
        }
        return Ok(rows);
    }

    bail!("missing phase table")
}

fn is_table_row(line: &str) -> bool {
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 3
}

fn is_separator_row(line: &str) -> bool {
    is_table_row(line)
        && split_table_row(line).iter().all(|cell| {
            let trimmed = cell.trim();
            !trimmed.is_empty()
                && trimmed
                    .chars()
                    .all(|ch| matches!(ch, '-' | ':' | ' ' | '\t'))
        })
}

fn split_table_row(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn normalize_header(header: String) -> String {
    header
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn find_col(headers: &[String], names: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|header| names.iter().any(|name| header == name))
}

fn clean_cell(cell: &str) -> String {
    let mut value = cell.trim().trim_matches('`').trim().to_string();
    if let Some((_, rest)) = value.split_once("](") {
        if let Some((link, _)) = rest.split_once(')') {
            value = link.to_string();
        }
    }
    value.trim_start_matches("./").to_string()
}

fn parse_first_u32(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn parse_phase_refs(cell: &str) -> Vec<u32> {
    if cell.trim().is_empty() || matches!(cell.trim(), "-" | "—" | "n/a" | "None" | "none") {
        return Vec::new();
    }
    let mut refs = Vec::new();
    let mut current = String::new();
    for ch in cell.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(value) = current.parse() {
                refs.push(value);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(value) = current.parse() {
            refs.push(value);
        }
    }
    refs.sort_unstable();
    refs.dedup();
    refs
}

fn parse_file_list(cell: &str) -> Vec<String> {
    cell.split("<br>")
        .flat_map(|part| part.split(','))
        .map(clean_cell)
        .filter(|value| !value.is_empty() && !matches!(value.as_str(), "-" | "—"))
        .collect()
}

fn strip_ordinal_prefix(stem: &str) -> String {
    stem.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '-' || ch == '_')
        .to_string()
}

fn extract_routing_tier(raw: &str) -> Option<String> {
    raw.lines()
        .find(|line| line.contains("Recommended") && line.contains("Codex model"))
        .and_then(|line| {
            ["max", "high", "medium", "low"]
                .into_iter()
                .find(|tier| line.to_ascii_lowercase().contains(tier))
        })
        .map(str::to_string)
}

fn extract_section<'a>(raw: &'a str, heading: &str) -> Option<&'a str> {
    let mut in_section = false;
    let mut start = 0_usize;
    let mut offset = 0_usize;
    for line in raw.lines() {
        if line.starts_with("## ") {
            if in_section {
                return Some(raw[start..offset].trim());
            }
            let title = line.trim_start_matches('#').trim();
            if title.eq_ignore_ascii_case(heading) {
                in_section = true;
                start = offset + line.len();
            }
        }
        offset += line.len() + 1;
    }
    in_section.then(|| raw[start..].trim())
}

fn first_meaningful_line(section: &str) -> Option<&str> {
    section
        .lines()
        .map(|line| line.trim().trim_start_matches("- ").trim_matches('`'))
        .find(|line| !line.is_empty())
}

fn is_primary_worktree(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "primary" | "same as primary" | "same as working tree" | "none" | "n/a" | "-"
    )
}

fn extract_files_likely_touched(raw: &str) -> Vec<String> {
    let Some(section) = extract_section(raw, "Files likely touched") else {
        return Vec::new();
    };
    section
        .lines()
        .filter_map(|line| line.trim().strip_prefix('-'))
        .map(clean_cell)
        .filter(|value| !value.is_empty())
        .collect()
}

fn flavor(readme: &str) -> String {
    let lower = readme.to_ascii_lowercase();
    if lower.contains("claude") {
        "claude".to_string()
    } else if lower.contains("mixed") {
        "mixed".to_string()
    } else {
        "codex".to_string()
    }
}

fn routing_dist(phases: &[PhaseInputs]) -> BTreeMap<String, u32> {
    let mut dist = BTreeMap::new();
    for phase in phases {
        *dist.entry(phase.routing_tier.clone()).or_default() += 1;
    }
    dist
}

fn repo_spread(phases: &[PhaseInputs]) -> u32 {
    let mut repos = BTreeSet::from(["primary".to_string()]);
    for phase in phases {
        if let Some(tree) = &phase.working_tree {
            repos.insert(tree.clone());
        }
    }
    repos.len() as u32
}

fn max_chain_depth(phases: &[PhaseInputs]) -> anyhow::Result<u32> {
    let by_ordinal = phases
        .iter()
        .map(|phase| (phase.ordinal, phase))
        .collect::<BTreeMap<_, _>>();
    let mut memo = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut max_depth = 0;
    for phase in phases {
        max_depth = max_depth.max(depth(phase.ordinal, &by_ordinal, &mut memo, &mut visiting)?);
    }
    Ok(max_depth)
}

fn depth(
    ordinal: u32,
    phases: &BTreeMap<u32, &PhaseInputs>,
    memo: &mut BTreeMap<u32, u32>,
    visiting: &mut BTreeSet<u32>,
) -> anyhow::Result<u32> {
    if let Some(depth) = memo.get(&ordinal) {
        return Ok(*depth);
    }
    if !visiting.insert(ordinal) {
        bail!("dependency cycle includes phase {ordinal}");
    }
    let phase = phases
        .get(&ordinal)
        .with_context(|| format!("phase {ordinal} referenced as dependency but is missing"))?;
    let mut max_dep = 0;
    for dep in &phase.depends_on {
        max_dep = max_dep.max(depth(*dep, phases, memo, visiting)?);
    }
    visiting.remove(&ordinal);
    let value = max_dep + 1;
    memo.insert(ordinal, value);
    Ok(value)
}

fn waves(phases: &[PhaseInputs]) -> anyhow::Result<Vec<Vec<u32>>> {
    let ordinals = phases
        .iter()
        .map(|phase| phase.ordinal)
        .collect::<BTreeSet<_>>();
    let mut indegree = BTreeMap::new();
    let mut downstream: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for phase in phases {
        indegree.entry(phase.ordinal).or_insert(0_u32);
        for dep in &phase.depends_on {
            if !ordinals.contains(dep) {
                bail!("phase {} depends on missing phase {dep}", phase.ordinal);
            }
            *indegree.entry(phase.ordinal).or_insert(0) += 1;
            downstream.entry(*dep).or_default().push(phase.ordinal);
        }
    }
    let mut queue = indegree
        .iter()
        .filter_map(|(ordinal, count)| (*count == 0).then_some(*ordinal))
        .collect::<VecDeque<_>>();
    let mut out = Vec::new();
    let mut seen = 0;
    while !queue.is_empty() {
        let mut wave = queue.drain(..).collect::<Vec<_>>();
        wave.sort_unstable();
        seen += wave.len();
        for ordinal in &wave {
            for child in downstream.get(ordinal).cloned().unwrap_or_default() {
                let count = indegree.get_mut(&child).expect("child exists");
                *count -= 1;
                if *count == 0 {
                    queue.push_back(child);
                }
            }
        }
        out.push(wave);
    }
    if seen != phases.len() {
        bail!("dependency cycle detected in phase table");
    }
    Ok(out)
}
