use anyhow::Result;

use crate::{
    calibration::{self, Db},
    cli::args::{
        AnalyzeFormat, CalibrationArgs, CalibrationCommand, Decision, EvalFormat, ExportFormat,
        HeuristicCategoryArg, HeuristicsCommand, HeuristicsFormat, QueryFormat,
    },
    config::DbTarget,
};

pub fn run(args: CalibrationArgs, target: DbTarget) -> Result<()> {
    if let CalibrationCommand::ShapeHash { plan_dir } = &args.command {
        return calibration::shape_hash::run(plan_dir.as_std_path());
    }

    let mut db = open_db(target)?;
    match args.command {
        CalibrationCommand::Record { plan_dir } => {
            calibration::record::run(plan_dir.as_std_path(), &mut db)
        }
        CalibrationCommand::Verify { plan_dir } => {
            calibration::record::run_verify(plan_dir.as_std_path(), &mut db)
        }
        CalibrationCommand::Init {
            plan_dir,
            stdout,
            force,
        } => calibration::init::run(
            &db,
            plan_dir.as_std_path(),
            calibration::init::InitOptions { stdout, force },
        ),
        CalibrationCommand::Eval { plan_dir, format } => calibration::eval::run(
            &db,
            plan_dir.as_std_path(),
            match format {
                EvalFormat::Json => calibration::eval::OutputFormat::Json,
                EvalFormat::Table => calibration::eval::OutputFormat::Table,
            },
        ),
        CalibrationCommand::MetaHeuristics { plan_dir, sidecar } => calibration::meta_cmd::run(
            &db,
            plan_dir.as_std_path(),
            sidecar.as_deref().map(camino::Utf8Path::as_std_path),
        ),
        CalibrationCommand::ShapeHash { .. } => unreachable!("handled before db open"),
        CalibrationCommand::Heuristics { command } => match command {
            HeuristicsCommand::List { format, category } => calibration::heuristics_cmd::list(
                &db,
                match format {
                    HeuristicsFormat::Json => calibration::heuristics_cmd::OutputFormat::Json,
                    HeuristicsFormat::Table => calibration::heuristics_cmd::OutputFormat::Table,
                },
                category.map(category_filter),
            ),
            HeuristicsCommand::Show { name } => calibration::heuristics_cmd::show(&db, &name),
        },
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

fn category_filter(category: HeuristicCategoryArg) -> calibration::heuristics_cmd::CategoryFilter {
    match category {
        HeuristicCategoryArg::Coordination => {
            calibration::heuristics_cmd::CategoryFilter::Coordination
        }
        HeuristicCategoryArg::Risk => calibration::heuristics_cmd::CategoryFilter::Risk,
        HeuristicCategoryArg::PlanShape => calibration::heuristics_cmd::CategoryFilter::PlanShape,
        HeuristicCategoryArg::QualityLint => {
            calibration::heuristics_cmd::CategoryFilter::QualityLint
        }
    }
}

fn open_db(target: DbTarget) -> Result<Db> {
    let db = match target {
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
    }?;
    calibration::catalog::ThresholdStore::load(&db)?;
    Ok(db)
}
