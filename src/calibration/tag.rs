use anyhow::{bail, Context};

use super::{db::DbParam as P, Db};

const AUTO_TAG_KEYS: &[&str] = &["flavor", "scope", "risk", "signal", "worktype", "outcome"];

pub fn add_tags(db: &mut Db, plan_id: &str, tags: &[(String, String)]) -> anyhow::Result<()> {
    ensure_plan_exists(db, plan_id)?;
    db.transaction(|tx| {
        for (key, value) in tags {
            tx.execute(
                "INSERT INTO tags (plan_id, key, value) VALUES ($1, $2, $3)
                 ON CONFLICT(plan_id, key, value) DO NOTHING",
                &[P::from(plan_id), P::from(key), P::from(value)],
            )
            .with_context(|| format!("failed to add tag {key}={value}"))?;
        }
        Ok(())
    })
    .context("failed to commit calibration tag transaction")?;
    println!("tagged {plan_id} ({} tags)", tags.len());
    Ok(())
}

pub fn remove_tags(db: &mut Db, plan_id: &str, tags: &[(String, String)]) -> anyhow::Result<()> {
    ensure_plan_exists(db, plan_id)?;
    for (key, _) in tags {
        if AUTO_TAG_KEYS.contains(&key.as_str()) {
            bail!(
                "cannot remove derived auto-tag key '{key}'; auto-tag keys are flavor, scope, risk, signal, worktype, outcome"
            );
        }
    }

    db.transaction(|tx| {
        for (key, value) in tags {
            tx.execute(
                "DELETE FROM tags WHERE plan_id = $1 AND key = $2 AND value = $3",
                &[P::from(plan_id), P::from(key), P::from(value)],
            )
            .with_context(|| format!("failed to remove tag {key}={value}"))?;
        }
        Ok(())
    })
    .context("failed to commit calibration untag transaction")?;
    println!("untagged {plan_id} ({} tags)", tags.len());
    Ok(())
}

fn ensure_plan_exists(db: &Db, plan_id: &str) -> anyhow::Result<()> {
    let exists: bool = db
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM plans WHERE id = $1)",
            &[P::from(plan_id)],
            |row| row.get_bool(0),
        )
        .context("failed to check plan existence")?;
    if !exists {
        bail!("unknown calibration plan id {plan_id}");
    }
    Ok(())
}
