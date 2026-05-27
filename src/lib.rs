//! `skillnet` is a CLI for managing canonical AI skill stores, materialising
//! derived skill views, and recording calibration data for `multi-phase-plan`.
//!
//! The supported interface in `0.6.0` is the `skillnet` binary. This crate
//! does not commit to a stable embeddable Rust API yet.

pub mod calibration;
mod catalog;
pub mod cli;
mod commands;
mod config;
pub mod exit;
mod fs_ops;
pub mod mirror;
pub mod model;
mod vcs;
pub mod view;
