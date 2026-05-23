use anyhow::Result;

use crate::{
    calibration::{self, Db},
    cli::args::{AnalyzeFormat, CalibrationArgs, CalibrationCommand, Decision},
};

pub fn run(args: CalibrationArgs) -> Result<()> {
    let mut db = Db::open(&Db::default_path())?;
    match args.command {
        CalibrationCommand::Record { plan_dir } => {
            calibration::record::run(plan_dir.as_std_path(), &mut db)
        }
        CalibrationCommand::Verify { plan_dir } => {
            calibration::record::run_verify(plan_dir.as_std_path(), &mut db)
        }
        // PHASE 03 commands here
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
