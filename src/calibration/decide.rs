use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};

use super::{db::DbParam as P, Db};

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
        .query_optional(
            "SELECT decision FROM calibration_proposals WHERE id = $1",
            &[P::from(proposal_id)],
            |row| row.get_string(0),
        )
        .context("failed to read proposal decision")?;

    let Some(existing) = existing else {
        bail!("proposal {proposal_id} not found");
    };
    if existing != "pending" {
        bail!("proposal {proposal_id} is already {existing}");
    }

    let stored = decision.as_stored();
    db.execute(
        "UPDATE calibration_proposals
        SET decision = $1, decided_at = $2, rationale = $3
        WHERE id = $4 AND decision = 'pending'",
        &[
            P::from(stored),
            P::from(unix_timestamp()?),
            P::from(rationale.as_str()),
            P::from(proposal_id),
        ],
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
