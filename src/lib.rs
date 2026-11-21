//! `skillnet` is a CLI for managing canonical AI skill stores, materialising
//! derived skill views, and recording calibration data for `multi-phase-plan`.
//!
//! The CLI and the `templates` and `models` modules are supported interfaces.

pub(crate) mod bundle;
pub mod calibration;
mod catalog;
pub mod cli;
mod commands;
mod config;
pub mod exit;
mod fs_ops;
pub mod link;
pub(crate) mod manifest;
pub mod mirror;
pub mod model;
pub mod models;
mod project_discovery;
pub mod templates;
mod vcs;
pub mod view;
