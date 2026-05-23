use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use rusqlite::params;
use rusqlite::OptionalExtension;

use super::Db;

pub enum Decision {
    Accept,
    Reject,
}

impl Decision {
    fn as_stored(&self) -> &'static str {
        match self {
            Self::Accept => "accepted",
            Self::Reject => "rejected",
        }
    }
}

pub fn run(
    db: &mut Db,
    proposal_id: i64,
    decision: Decision,
    rationale: String,
) -> anyhow::Result<()> {
    let existing = db
        .connection()
        .query_row(
            "SELECT decision FROM calibration_proposals WHERE id = ?1",
            [proposal_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read proposal decision")?;

    let Some(existing) = existing else {
        bail!("proposal {proposal_id} not found");
    };
    if existing != "pending" {
        bail!("proposal {proposal_id} is already {existing}");
    }

    let stored = decision.as_stored();
    db.connection_mut().execute(
        "UPDATE calibration_proposals
        SET decision = ?1, decided_at = ?2, rationale = ?3
        WHERE id = ?4 AND decision = 'pending'",
        params![stored, unix_timestamp()?, rationale, proposal_id],
    )?;

    println!("proposal {proposal_id} {stored}");
    Ok(())
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}
