//! `skillnet` is a CLI for reconciling local AI skill directories into a
//! checked-in mirror, editing mirrored skills, syncing mirror state back to
//! live agent directories, and recording calibration data for
//! `multi-phase-plan`.
//!
//! The supported interface in `0.1.1` is the `skillnet` binary. This crate
//! does not commit to a stable embeddable Rust API yet.

mod cache;
pub mod calibration;
mod catalog;
pub mod cli;
mod commands;
mod config;
mod fs_ops;
mod model;
mod reconcile;
mod vcs;
