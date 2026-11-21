use std::{
    fs,
    io::{ErrorKind, Write as _},
    process::{Command, Output},
};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;
use skillnet::templates::{self, BindingsDoc, Settings, SkillBinding};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    Fleet,
    Local,
    Drop,
}

pub fn edit_docs(
    fleet: &BindingsDoc,
    local: &BindingsDoc,
    scope: Apply,
    skill: &str,
    binding: &SkillBinding,
) -> (Option<BindingsDoc>, Option<BindingsDoc>) {
    let fleet = (scope == Apply::Fleet).then(|| {
        let mut doc = fleet.clone();
        doc.skills.insert(skill.into(), binding.clone());
        doc
    });
    let local = if scope == Apply::Local {
        let mut doc = local.clone();
        doc.skills.insert(skill.into(), binding.clone());
        Some(doc)
    } else if local.skills.contains_key(skill) {
        let mut doc = local.clone();
        doc.skills.remove(skill);
        Some(doc)
    } else {
        None
    };
    (fleet, local)
}

#[derive(Clone, Default)]
pub struct Applied {
    pub fleet: Option<BindingsDoc>,
    pub local: Option<BindingsDoc>,
    pub status: String,
    pub hosts: Option<String>,
    pub dropped: bool,
    /// Every write this action needed succeeded.
    pub saved: bool,
}

// No --allow-delete: a binding change never leaves stale view entries, and
// the UI must not prune anything authored by hand.
const SYNC_ARGS: [&str; 5] = [
    "--allow-dirty-destination",
    "sync",
    "--scope",
    "global",
    "--no-promote",
];

/// Paths the editor opened with: fleet, local, canonical.
pub type Opened = (Option<Utf8PathBuf>, Option<Utf8PathBuf>, Utf8PathBuf);

pub fn run(
    scope: Apply,
    config_path: &Utf8Path,
    opened: &Opened,
    skill: &str,
    binding: &SkillBinding,
    hosts_command: Option<Vec<String>>,
) -> Applied {
    let mut applied = Applied::default();
    let mut log = Vec::new();
    let writes = (|| -> Result<_> {
        // The sync below rereads skillnet.toml; refuse if it now points
        // elsewhere than the files this editor showed.
        let settings = Settings::load(config_path)?;
        let current: Opened = (settings.fleet, settings.local, settings.canonical);
        if &current != opened {
            bail!("{config_path} changed since the editor opened; reopen it before applying");
        }
        let (fleet_path, local_path, canonical) = current;
        let canonical = canonical.as_path();
        // Re-read both files so edits made elsewhere since the window opened
        // survive; only the selected skill changes.
        let load = |path: &Option<Utf8PathBuf>| {
            templates::load_existing(path.as_slice()).with_context(|| format!("reading {path:?}"))
        };
        let fresh_fleet = load(&fleet_path)?;
        // Whatever binding the skill ends up with must render against the
        // current template before any file is written.
        if scope == Apply::Drop {
            let fallback = fresh_fleet.skills.get(skill).with_context(|| {
                format!("no fleet binding for `{skill}` to fall back to; local override kept")
            })?;
            templates::render_skill(canonical, skill, fallback).with_context(|| {
                format!("fleet binding for `{skill}` does not render; local override kept")
            })?;
        } else {
            templates::render_skill(canonical, skill, binding).with_context(|| {
                format!("binding for `{skill}` does not render; nothing written")
            })?;
        }
        let fresh_local = load(&local_path)?;
        let (fleet, local) = edit_docs(&fresh_fleet, &fresh_local, scope, skill, binding);
        let fleet = fleet_path.zip(fleet);
        let local = local_path.zip(local);
        // Check every file this action replaces before writing any of them.
        let permissions = |file: &Option<(Utf8PathBuf, BindingsDoc)>| {
            file.as_ref()
                .map(|(path, _)| managed_permissions(path))
                .transpose()
                .map(Option::flatten)
        };
        let (fleet_permissions, local_permissions) = (permissions(&fleet)?, permissions(&local)?);
        let mut fleet_path = None;
        if let Some((path, doc)) = fleet {
            write_doc(&path, &doc, fleet_permissions).with_context(|| format!("writing {path}"))?;
            applied.fleet = Some(doc);
            fleet_path = Some(path);
        }
        if let Some((path, doc)) = local {
            write_doc(&path, &doc, local_permissions).with_context(|| format!("writing {path}"))?;
            applied.local = Some(doc);
        }
        // Refresh the editor from disk even when a file needed no write
        // (for example an override already removed elsewhere).
        applied.fleet.get_or_insert(fresh_fleet);
        applied.local.get_or_insert(fresh_local);
        applied.dropped = scope == Apply::Drop;
        applied.saved = true;
        Ok(fleet_path)
    })();
    match writes {
        Err(error) => log.push(format!("{error:#}")),
        Ok(fleet_path) => {
            // Sync the exact config this editor validated, never a rediscovered
            // one, and publish only a fleet file that sync accepted.
            let mut sync = Command::new("skillnet");
            sync.arg("--config").arg(config_path).args(SYNC_ARGS);
            match run_command(&mut sync) {
                Ok(output) => {
                    log.push(command_output(&output));
                    if let Some(path) = fleet_path {
                        match publish(&path, skill) {
                            Ok(()) => log.push("Published fleet bindings.".into()),
                            Err(error) => log.push(format!("{error:#}")),
                        }
                    }
                }
                Err(error) => log.push(format!("{error:#}; fleet file not published")),
            }
        }
    }
    if scope == Apply::Fleet {
        if let Some(command) = hosts_command {
            match hosts(&command) {
                Ok(hosts) => applied.hosts = Some(hosts),
                Err(error) => log.push(error),
            }
        }
    }
    log.retain(|line| !line.is_empty());
    applied.status = if log.is_empty() {
        "Applied".into()
    } else {
        log.join("\n\n")
    };
    applied
}

/// Refuse files skillnet did not write; keep the permissions of ones it did.
fn managed_permissions(path: &Utf8Path) -> Result<Option<fs::Permissions>> {
    // Replacing a link would leave its (versioned) target unchanged.
    if path.is_symlink() {
        bail!("{path} is a symlink; point the config at the file itself");
    }
    match fs::metadata(path) {
        Ok(meta) => {
            if !fs::read(path)?.starts_with(b"// Managed by skillnet; edits are overwritten.") {
                bail!("{path} is not managed by skillnet; refusing to overwrite");
            }
            Ok(Some(meta.permissions()))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_doc(
    path: &Utf8Path,
    doc: &BindingsDoc,
    permissions: Option<fs::Permissions>,
) -> Result<()> {
    let dir = parent_dir(path).context("bindings path has no parent")?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(templates::to_pkl(doc).as_bytes())?;
    if let Some(permissions) = permissions {
        file.as_file().set_permissions(permissions)?;
    }
    file.persist(path)?;
    Ok(())
}

fn parent_dir(path: &Utf8Path) -> Option<&Utf8Path> {
    path.parent()
        .map(|dir| if dir == "" { Utf8Path::new(".") } else { dir })
}

fn publish(path: &Utf8Path, skill: &str) -> Result<()> {
    let dir = parent_dir(path).context("fleet bindings path has no parent")?;
    let file = path
        .file_name()
        .context("fleet bindings path has no file name")?;
    let git = || {
        let mut git = Command::new("git");
        git.arg("-C").arg(dir).env("GIT_LITERAL_PATHSPECS", "1");
        git
    };
    let stdout = |command: &mut Command| -> Result<String> {
        Ok(String::from_utf8(run_command(command)?.stdout)?
            .trim()
            .to_owned())
    };
    // Commit only when the file differs, so a retry after a failed push
    // goes straight to publishing the earlier commit.
    if !stdout(git().args(["status", "--porcelain", "--", file]))?.is_empty() {
        // A path-limited commit needs an index entry if the binding file is new.
        run_command(git().args(["add", "--intent-to-add", "--", file])).context("git add")?;
        run_command(
            git()
                .args(["commit", "-m"])
                .arg(format!("chore(ai): bind {skill} skill models"))
                .args(["--", file]),
        )
        .context("git commit")?;
    }
    let tracked = String::from_utf8(
        run_command(git().args(["ls-files", "--full-name", "-z", "--", file]))?.stdout,
    )?;
    let tracked = tracked.trim_end_matches('\0');
    if tracked.is_empty() {
        bail!("bindings file is not tracked (gitignored?)");
    }
    // Push only bindings commits: other unpushed work on the branch is not
    // ours to publish.
    let branch = stdout(git().args(["symbolic-ref", "HEAD"])).context("detached HEAD")?;
    let upstream = stdout(git().args([
        "for-each-ref",
        "--format=%(upstream:remotename) %(upstream:remoteref)",
        &branch,
    ]))?;
    let (remote, remote_ref) = upstream
        .split_once(' ')
        .filter(|(remote, _)| !remote.is_empty())
        .with_context(|| format!("{branch} has no upstream; push skipped"))?;
    // Audit and push one pinned commit, so a concurrent commit cannot slip in.
    let head = stdout(git().args(["rev-parse", &branch]))?;
    let short = branch.strip_prefix("refs/heads/").unwrap_or(&branch);
    let upstream_sha = stdout(git().args(["rev-parse", &format!("{short}@{{u}}")]))?;
    if !git()
        .args(["merge-base", "--is-ancestor", &upstream_sha, &head])
        .status()?
        .success()
    {
        bail!("push skipped: branch diverged from upstream");
    }
    if head == upstream_sha {
        return Ok(());
    }
    let range = format!("{upstream_sha}..{head}");
    let root = stdout(git().args(["rev-parse", "--show-toplevel"]))?;
    let total = stdout(git().args(["rev-list", "--count", &range]))?;
    let bindings = stdout(git().args(["-C", &root, "rev-list", "--count", &range, "--", tracked]))?;
    if total != bindings {
        bail!("push skipped: unpushed commits other than bindings commits");
    }
    // -m lists merge changes per parent; --no-renames lists rename sources.
    let outgoing = String::from_utf8(
        run_command(git().args([
            "log",
            "-m",
            "--no-renames",
            "--no-relative",
            "--format=",
            "--name-only",
            "-z",
            &range,
        ]))?
        .stdout,
    )?;
    let mut touched: Vec<_> = outgoing
        .split('\0')
        .filter(|path| !path.is_empty())
        .collect();
    touched.sort_unstable();
    touched.dedup();
    if touched != [tracked] {
        bail!(
            "committed locally, push skipped: unpushed commits on {branch} also touch {}",
            touched
                .iter()
                .filter(|path| **path != tracked)
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    run_command(git().args([
        "push",
        "--no-follow-tags",
        &format!("--force-with-lease={remote_ref}:{upstream_sha}"),
        remote,
        &format!("{head}:{remote_ref}"),
    ]))
    .context("committed locally; git push failed (pull/rebase if remote moved)")?;
    Ok(())
}

/// Check whether `path` can be written by `skillnet-ui` (not in `/nix/store`,
/// not a symlink, and not read-only).
pub fn is_writable_target(path: &Utf8Path) -> bool {
    if path == Utf8Path::new("/nix/store") || path.as_str().starts_with("/nix/store/") {
        return false;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || meta.permissions().readonly() {
                return false;
            }
            parent_dir(path)
                .and_then(|dir| fs::metadata(dir).ok())
                .is_some_and(|dir| !dir.permissions().readonly())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let mut cursor = parent_dir(path);
            while let Some(dir) = cursor {
                match fs::metadata(dir) {
                    Ok(meta) => return meta.is_dir() && !meta.permissions().readonly(),
                    Err(err) if err.kind() == ErrorKind::NotFound => cursor = dir.parent(),
                    Err(_) => return false,
                }
            }
            false
        }
        Err(_) => false,
    }
}

pub fn hosts(command: &[String]) -> Result<String, String> {
    (|| -> Result<String> {
        let (program, args) = command.split_first().context("ui.hosts_command is empty")?;
        let output = run_command(Command::new(program).args(args))?;
        let json: Value = serde_json::from_slice(&output.stdout)?;
        let data = json["data"].as_array().context("expected a data array")?;
        let hosts = data
            .iter()
            .map(|row| {
                let host = row["host"].as_str().context("missing host")?;
                let kind = row["deviceType"].as_str().context("missing deviceType")?;
                Ok(format!("{host} — {kind}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(hosts.join("\n"))
    })()
    .map_err(|error| format!("Fleet hosts: {error:#}"))
}

fn run_command(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("could not start {command:?}"))?;
    if !output.status.success() {
        bail!(
            "{command:?} exited {}:\n{}",
            output.status,
            command_output(&output)
        );
    }
    Ok(output)
}

fn command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_doc_refuses_unmanaged_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let path = Utf8Path::from_path(temp.path())
            .unwrap()
            .join("bindings.pkl");
        let doc = BindingsDoc::default();
        fs::write(&path, "skills {}\n").unwrap();
        assert_eq!(
            managed_permissions(&path).unwrap_err().to_string(),
            format!("{path} is not managed by skillnet; refusing to overwrite")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "skills {}\n");
        fs::write(&path, templates::to_pkl(&doc)).unwrap();
        write_doc(&path, &doc, managed_permissions(&path).unwrap()).unwrap();
    }

    #[test]
    fn sync_never_prunes_views() {
        assert!(!SYNC_ARGS.contains(&"--allow-delete"));
    }

    #[test]
    fn writable_target_rejects_nix_store_symlinks_and_readonly() {
        assert!(!is_writable_target(Utf8Path::new(
            "/nix/store/abc-SkillModels.pkl"
        )));
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let writable = root.join("SkillModels.pkl");
        assert!(is_writable_target(&writable));
        fs::write(
            &writable,
            "// Managed by skillnet; edits are overwritten.\n",
        )
        .unwrap();
        assert!(is_writable_target(&writable));

        let link = root.join("link.pkl");
        std::os::unix::fs::symlink(&writable, &link).unwrap();
        assert!(!is_writable_target(&link));

        let mut perms = fs::metadata(&writable).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&writable, perms).unwrap();
        assert!(!is_writable_target(&writable));
    }

    #[test]
    fn publish_pushes_only_bindings_commits() {
        // Disposable fixture repos: no global config, hooks or signing.
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let git = |dir: &Utf8Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        let remote = root.join("remote.git");
        let work = root.join("work");
        git(
            root,
            &["init", "-q", "--bare", "-b", "trunk", remote.as_str()],
        );
        git(root, &["clone", "-q", remote.as_str(), work.as_str()]);
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
        ] {
            git(&work, &["config", key, value]);
        }
        git(&work, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(&work, &["push", "-q", "-u", "origin", "trunk"]);
        let file = work.join("lib/SkillModels.pkl");
        fs::create_dir_all(file.parent().unwrap()).unwrap();

        fs::write(&file, "skills {}\n").unwrap();
        publish(&file, "council").unwrap();
        assert_eq!(
            git(&remote, &["rev-parse", "trunk"]),
            git(&work, &["rev-parse", "HEAD"])
        );

        // Every outgoing commit must touch the bindings, even if it changes no paths.
        let pushed = git(&remote, &["rev-parse", "trunk"]);
        git(
            &work,
            &["switch", "-q", "-c", "empty", "--track", "origin/trunk"],
        );
        git(&work, &["commit", "-q", "--allow-empty", "-m", "empty"]);
        fs::write(&file, "skills { [\"council\"] { seats = List() } }\n").unwrap();
        let error = publish(&file, "council").unwrap_err().to_string();
        assert_eq!(
            error,
            "push skipped: unpushed commits other than bindings commits"
        );
        assert_eq!(git(&remote, &["rev-parse", "trunk"]), pushed);
        git(&work, &["switch", "-q", "trunk"]);

        // Unrelated unpushed work blocks publication; the bindings commit stays local.
        fs::write(work.join("other.txt"), "x").unwrap();
        git(&work, &["add", "other.txt"]);
        git(&work, &["commit", "-q", "-m", "other"]);
        fs::write(&file, "skills { [\"council\"] { seats = List() } }\n").unwrap();
        let error = publish(&file, "council").unwrap_err().to_string();
        assert_eq!(
            error,
            "push skipped: unpushed commits other than bindings commits"
        );
        assert_eq!(git(&remote, &["rev-parse", "trunk"]), pushed);
        assert!(git(&work, &["status", "--porcelain"]).is_empty());
    }
}
