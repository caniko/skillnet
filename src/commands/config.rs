use std::{env, fs, io};

use anyhow::{bail, Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    cli::args::ConfigCommand,
    config::{
        default_catalog_config_path, default_config_path, legacy_catalog_config_path,
        legacy_config_path,
    },
};

enum MigrateAction {
    NoLegacyAbsent,
    AlreadyCentralised,
    MoveLegacyToXdg,
    DeleteLegacyEqualContent,
    RefuseDifferent,
    ForceOverwrite,
}

pub(crate) fn run(command: ConfigCommand, global_dry_run: bool) -> Result<()> {
    match command {
        ConfigCommand::Migrate {
            dry_run,
            force,
            remove_breadcrumbs,
        } => {
            let dry_run = dry_run || global_dry_run;
            if remove_breadcrumbs {
                return remove_breadcrumb_files(dry_run);
            }

            migrate_one(
                "skillnet.toml",
                &legacy_config_path(),
                &default_config_path()?,
                dry_run,
                force,
            )?;
            migrate_one(
                "skillnet.catalog.toml",
                &legacy_catalog_config_path(),
                &default_catalog_config_path()?,
                dry_run,
                force,
            )?;
            Ok(())
        }
    }
}

fn classify(legacy: &Utf8Path, xdg: &Utf8Path, force: bool) -> Result<MigrateAction> {
    let legacy_exists = legacy.exists();
    let xdg_exists = xdg.exists();

    match (legacy_exists, xdg_exists) {
        (false, false) => Ok(MigrateAction::NoLegacyAbsent),
        (false, true) => Ok(MigrateAction::AlreadyCentralised),
        (true, false) => Ok(MigrateAction::MoveLegacyToXdg),
        (true, true) => {
            let legacy_bytes =
                fs::read(legacy).with_context(|| format!("failed to read {legacy}"))?;
            let xdg_bytes = fs::read(xdg).with_context(|| format!("failed to read {xdg}"))?;
            if legacy_bytes == xdg_bytes {
                Ok(MigrateAction::DeleteLegacyEqualContent)
            } else if force {
                Ok(MigrateAction::ForceOverwrite)
            } else {
                Ok(MigrateAction::RefuseDifferent)
            }
        }
    }
}

fn migrate_one(
    file_name: &str,
    legacy: &Utf8Path,
    xdg: &Utf8Path,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    match classify(legacy, xdg, force)? {
        MigrateAction::NoLegacyAbsent => {
            println!("{file_name}: no config to migrate");
            Ok(())
        }
        MigrateAction::AlreadyCentralised => {
            println!("{file_name}: already centralised at {xdg}");
            Ok(())
        }
        MigrateAction::MoveLegacyToXdg => {
            if dry_run {
                println!("{file_name}: would move {legacy} -> {xdg}");
                return Ok(());
            }

            move_legacy_to_xdg(legacy, xdg)?;
            write_breadcrumb(file_name, legacy, xdg)?;
            println!("{file_name}: moved to {xdg}");
            Ok(())
        }
        MigrateAction::DeleteLegacyEqualContent => {
            if dry_run {
                println!("{file_name}: would delete {legacy} (XDG copy is byte-identical)");
                return Ok(());
            }

            fs::remove_file(legacy).with_context(|| format!("failed to remove {legacy}"))?;
            write_breadcrumb(file_name, legacy, xdg)?;
            println!("{file_name}: deleted {legacy}; XDG copy retained at {xdg}");
            Ok(())
        }
        MigrateAction::RefuseDifferent => {
            bail!(
                "{file_name}: both {legacy} and {xdg} exist and differ; pass --force to overwrite XDG with legacy contents"
            )
        }
        MigrateAction::ForceOverwrite => {
            if dry_run {
                println!("{file_name}: would overwrite {xdg} with {legacy} contents (--force)");
                return Ok(());
            }

            move_legacy_to_xdg(legacy, xdg)?;
            write_breadcrumb(file_name, legacy, xdg)?;
            println!("{file_name}: overwrote {xdg} with {legacy} contents (--force)");
            Ok(())
        }
    }
}

fn remove_breadcrumb_files(dry_run: bool) -> Result<()> {
    for path in [
        breadcrumb_path("skillnet.toml", &legacy_config_path()),
        breadcrumb_path("skillnet.catalog.toml", &legacy_catalog_config_path()),
    ] {
        if !path.exists() {
            println!("breadcrumb {path} not present");
            continue;
        }

        if dry_run {
            println!("would remove breadcrumb {path}");
        } else {
            fs::remove_file(&path).with_context(|| format!("failed to remove {path}"))?;
            println!("removed breadcrumb {path}");
        }
    }

    Ok(())
}

fn move_legacy_to_xdg(legacy: &Utf8Path, xdg: &Utf8Path) -> Result<()> {
    let parent = xdg
        .parent()
        .with_context(|| format!("XDG config path {xdg} has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {parent}"))?;
    move_file(legacy, xdg)
        .with_context(|| format!("failed to move {legacy} to XDG config path {xdg}"))
}

fn move_file(src: &Utf8Path, dst: &Utf8Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(18) => {
            // EXDEV: XDG_CONFIG_HOME and cwd may be on different filesystems.
            fs::copy(src, dst)?;
            fs::remove_file(src)
        }
        Err(err) => Err(err),
    }
}

fn write_breadcrumb(file_name: &str, legacy: &Utf8Path, xdg: &Utf8Path) -> Result<()> {
    let breadcrumb = breadcrumb_path(file_name, legacy);
    let xdg = absolute_path(xdg)?;
    if let Err(err) = fs::write(&breadcrumb, format!("{xdg}\n")) {
        eprintln!("warning: could not write breadcrumb {breadcrumb}: {err}");
    }
    Ok(())
}

fn breadcrumb_path(file_name: &str, legacy: &Utf8Path) -> Utf8PathBuf {
    let parent = legacy
        .parent()
        .filter(|path| !path.as_str().is_empty())
        .unwrap_or_else(|| Utf8Path::new("."));
    parent.join(format!(".{file_name}.moved-to-xdg"))
}

fn absolute_path(path: &Utf8Path) -> Result<Utf8PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let cwd = Utf8PathBuf::from_path_buf(env::current_dir()?)
        .map_err(|path| anyhow::anyhow!("current directory is not UTF-8: {}", path.display()))?;
    Ok(cwd.join(path))
}
