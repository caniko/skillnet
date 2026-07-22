use anyhow::{Context as AnyhowContext, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    cli::Scope,
    config::{Config, ProjectConfig},
    model::Target,
};

pub struct Context {
    pub config_path: Utf8PathBuf,
    pub catalog_config_path: Utf8PathBuf,
    pub config: Config,
    pub mirror_root: Utf8PathBuf,
    pub data_dir: Utf8PathBuf,
    pub dry_run: bool,
    pub allow_dirty_destination: bool,
}

impl Context {
    pub fn load(
        config_path: &Utf8Path,
        mirror_root: Option<&Utf8PathBuf>,
        catalog_config_path: &Utf8Path,
        dry_run: bool,
        allow_dirty_destination: bool,
    ) -> Result<Self> {
        let config = Config::load(config_path)?;
        let mirror_root = crate::cli::resolve_mirror_root(&config, mirror_root)?;
        let data_dir =
            match config.data_dir.as_deref() {
                Some(path) => crate::config::expand_path(path)?,
                None => Utf8PathBuf::from_path_buf(crate::config::default_data_dir()).map_err(
                    |path| anyhow::anyhow!("skillnet data directory is not UTF-8: {path:?}"),
                )?,
            };

        Ok(Self {
            config_path: config_path.to_path_buf(),
            catalog_config_path: catalog_config_path.to_path_buf(),
            config,
            mirror_root,
            data_dir,
            dry_run,
            allow_dirty_destination,
        })
    }

    pub fn ensure_target_clean(&self, target: &Utf8Path) -> Result<()> {
        if self.dry_run || self.allow_dirty_destination {
            return Ok(());
        }
        crate::vcs::ensure_clean(target)
            .with_context(|| format!("destination target `{target}` is not clean"))
    }

    pub(crate) fn ensure_destination_clean(&self) -> Result<()> {
        self.ensure_target_clean(&self.mirror_root)
    }

    pub(super) fn all_targets(&self) -> Result<Vec<Target>> {
        self.config.targets(&self.mirror_root)
    }

    pub(super) fn target(&self, scope: &Scope) -> Result<Target> {
        match scope {
            Scope::Global => self.config.global_target(&self.mirror_root),
            Scope::Project(name) => {
                let project = self
                    .project(name)
                    .with_context(|| format!("unknown target `{name}`"))?;
                self.config.project_target(&self.mirror_root, project)
            }
        }
    }

    pub(super) fn targets(&self, scopes: &[Scope]) -> Result<Vec<Target>> {
        scopes.iter().map(|scope| self.target(scope)).collect()
    }

    pub(super) fn project(&self, name: &str) -> Option<&ProjectConfig> {
        self.config
            .projects
            .iter()
            .find(|project| project.name == name)
    }

    pub(crate) fn bundle_plan(&self, target: &Target) -> Result<Option<crate::bundle::BundlePlan>> {
        let external = self
            .config
            .external_manifests
            .iter()
            .map(|path| crate::config::expand_path(path))
            .collect::<Result<Vec<_>>>()?;
        crate::bundle::plan(
            &target.canonical_path,
            &target.name,
            &self.data_dir,
            &external,
        )
    }
}
