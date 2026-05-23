use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Write},
};

use anyhow::Context;
use camino::Utf8Path;

use super::{query, Db};

#[derive(Clone, Copy, Debug)]
pub enum ExportFormat {
    Jsonl,
}

pub fn migrate(_db: &Db) -> anyhow::Result<()> {
    println!("migrated calibration database");
    Ok(())
}

pub fn vacuum(db: &Db) -> anyhow::Result<()> {
    db.execute_batch("VACUUM;")
        .context("failed to vacuum calibration database")?;
    println!("vacuumed calibration database");
    Ok(())
}

pub fn export(db: &Db, format: ExportFormat, out: Option<&Utf8Path>) -> anyhow::Result<()> {
    match format {
        ExportFormat::Jsonl => export_jsonl(db, out),
    }
}

fn export_jsonl(db: &Db, out: Option<&Utf8Path>) -> anyhow::Result<()> {
    let mut writer: Box<dyn Write> = match out {
        Some(path) => Box::new(BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| {
                    format!(
                        "failed to create export file {path}; refusing to overwrite existing files"
                    )
                })?,
        )),
        None => Box::new(BufWriter::new(io::stdout())),
    };

    let ids = db
        .query_all(
            "SELECT id FROM plans ORDER BY created_at ASC, id ASC",
            &[],
            |row| row.get_string(0),
        )
        .context("failed to query export plan ids")?;
    for id in ids {
        let dump = query::load_plan_dump(db, &id)?;
        serde_json::to_writer(&mut writer, &dump).context("failed to write export row")?;
        writer
            .write_all(b"\n")
            .context("failed to write export newline")?;
    }
    writer.flush().context("failed to flush export output")?;
    Ok(())
}
