use anyhow::Result;

use crate::{
    calibration::{self, Db},
    cli::args::{
        AnalyzeFormat, CalibrationArgs, CalibrationCommand, Decision, ExportFormat, QueryFormat,
    },
    config::DbTarget,
};

pub fn run(args: CalibrationArgs, target: DbTarget) -> Result<()> {
    let mut db = open_db(target)?;
    match args.command {
        CalibrationCommand::Record { plan_dir } => {
            calibration::record::run(plan_dir.as_std_path(), &mut db)
        }
        CalibrationCommand::Verify { plan_dir } => {
            calibration::record::run_verify(plan_dir.as_std_path(), &mut db)
        }
        // PHASE 03 commands here
        CalibrationCommand::Tag { plan_id, tags } => {
            calibration::tag::add_tags(&mut db, &plan_id, &tags)
        }
        CalibrationCommand::Untag { plan_id, tags } => {
            calibration::tag::remove_tags(&mut db, &plan_id, &tags)
        }
        CalibrationCommand::Show { plan_id } => calibration::query::show(&db, &plan_id),
        CalibrationCommand::Query {
            tag,
            trigger,
            fired,
            missed,
            limit,
            format,
        } => calibration::query::query(
            &db,
            calibration::query::QueryOptions {
                tags: tag,
                trigger,
                fired,
                missed,
                limit,
            },
            match format {
                QueryFormat::Table => calibration::query::OutputFormat::Table,
                QueryFormat::Json => calibration::query::OutputFormat::Json,
            },
        ),
        CalibrationCommand::Migrate => calibration::housekeeping::migrate(&db),
        CalibrationCommand::Vacuum => calibration::housekeeping::vacuum(&db),
        CalibrationCommand::Export { format, out } => calibration::housekeeping::export(
            &db,
            match format {
                ExportFormat::Jsonl => calibration::housekeeping::ExportFormat::Jsonl,
            },
            out.as_deref(),
        ),
        CalibrationCommand::Analyze {
            filter_tag,
            trigger,
            min_n,
            format,
        } => calibration::analyze::run(
            &db,
            calibration::analyze::AnalyzeOptions {
                filter_tags: filter_tag,
                trigger,
                min_n,
            },
            match format {
                AnalyzeFormat::Table => calibration::analyze::OutputFormat::Table,
                AnalyzeFormat::Json => calibration::analyze::OutputFormat::Json,
            },
        ),
        CalibrationCommand::Propose {
            trigger,
            new_threshold,
            filter_tag,
            rationale,
            supporting_plan_ids,
        } => calibration::propose::run(
            &mut db,
            calibration::propose::ProposeInput {
                trigger,
                new_threshold,
                filter_tags: filter_tag,
                rationale,
                supporting_plan_ids,
            },
        ),
        CalibrationCommand::Proposals {
            pending,
            accepted,
            rejected,
        } => calibration::propose::list(
            &db,
            calibration::propose::ProposalFilter::from_flags(pending, accepted, rejected),
        ),
        CalibrationCommand::Decide {
            proposal_id,
            decision,
            rationale,
        } => calibration::decide::run(
            &mut db,
            proposal_id,
            match decision {
                Decision::Accept => calibration::decide::Decision::Accept,
                Decision::Reject => calibration::decide::Decision::Reject,
            },
            rationale,
        ),
        CalibrationCommand::ExportChangelog { since } => {
            calibration::changelog::run(&db, since.as_deref())
        } // PHASE 04 commands here
    }
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
                    "Postgres calibration database requested, but this skillnet binary was built \
                     without the `postgres` feature; rebuild with `--features postgres` or select \
                     the sqlite backend"
                )
            }
        }
    }
}
