use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::ErrorKind,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::schema::Cooldown;

const REPO_COOLDOWN: u64 = 30 * 24 * 60 * 60;

#[derive(Default, Deserialize, Serialize)]
struct SessionState {
    cards: BTreeSet<String>,
    tokens: usize,
}

type RepoState = BTreeMap<String, u64>;

#[derive(Default)]
pub(super) struct State {
    session: SessionState,
    repo: RepoState,
    now: u64,
    session_changed: bool,
    repo_changed: bool,
}

impl State {
    pub(super) fn seen(&self, id: &str, cooldown: Cooldown) -> bool {
        match cooldown {
            Cooldown::Always => false,
            Cooldown::Session => self.session.cards.contains(id),
            Cooldown::Repo30d => self
                .repo
                .get(id)
                .is_some_and(|&time| self.now.saturating_sub(time) < REPO_COOLDOWN),
        }
    }

    pub(super) fn tokens(&self) -> usize {
        self.session.tokens
    }

    pub(super) fn record(&mut self, id: &str, cooldown: Cooldown, tokens: usize) {
        self.session.tokens += tokens;
        self.session_changed = true;
        match cooldown {
            Cooldown::Always => {}
            Cooldown::Session => {
                self.session.cards.insert(id.to_owned());
            }
            Cooldown::Repo30d => {
                self.repo
                    .retain(|_, time| self.now.saturating_sub(*time) < REPO_COOLDOWN);
                self.repo.insert(id.to_owned(), self.now);
                self.repo_changed = true;
            }
        }
    }
}

pub(super) fn with_state(
    dir: Option<&Utf8Path>,
    root: &Utf8Path,
    harness: &str,
    session: Option<&str>,
    needs_repo: bool,
    apply: impl FnOnce(&mut State),
) -> Result<()> {
    if session.is_none() && !needs_repo {
        apply(&mut State::default());
        return Ok(());
    }
    let dir = dir.map(Utf8Path::to_owned).map_or_else(default_dir, Ok)?;
    fs::create_dir_all(&dir).with_context(|| format!("create trigger state directory {dir}"))?;
    let root_hash = hash(&[root.as_str()]);
    let lock_path = dir.join(format!("{root_hash}.lock"));
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open trigger lock {lock_path}"))?;
    lock.lock()
        .with_context(|| format!("lock trigger state {lock_path}"))?;
    let session_path =
        session.map(|id| dir.join(format!("session-{root_hash}-{}.json", hash(&[harness, id]))));
    let repo_path = needs_repo.then(|| dir.join(format!("repo-{root_hash}.json")));
    let mut session_changed = false;
    let mut state = State {
        session: read(session_path.as_deref(), Some(&mut session_changed))?,
        repo: read(repo_path.as_deref(), None)?,
        session_changed,
        now: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        ..State::default()
    };
    apply(&mut state);
    if let Some(path) = repo_path.filter(|_| state.repo_changed) {
        write(&path, &state.repo, &dir)?;
    }
    if let Some(path) = session_path.filter(|_| state.session_changed) {
        write(&path, &state.session, &dir)?;
    }
    Ok(())
}

fn hash(parts: &[&str]) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize()
}

fn default_dir() -> Result<Utf8PathBuf> {
    let base = env::var("XDG_STATE_HOME")
        .ok()
        .filter(|dir| !dir.is_empty())
        .map(Utf8PathBuf::from)
        .or_else(|| {
            env::var("HOME")
                .ok()
                .filter(|dir| !dir.is_empty())
                .map(|home| Utf8PathBuf::from(home).join(".local/state"))
        })
        .context("set --state-dir, XDG_STATE_HOME or HOME to store trigger state")?;
    Ok(base.join("skillnet/trigger-state"))
}

fn read<T: DeserializeOwned + Default>(
    path: Option<&Utf8Path>,
    reset_invalid: Option<&mut bool>,
) -> Result<T> {
    let Some(path) = path else {
        return Ok(T::default());
    };
    match fs::read(path) {
        Ok(bytes) => match (serde_json::from_slice(&bytes), reset_invalid) {
            (Err(err), Some(changed)) => {
                eprintln!("skillnet: warning: resetting corrupt session state {path}: {err}");
                *changed = true;
                Ok(T::default())
            }
            (result, _) => result.with_context(|| format!("parse trigger state {path}")),
        },
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err).with_context(|| format!("read trigger state {path}")),
    }
}

fn write(path: &Utf8Path, value: &impl Serialize, dir: &Utf8Path) -> Result<()> {
    let write = || -> Result<()> {
        let mut temp = NamedTempFile::new_in(dir)?;
        serde_json::to_writer(&mut temp, value)?;
        temp.persist(path)?;
        Ok(())
    };
    write().with_context(|| format!("save trigger state {path}"))
}

#[test]
fn repo_cooldown_expires_at_thirty_days() {
    let mut state = State {
        now: REPO_COOLDOWN,
        ..State::default()
    };
    state.record("rust/check", Cooldown::Repo30d, 10);
    state.now += REPO_COOLDOWN - 1;
    assert!(state.seen("rust/check", Cooldown::Repo30d));
    state.now += 1;
    assert!(!state.seen("rust/check", Cooldown::Repo30d));
}
