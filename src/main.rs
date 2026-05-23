mod cache;
mod calibration;
mod catalog;
mod cli;
mod commands;
mod config;
mod fs_ops;
mod model;
mod reconcile;

fn main() -> anyhow::Result<()> {
    cli::run()
}
