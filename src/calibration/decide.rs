use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};

use super::{catalog::ThresholdStore, db::DbParam as P, Db};

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
            "SELECT decision, trigger_name, proposed_threshold
             FROM calibration_proposals WHERE id = $1",
            &[P::from(proposal_id)],
            |row| Ok((row.get_string(0)?, row.get_string(1)?, row.get_f64(2)?)),
        )
        .context("failed to read proposal decision")?;

    let Some(existing) = existing else {
        bail!("proposal {proposal_id} not found");
    };
    let (existing_decision, trigger_name, proposed_threshold) = existing;
    if existing_decision == "accepted" && matches!(decision, Decision::Accept) {
        println!("proposal {proposal_id} already accepted");
        return Ok(());
    }
    if existing_decision != "pending" {
        bail!("proposal {proposal_id} is already {existing_decision}");
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
    if matches!(decision, Decision::Accept) {
        let source = format!("proposal:{proposal_id}");
        ThresholdStore::load(db)?.set(&trigger_name, proposed_threshold, &source)?;
    }

    println!("proposal {proposal_id} {stored}");
    Ok(())
}

fn unix_timestamp() -> anyhow::Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    i64::try_from(duration.as_secs()).context("unix timestamp does not fit in i64")
}
