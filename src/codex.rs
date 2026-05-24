use std::{io::ErrorKind, process::Command};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoCommitRequest {
    pub model: String,
    pub reasoning_effort: String,
    pub dirty_paths: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutoCommitOutcome {
    pub final_message: Option<String>,
    pub stdout: String,
    pub stderr: String,
}

pub(crate) fn auto_commit(
    repo_root: &Utf8Path,
    request: &AutoCommitRequest,
) -> Result<AutoCommitOutcome> {
    let output_last_message = NamedTempFile::new().context("failed to create Codex output file")?;
    let prompt = build_prompt(repo_root, &request.dirty_paths);
    let output = Command::new("codex")
        .arg("exec")
        .arg("--color")
        .arg("never")
        .arg("--ephemeral")
        .arg("--skip-git-repo-check")
        .arg("--dangerously-bypass-approvals-and-sandbox")
        .arg("-C")
        .arg(repo_root.as_str())
        .arg("-m")
        .arg(&request.model)
        .arg("-c")
        .arg(format!(
            "model_reasoning_effort={:?}",
            request.reasoning_effort
        ))
        .arg("-o")
        .arg(output_last_message.path())
        .arg(prompt)
        .output()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => anyhow::anyhow!(
                "Codex auto-commit requires `codex` on PATH; install/login to Codex CLI and retry"
            ),
            _ => anyhow::Error::new(error)
                .context(format!("failed to launch Codex CLI in {repo_root}")),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let final_message = std::fs::read_to_string(output_last_message.path())
        .ok()
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty());

    if !output.status.success() {
        bail!(
            "Codex auto-commit failed (exit {}): {}",
            output.status,
            summarize_output(&final_message, &stderr, &stdout)
        );
    }

    Ok(AutoCommitOutcome {
        final_message,
        stdout,
        stderr,
    })
}

fn build_prompt(repo_root: &Utf8Path, dirty_paths: &[Utf8PathBuf]) -> String {
    let mut prompt = String::new();
    prompt.push_str("Work in the current Git repository only.\n");
    prompt.push_str("Do not modify file contents.\n");
    prompt.push_str("Do not create or delete files.\n");
    prompt.push_str("Only stage and commit the already-dirty paths listed below.\n");
    prompt.push_str("Create exactly one Git commit with a concise message summarizing the pending skillnet mirror/cache changes.\n");
    prompt.push_str("If any unlisted path is dirty, or if you cannot commit exactly these paths, exit with failure.\n");
    prompt.push_str("Dirty paths:\n");
    for path in dirty_paths {
        prompt.push_str("- ");
        prompt.push_str(path.as_str());
        prompt.push('\n');
    }
    prompt.push_str("\nRepository root: ");
    prompt.push_str(repo_root.as_str());
    prompt.push('\n');
    prompt
}

fn summarize_output(final_message: &Option<String>, stderr: &str, stdout: &str) -> String {
    final_message
        .as_deref()
        .filter(|message| !message.is_empty())
        .or_else(|| (!stderr.is_empty()).then_some(stderr))
        .or_else(|| (!stdout.is_empty()).then_some(stdout))
        .unwrap_or("Codex did not return any output")
        .to_string()
}
