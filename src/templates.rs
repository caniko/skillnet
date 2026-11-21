//! Model bindings and rendering for `SKILL.md.tmpl` entrypoints.

use std::{collections::BTreeMap, fmt::Write as _, fs, io::ErrorKind};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::config::{expand_path, Config};

pub(crate) const TEMPLATE_FILE: &str = "SKILL.md.tmpl";
pub(crate) const META_FILE: &str = "skillnet-template.toml";

/// Expanded paths for editing the global template bindings.
pub struct Settings {
    pub canonical: Utf8PathBuf,
    pub fleet: Option<Utf8PathBuf>,
    pub local: Option<Utf8PathBuf>,
    pub t3_caches: Utf8PathBuf,
}

impl Settings {
    pub fn load(config_path: &Utf8Path) -> Result<Self> {
        let config = Config::load(config_path)?;
        let expanded = |path: &Option<String>| path.as_deref().map(expand_path).transpose();
        let (fleet, local) = (
            expanded(&config.templates.fleet)?,
            expanded(&config.templates.local)?,
        );
        if fleet.is_some() && fleet == local {
            bail!("[templates] fleet and local must be different files");
        }
        let mirror_root = crate::cli::resolve_mirror_root(&config, None)?;
        let canonical = config.global_target(&mirror_root)?.canonical_path;
        Ok(Self {
            canonical: if canonical.is_absolute() {
                canonical
            } else {
                expand_path(canonical.as_str())?
            },
            fleet,
            local,
            t3_caches: expand_path(
                config
                    .models
                    .t3_caches
                    .as_deref()
                    .unwrap_or("~/.local/share/t3code-native/caches"),
            )?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BindingsDoc {
    #[serde(default)]
    pub skills: BTreeMap<String, SkillBinding>,
    pub ui: Option<UiConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SkillBinding {
    pub seats: Vec<Seat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Seat {
    pub name: String,
    pub label: Option<String>,
    pub route: Route,
    #[serde(default)]
    pub fallbacks: Vec<Fallback>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Route {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Fallback {
    #[serde(flatten)]
    pub route: Route,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UiConfig {
    pub providers: Option<Vec<String>>,
    pub hosts_command: Option<Vec<String>>,
}

/// Allowed seat count, defaulting to one or more seats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct TemplateMeta {
    pub min: usize,
    pub max: Option<usize>,
}

impl Default for TemplateMeta {
    fn default() -> Self {
        Self { min: 1, max: None }
    }
}

/// Evaluate each file locally and merge in order, replacing whole skill entries.
pub fn load_bindings(paths: &[Utf8PathBuf]) -> Result<BindingsDoc> {
    let mut merged = BindingsDoc::default();
    for path in paths {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read Skillnet bindings {path}"))?;
        let doc: BindingsDoc = serde_json::from_value(crate::manifest::eval_local(path, &source)?)
            .with_context(|| format!("invalid evaluated Skillnet bindings {path}"))?;
        merged.skills.extend(doc.skills);
        if doc.ui.is_some() {
            merged.ui = doc.ui;
        }
    }
    Ok(merged)
}

/// Like `load_bindings`, skipping configured files that do not exist.
pub fn load_existing(paths: &[Utf8PathBuf]) -> Result<BindingsDoc> {
    let mut existing = Vec::new();
    for path in paths {
        // Only absence is skipped; an unreadable override must not silently
        // fall back to the fleet binding.
        if path
            .try_exists()
            .with_context(|| format!("cannot access Skillnet bindings {path}"))?
        {
            existing.push(path.clone());
        }
    }
    load_bindings(&existing)
}

/// Write a self-contained, deterministic Pkl bindings document.
pub fn to_pkl(doc: &BindingsDoc) -> String {
    let mut out = String::from("// Managed by skillnet; edits are overwritten.\nskills {\n");
    for (name, binding) in &doc.skills {
        if binding.seats.is_empty() {
            let _ = writeln!(out, "  [{}] {{ seats = List() }}", pkl_string(name));
            continue;
        }
        let _ = writeln!(out, "  [{}] {{\n    seats {{", pkl_string(name));
        for seat in &binding.seats {
            let _ = writeln!(
                out,
                "      new {{\n        name = {}",
                pkl_string(&seat.name)
            );
            if let Some(label) = &seat.label {
                let _ = writeln!(out, "        label = {}", pkl_string(label));
            }
            out.push_str("        route {\n");
            seat.route.write_pkl(&mut out, "          ");
            out.push_str("        }\n");
            if !seat.fallbacks.is_empty() {
                out.push_str("        fallbacks {\n");
                for fallback in &seat.fallbacks {
                    out.push_str("          new {\n");
                    fallback.route.write_pkl(&mut out, "            ");
                    let _ = writeln!(out, "            enabled = {}", fallback.enabled);
                    out.push_str("          }\n");
                }
                out.push_str("        }\n");
            }
            out.push_str("      }\n");
        }
        out.push_str("    }\n  }\n");
    }
    out.push_str("}\n");
    if let Some(ui) = &doc.ui {
        let list = |values: &[String]| {
            let values = values
                .iter()
                .map(|value| pkl_string(value))
                .collect::<Vec<_>>();
            format!("List({})", values.join(", "))
        };
        let providers = ui.providers.as_deref().map_or_else(|| "null".into(), list);
        let _ = write!(out, "ui {{ providers = {providers}");
        if let Some(command) = &ui.hosts_command {
            let _ = write!(out, "; hosts_command = {}", list(command));
        }
        out.push_str(" }\n");
    }
    out
}

fn pkl_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(quoted, "\\u{{{:x}}}", u32::from(c));
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

impl Route {
    fn write_pkl(&self, out: &mut String, indent: &str) {
        let _ = writeln!(out, "{indent}provider = {}", pkl_string(&self.provider));
        let _ = writeln!(out, "{indent}model = {}", pkl_string(&self.model));
        if !self.options.is_empty() {
            let _ = writeln!(out, "{indent}options {{");
            for (key, value) in &self.options {
                let _ = writeln!(
                    out,
                    "{indent}  [{}] = {}",
                    pkl_string(key),
                    pkl_string(value)
                );
            }
            let _ = writeln!(out, "{indent}}}");
        }
    }

    fn dispatch_target(&self) -> String {
        let quote = |value: &str| serde_json::to_string(value).expect("strings serialize to JSON");
        let options = self
            .options
            .iter()
            .map(|(key, value)| {
                let identifier = key
                    .bytes()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || b"_$".contains(&c))
                    && key
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_$".contains(&c));
                let key = if identifier { key.clone() } else { quote(key) };
                format!("{key}: {}", quote(value))
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "`{{providerInstanceId: {}, model: {}, options: {{{options}}}}}`",
            quote(&self.provider),
            quote(&self.model),
        )
    }
}

impl Seat {
    fn dispatch(&self) -> String {
        let mut text = format!("`delegate_task` target {}.", self.route.dispatch_target());
        let fallbacks = self
            .fallbacks
            .iter()
            .filter(|fallback| fallback.enabled)
            .map(|fallback| fallback.route.dispatch_target())
            .collect::<Vec<_>>();
        if !fallbacks.is_empty() {
            let _ = write!(
                text,
                " On a usage-limit or quota error, send the same request to the next route in order: {}. Never switch to a different model.",
                fallbacks.join(", ")
            );
        }
        text
    }
}

/// Render scalar tokens and sequential, non-nested seat repeat blocks.
pub fn render(
    template: &str,
    meta: TemplateMeta,
    skill: &str,
    binding: &SkillBinding,
) -> Result<String> {
    const OPEN: &str = "{{#seats}}";
    const CLOSE: &str = "{{/seats}}";
    let count = binding.seats.len();
    if count < meta.min || meta.max.is_some_and(|max| count > max) {
        let max = meta.max.map_or("unbounded".into(), |max| max.to_string());
        bail!(
            "template `{skill}`: seat count {count} is outside [{}, {max}]",
            meta.min
        );
    }
    let labels = binding
        .seats
        .iter()
        .map(|seat| {
            seat.label.clone().unwrap_or_else(|| {
                std::iter::once(&seat.route.model)
                    .chain(seat.route.options.values())
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        })
        .collect::<Vec<_>>();
    let seat_labels = labels.join(", ");
    let globals = |key: &str| match key {
        "name" => Some(skill.to_owned()),
        "seat_count" => Some(count.to_string()),
        "seat_labels" => Some(seat_labels.clone()),
        _ => None,
    };
    let template: String = template
        .split_inclusive('\n')
        .map(|line| match line.trim() {
            OPEN => OPEN,
            CLOSE => CLOSE,
            _ => line,
        })
        .collect();
    let mut out = String::with_capacity(template.len());
    let mut remaining = template.as_str();
    while let Some((prefix, rest)) = remaining.split_once(OPEN) {
        expand(prefix, &mut out, skill, globals)?;
        let (body, suffix) = rest
            .split_once(CLOSE)
            .with_context(|| format!("template `{skill}`: unclosed block {OPEN}"))?;
        remaining = suffix;
        let mut unused = String::new();
        for index in 0..count.max(1) {
            let seat = binding.seats.get(index);
            expand(
                body,
                if seat.is_some() {
                    &mut out
                } else {
                    &mut unused
                },
                skill,
                |key| match key {
                    "seat.name" => Some(seat.map_or("", |seat| &seat.name).to_owned()),
                    "seat.label" => Some(labels.get(index).cloned().unwrap_or_default()),
                    "seat.dispatch" => Some(seat.map(Seat::dispatch).unwrap_or_default()),
                    _ => globals(key),
                },
            )?;
        }
    }
    expand(remaining, &mut out, skill, globals)?;
    if let Some((_, token)) = out.split_once("{{") {
        let token = token.find("}}").map_or(token, |end| &token[..end + 2]);
        bail!("template `{skill}`: unresolved token `{{{{{token}`");
    }
    Ok(out)
}

fn expand(
    mut text: &str,
    out: &mut String,
    skill: &str,
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> Result<()> {
    while let Some((prefix, rest)) = text.split_once("{{") {
        out.push_str(prefix);
        let (key, suffix) = rest
            .split_once("}}")
            .with_context(|| format!("template `{skill}`: unclosed token `{{{{{rest}`"))?;
        let value = resolve(key)
            .with_context(|| format!("template `{skill}`: unexpected token `{{{{{key}}}}}`"))?;
        out.push_str(&value);
        text = suffix;
    }
    out.push_str(text);
    Ok(())
}

pub(crate) fn is_template(path: &Utf8Path) -> bool {
    path.join(TEMPLATE_FILE).is_file()
        && fs::symlink_metadata(path.join("SKILL.md"))
            .is_err_and(|error| error.kind() == ErrorKind::NotFound)
}

pub(crate) fn canonical_skill_dirs(root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = Utf8PathBuf::from_path_buf(entry?.path())
            .map_err(|p| anyhow::anyhow!("non-UTF-8 path in mirror: {}", p.display()))?;
        if path.is_dir() && (path.join("SKILL.md").is_file() || is_template(&path)) {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

pub fn list_templates(canonical: &Utf8Path) -> Result<Vec<(String, TemplateMeta)>> {
    canonical_skill_dirs(canonical)?
        .into_iter()
        .filter(|path| is_template(path))
        .map(|path| {
            let skill = path.file_name().context("template directory has no name")?;
            Ok((skill.to_owned(), read_meta(&path, skill)?))
        })
        .collect()
}

pub fn render_skill(canonical: &Utf8Path, skill: &str, binding: &SkillBinding) -> Result<String> {
    crate::bundle::validate_skill_name(skill, "template skill")?;
    let path = canonical.join(skill);
    if !is_template(&path) {
        bail!("`{skill}` is not a canonical template at {path}");
    }
    render_template(&path, skill, binding)
}

pub(crate) fn render_dir(
    path: &Utf8Path,
    skill: &str,
    bindings: &BindingsDoc,
    binding_files: &[Utf8PathBuf],
) -> Result<String> {
    let binding = bindings.skills.get(skill).with_context(|| {
        if binding_files.is_empty() {
            format!("template `{skill}` has no binding: set [templates] fleet or local in skillnet.toml")
        } else {
            format!("template `{skill}` has no binding in {binding_files:?} (absent files are skipped)")
        }
    })?;
    render_template(path, skill, binding)
}

fn render_template(path: &Utf8Path, skill: &str, binding: &SkillBinding) -> Result<String> {
    let meta = read_meta(path, skill)?;
    let template_path = path.join(TEMPLATE_FILE);
    let template = fs::read_to_string(&template_path)
        .with_context(|| format!("failed to read template {template_path}"))?;
    render(&template, meta, skill, binding)
}

fn read_meta(path: &Utf8Path, skill: &str) -> Result<TemplateMeta> {
    let meta_path = path.join(META_FILE);
    match fs::read_to_string(&meta_path) {
        Ok(source) => {
            #[derive(Default, Deserialize)]
            #[serde(default)]
            struct Meta {
                seats: TemplateMeta,
            }
            toml::from_str::<Meta>(&source)
                .map(|meta| meta.seats)
                .with_context(|| format!("invalid metadata for template `{skill}` at {meta_path}"))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(TemplateMeta::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {meta_path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reviewer() -> Seat {
        Seat {
            name: "reviewer".into(),
            label: None,
            route: Route {
                provider: "opencode".into(),
                model: "openai/gpt-6.2-sol".into(),
                options: BTreeMap::from([("variant".into(), "high".into())]),
            },
            fallbacks: Vec::new(),
        }
    }

    #[test]
    fn renders_one_or_two_seats_and_only_enabled_fallbacks() {
        let mut binding = SkillBinding {
            seats: vec![reviewer()],
        };
        let template = "{{name}} [{{seat_count}}]: {{seat_labels}}\n  {{#seats}} \n{{seat.name}}: {{seat.label}}. {{seat.dispatch}}\n  {{/seats}}\nDone.";
        let target = r#"`delegate_task` target `{providerInstanceId: "opencode", model: "openai/gpt-6.2-sol", options: {variant: "high"}}`."#;
        let first = format!("reviewer: openai/gpt-6.2-sol high. {target}\n");
        assert_eq!(
            render(template, TemplateMeta::default(), "review", &binding).unwrap(),
            format!("review [1]: openai/gpt-6.2-sol high\n{first}Done.")
        );
        let fallback = |provider: &str, enabled| Fallback {
            route: Route {
                provider: provider.into(),
                model: "gpt-6.2-sol".into(),
                options: BTreeMap::from([("reasoningEffort".into(), "high".into())]),
            },
            enabled,
        };
        binding.seats.push(Seat {
            name: "second".into(),
            label: Some("Sol".into()),
            fallbacks: vec![
                fallback("codex", true),
                fallback("disabled", false),
                fallback("backup", true),
            ],
            ..reviewer()
        });
        let tail = r#" On a usage-limit or quota error, send the same request to the next route in order: `{providerInstanceId: "codex", model: "gpt-6.2-sol", options: {reasoningEffort: "high"}}`, `{providerInstanceId: "backup", model: "gpt-6.2-sol", options: {reasoningEffort: "high"}}`. Never switch to a different model."#;
        assert_eq!(
            render(template, TemplateMeta::default(), "review", &binding).unwrap(),
            format!("review [2]: openai/gpt-6.2-sol high, Sol\n{first}second: Sol. {target}{tail}\nDone.")
        );
        assert_eq!(
            render(
                "a{{#seats}}({{seat.name}}){{/seats}}-{{#seats}}[{{seat.name}}]{{/seats}}b",
                TemplateMeta::default(),
                "review",
                &binding
            )
            .unwrap(),
            "a(reviewer)(second)-[reviewer][second]b"
        );
    }

    #[test]
    fn rejects_invalid_tokens_and_seat_counts() {
        let binding = SkillBinding {
            seats: vec![Seat {
                label: Some("{{leftover}}".into()),
                ..reviewer()
            }],
        };
        for (template, token) in [
            ("{{bogus}}", "{{bogus}}"),
            ("{{seat.name}}", "{{seat.name}}"),
            ("{{#seats}}open", "{{#seats}}"),
            ("{{#seats}}{{#seats}}{{/seats}}{{/seats}}", "{{#seats}}"),
            ("{{/seats}}", "{{/seats}}"),
            ("{{", "{{"),
            ("{{seat_labels}}", "{{leftover}}"),
        ] {
            let error = render(template, TemplateMeta::default(), "review", &binding)
                .unwrap_err()
                .to_string();
            assert!(error.contains("review") && error.contains(token), "{error}");
        }
        for meta in [
            TemplateMeta { min: 2, max: None },
            TemplateMeta {
                min: 0,
                max: Some(0),
            },
        ] {
            let error = render("", meta, "review", &binding)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("review") && error.contains("seat count"),
                "{error}"
            );
        }
        let empty = SkillBinding { seats: vec![] };
        let meta = TemplateMeta { min: 0, max: None };
        assert_eq!(
            render("{{#seats}}{{seat.name}}{{/seats}}", meta, "review", &empty).unwrap(),
            ""
        );
        assert!(render("{{#seats}}{{unknown}}{{/seats}}", meta, "review", &empty).is_err());
    }

    #[test]
    fn lists_and_renders_canonical_templates() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        for skill in ["plain", "review"] {
            fs::create_dir(root.join(skill)).unwrap();
            fs::write(root.join(skill).join(TEMPLATE_FILE), "{{seat_count}} seats").unwrap();
        }
        fs::write(root.join("plain/SKILL.md"), "plain").unwrap();
        fs::write(
            root.join("review").join(META_FILE),
            "[seats]\nmin = 1\nmax = 1",
        )
        .unwrap();
        let meta = TemplateMeta {
            min: 1,
            max: Some(1),
        };
        assert_eq!(list_templates(root).unwrap(), vec![("review".into(), meta)]);
        let mut binding = SkillBinding {
            seats: vec![reviewer()],
        };
        assert_eq!(render_skill(root, "review", &binding).unwrap(), "1 seats");
        binding.seats.push(reviewer());
        assert!(render_skill(root, "review", &binding).is_err());
        assert!(render_skill(root, "plain", &binding).is_err());
    }

    #[test]
    fn bindings_round_trip_and_merge_through_local_pkl() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let base = root.join("base.pkl");
        let override_path = root.join("override.pkl");
        let names = root.join("names.pkl");
        let seat = Seat {
            label: Some("quoted \" text \\(literal)\n\t\u{1b}🧪".into()),
            fallbacks: vec![Fallback {
                enabled: false,
                route: reviewer().route,
            }],
            ..reviewer()
        };
        let doc = BindingsDoc {
            skills: BTreeMap::from([
                ("review".into(), SkillBinding { seats: vec![seat] }),
                ("other".into(), SkillBinding { seats: vec![] }),
            ]),
            ui: Some(UiConfig {
                providers: Some(vec!["opencode".into()]),
                hosts_command: Some(vec!["canix".into(), "hosts".into(), "--json".into()]),
            }),
        };
        let pkl = to_pkl(&doc);
        assert!(pkl.starts_with("// Managed by skillnet; edits are overwritten.\n"));
        fs::write(&base, pkl).unwrap();
        assert_eq!(load_bindings(std::slice::from_ref(&base)).unwrap(), doc);
        fs::write(names, "reviewer = \"replacement\"").unwrap();
        fs::write(
            &override_path,
            r#"
            amends "names.pkl"
            import "base.pkl" as base
            skills { ["review"] { seats { new {
                name = reviewer
                route = base.skills["review"].seats[0].route
                fallbacks { new { provider = "codex"; model = "gpt-6.2-sol" } }
            } } } }
            ui { providers = null }
        "#,
        )
        .unwrap();
        let merged = load_bindings(&[base, override_path]).unwrap();
        assert_eq!(
            merged.ui,
            Some(UiConfig {
                providers: None,
                hosts_command: None,
            })
        );
        assert_eq!(merged.skills["other"], doc.skills["other"]);
        let replacement = &merged.skills["review"].seats[0];
        assert_eq!(replacement.name, "replacement");
        assert_eq!(replacement.label, None);
        assert_eq!(replacement.route, reviewer().route);
        assert!(replacement.fallbacks[0].enabled);
    }
}
