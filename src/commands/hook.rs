use std::{
    env, fs,
    io::{self, IsTerminal, Read},
};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    calibration::{db::DbParam as P, Db},
    cli::args::{HookArgs, HookCommand},
    commands::hook_install,
    config::DbTarget,
};

pub fn run(args: HookArgs, target: Option<DbTarget>) -> Result<()> {
    match args.command {
        HookCommand::Ingest {
            event,
            payload_file,
            strict,
        } => {
            let target = target
                .ok_or_else(|| anyhow::anyhow!("internal error: missing hook ingest database"))?;
            let result = ingest(target, &event, payload_file);
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

fn ingest(target: DbTarget, event: &str, payload_file: Option<Utf8PathBuf>) -> Result<()> {
    let payload = read_payload(payload_file.as_ref())?;
    let parsed: Value =
        serde_json::from_str(&payload).context("failed to parse hook payload JSON")?;
    let extracted = ExtractedHookPayload::from_json(&parsed, event);
    let session_id = env::var("CLAUDE_SESSION_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            eprintln!(
                "warning: CLAUDE_SESSION_ID is unset; using nil UUID for hook invocation session"
            );
            Uuid::nil().to_string()
        });
    let project_dir = env::var("CLAUDE_PROJECT_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty());

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
            hook_event
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
            $7
        )",
        &[
            P::from(session_id.as_str()),
            P::from(extracted.skill_name.as_str()),
            P::nullable_text(extracted.tool_name.as_deref()),
            P::nullable_text(project_dir.as_deref()),
            P::from(extracted.outcome.as_str()),
            P::from(payload.as_str()),
            P::from(event),
        ],
    )?;
    Ok(())
}

fn read_payload(payload_file: Option<&Utf8PathBuf>) -> Result<String> {
    match payload_file {
        Some(path) => fs::read_to_string(path.as_std_path())
            .with_context(|| format!("failed to read hook payload file {path}")),
        None => {
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
    current.as_str().filter(|value| !value.trim().is_empty())
}

fn open_db(target: DbTarget) -> Result<Db> {
    match target {
        DbTarget::Sqlite(path) => Db::open(&path),
        DbTarget::Postgres(_url) => {
            #[cfg(feature = "postgres")]
            {
                Db::open_postgres(&_url)
            }
            #[cfg(not(feature = "postgres"))]
            {
                anyhow::bail!(
                    "Postgres hook database requested, but this skillnet binary was built without \
                     the `postgres` feature; rebuild with `--features postgres` or select sqlite"
                )
            }
        }
    }
}
