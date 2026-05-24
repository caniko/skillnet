use std::collections::BTreeMap;

use anyhow::{bail, Context};
use serde::Serialize;

use super::heuristics::HEURISTICS;
use crate::calibration::{db::DbParam as P, Db};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ThresholdSource {
    Default,
    Override {
        updated_at: String,
        updated_by: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct ThresholdOverride {
    threshold: f64,
    updated_at: String,
    updated_by: Option<String>,
}

pub struct ThresholdStore<'db> {
    db: &'db Db,
    overrides: BTreeMap<String, ThresholdOverride>,
}

impl<'db> ThresholdStore<'db> {
    pub fn load(db: &'db Db) -> anyhow::Result<Self> {
        seed_defaults(db)?;
        let overrides = db
            .query_all(
                "SELECT name, threshold, CAST(updated_at AS TEXT), updated_by
                 FROM heuristic_thresholds
                 WHERE updated_by IS NULL OR updated_by != 'seed'
                 ORDER BY name",
                &[],
                |row| {
                    Ok((
                        row.get_string(0)?,
                        ThresholdOverride {
                            threshold: row.get_f64(1)?,
                            updated_at: row.get_string(2)?,
                            updated_by: row.get_optional_string(3)?,
                        },
                    ))
                },
            )
            .context("failed to load heuristic threshold overrides")?
            .into_iter()
            .collect();
        Ok(Self { db, overrides })
    }

    pub fn get(&self, name: &str) -> f64 {
        self.get_optional(name).unwrap_or(f64::NAN)
    }

    pub fn get_optional(&self, name: &str) -> Option<f64> {
        self.overrides
            .get(name)
            .map(|entry| entry.threshold)
            .or_else(|| default_threshold(name))
    }

    pub fn default_threshold(&self, name: &str) -> Option<f64> {
        default_threshold(name)
    }

    pub fn source(&self, name: &str) -> Option<ThresholdSource> {
        if let Some(entry) = self.overrides.get(name) {
            Some(ThresholdSource::Override {
                updated_at: entry.updated_at.clone(),
                updated_by: entry.updated_by.clone(),
            })
        } else {
            default_threshold(name).map(|_| ThresholdSource::Default)
        }
    }

    pub fn set(&mut self, name: &str, value: f64, source: &str) -> anyhow::Result<()> {
        if default_threshold(name).is_none() {
            bail!("unknown heuristic threshold {name}");
        }
        self.db
            .execute(
                "INSERT INTO heuristic_thresholds (name, threshold, updated_by)
                 VALUES ($1, $2, $3)
                 ON CONFLICT(name) DO UPDATE SET
                    threshold = excluded.threshold,
                    updated_at = CURRENT_TIMESTAMP,
                    updated_by = excluded.updated_by",
                &[P::from(name), P::from(value), P::from(source)],
            )
            .with_context(|| format!("failed to update heuristic threshold {name}"))?;
        let refreshed = Self::load(self.db)?;
        self.overrides = refreshed.overrides;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, f64, ThresholdSource)> + '_ {
        HEURISTICS.iter().copied().map(|heuristic| {
            let name = heuristic.name();
            (
                name,
                self.get(name),
                self.source(name).unwrap_or(ThresholdSource::Default),
            )
        })
    }
}

fn seed_defaults(db: &Db) -> anyhow::Result<()> {
    for heuristic in HEURISTICS {
        db.execute(
            "INSERT INTO heuristic_thresholds (name, threshold, updated_by)
             VALUES ($1, $2, 'seed')
             ON CONFLICT(name) DO NOTHING",
            &[
                P::from(heuristic.name()),
                P::from(heuristic.default_threshold()),
            ],
        )
        .with_context(|| format!("failed to seed heuristic threshold {}", heuristic.name()))?;
    }
    Ok(())
}

fn default_threshold(name: &str) -> Option<f64> {
    HEURISTICS
        .iter()
        .find(|heuristic| heuristic.name() == name)
        .map(|heuristic| heuristic.default_threshold())
}
