use std::{
    env, fs,
    io::{self, IsTerminal, Read},
};

use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    calibration::{db::DbParam as P, Db},
    cards::repo_root,
    cli::args::{HookArgs, HookCommand},
    commands::hook_install,
    config::{expand_path, DbTarget, ProjectConfig},
};

pub(crate) fn run(
    args: HookArgs,
    target: Option<DbTarget>,
    global_skills: Option<&Utf8Path>,
    projects: &[ProjectConfig],
) -> Result<()> {
    match args.command {
        HookCommand::Ingest {
            event,
            harness,
            payload_file,
            strict,
        } => {
            let target = target
                .ok_or_else(|| anyhow::anyhow!("internal error: missing hook ingest database"))?;
            let result = ingest(
                target,
                &event,
                harness.trim(),
                payload_file.as_deref(),
                global_skills,
                projects,
            );
            if strict {
                result
            } else {
                if let Err(err) = result {
                    eprintln!("skillnet hook ingest failed: {err:#}");
                }
                Ok(())
            }
        }
        HookCommand::Install {
            settings,
            events,
            matchers,
            dry_run,
        } => hook_install::install(&settings, &events, &matchers, dry_run),
        HookCommand::Uninstall { settings } => hook_install::uninstall(&settings),
        HookCommand::Status { settings } => {
            if hook_install::is_installed(&settings)? {
                println!("installed");
                Ok(())
            } else {
                println!("not installed");
                std::process::exit(1);
            }
        }
    }
}

fn ingest(
    target: DbTarget,
    event: &str,
    harness: &str,
    payload_file: Option<&Utf8Path>,
    global_skills: Option<&Utf8Path>,
    projects: &[ProjectConfig],
) -> Result<()> {
    if harness.is_empty() {
        bail!("harness must not be empty");
    }
    let payload = read_payload(payload_file)?;
    let parsed: Value =
        serde_json::from_str(&payload).context("failed to parse hook payload JSON")?;
    let extracted = ExtractedHookPayload::from_json(&parsed, event);
    let session_env = env::var("CLAUDE_SESSION_ID").ok();
    let session_id = ["session_id", "sessionId"]
        .iter()
        .find_map(|key| string_at(&parsed, &[*key]))
        .or_else(|| session_env.as_deref().and_then(non_empty))
        .map_or_else(
            || {
                eprintln!(
                    "warning: no session ID in hook payload or CLAUDE_SESSION_ID; using nil UUID"
                );
                Uuid::nil().to_string()
            },
            ToOwned::to_owned,
        );
    let project_env = env::var("CLAUDE_PROJECT_DIR").ok();
    let project_dir = ["project_dir", "projectDir"]
        .iter()
        .find_map(|key| string_at(&parsed, &[*key]))
        .or_else(|| project_env.as_deref().and_then(non_empty))
        .or_else(|| string_at(&parsed, &["cwd"]));
    let root = project_dir.and_then(|path| repo_root(Utf8Path::new(path)).ok());
    let canonical = canonical_skill_name(
        &extracted.skill_name,
        root.as_deref(),
        global_skills,
        projects,
    );

    let source_event_id = format!(
        "{harness}:{session_id}:{event}:{}",
        blake3::hash(payload.as_bytes()).to_hex()
    );
    let db = open_db(target)?;
    db.execute(
        "INSERT INTO skill_invocations (
            session_id,
            skill_name,
            tool_name,
            project_dir,
            started_at,
            ended_at,
            outcome,
            plan_id,
            payload,
            hook_event,
            harness,
            source_event_id,
            adapter_version,
            canonical_skill_name
        ) VALUES (
            $1,
            $2,
            $3,
            $4,
            CURRENT_TIMESTAMP,
            CURRENT_TIMESTAMP,
            $5,
            NULL,
            $6,
            $7,
            $8,
            $9,
            '1',
            $10
        )
        ON CONFLICT DO NOTHING",
        &[
            P::from(session_id.as_str()),
            P::from(extracted.skill_name.as_str()),
            P::nullable_text(extracted.tool_name.as_deref()),
            P::nullable_text(root.as_deref().map(Utf8Path::as_str).or(project_dir)),
            P::from(extracted.outcome.as_str()),
            P::from("{}"),
            P::from(event),
            P::from(harness),
            P::from(source_event_id.as_str()),
            P::from(canonical.as_str()),
        ],
    )?;
    Ok(())
}

fn read_payload(payload_file: Option<&Utf8Path>) -> Result<String> {
    if let Some(path) = payload_file {
        fs::read_to_string(path.as_std_path())
            .with_context(|| format!("failed to read hook payload file {path}"))
    } else {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            bail!("refusing to read hook payload from an interactive terminal; pipe JSON on stdin or use --payload-file");
        }
        let mut payload = String::new();
        stdin
            .lock()
            .read_to_string(&mut payload)
            .context("failed to read hook payload from stdin")?;
        Ok(payload)
    }
}

struct ExtractedHookPayload {
    skill_name: String,
    tool_name: Option<String>,
    outcome: String,
}

impl ExtractedHookPayload {
    fn from_json(payload: &Value, event: &str) -> Self {
        let tool_name = string_at(payload, &["tool_name"])
            .or_else(|| string_at(payload, &["tool", "name"]))
            .map(ToOwned::to_owned);
        let skill_name = if tool_name.as_deref() == Some("Skill") {
            string_at(payload, &["tool_input", "skill"])
                .or(tool_name.as_deref())
                .unwrap_or(event)
                .to_string()
        } else {
            tool_name.as_deref().unwrap_or(event).to_string()
        };
        let is_error = payload
            .get("tool_response")
            .and_then(|response| response.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Self {
            skill_name,
            tool_name,
            outcome: if is_error { "error" } else { "ok" }.to_string(),
        }
    }
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().and_then(non_empty)
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn canonical_skill_name(
    raw: &str,
    project: Option<&Utf8Path>,
    global: Option<&Utf8Path>,
    projects: &[ProjectConfig],
) -> String {
    if raw.contains(['/', '\\']) || matches!(raw, "." | "..") {
        return raw.to_owned();
    }
    let contains = |root: &Utf8Path| root.join(raw).join("SKILL.md").is_file();
    if let Some(path) = project {
        if [".skills", ".agents/skills", ".claude/skills"]
            .iter()
            .any(|dir| contains(&path.join(dir)))
        {
            let configured = projects.iter().find(|project| {
                expand_path(&project.path)
                    .and_then(|root| Ok(root.canonicalize_utf8()?))
                    .is_ok_and(|root| root == path)
            });
            return configured.map_or_else(|| raw.to_owned(), |p| format!("{}/{raw}", p.name));
        }
    }
    if global.is_some_and(contains) {
        return format!("global/{raw}");
    }
    raw.to_owned()
}

fn open_db(target: DbTarget) -> Result<Db> {
    match target {
        DbTarget::Sqlite(path) => Db::open(&path),
        #[cfg(feature = "postgres")]
        DbTarget::Postgres(url) => Db::open_postgres(&url),
        #[cfg(not(feature = "postgres"))]
        DbTarget::Postgres(_) => anyhow::bail!(
            "Postgres hook database requested, but this skillnet binary was built without \
             the `postgres` feature; rebuild with `--features postgres` or select sqlite"
        ),
    }
}
