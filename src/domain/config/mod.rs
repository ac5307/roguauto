//! Limits filesystem and TOML handling to the settings file adapter.

mod file;
mod locate;
pub(crate) mod menu_config;

use std::{error::Error, io, path::Path};

use crate::domain::config::{
  file::{defaults, load, ConfigFileError},
  locate::{ConfigLocation, ConfigSource},
  menu_config::MenuConfig,
};

/// Owns the chosen settings path and loads validated snapshots from it.
pub(crate) struct ConfigFile {
  location: ConfigLocation,
  action_count: usize,
}

impl ConfigFile {
  /// Selects the file used for startup and subsequent reloads.
  pub(crate) fn locate(
    explicit_path: Option<&Path>,
    action_count: usize,
  ) -> Result<Self, Box<dyn Error>> {
    Ok(Self {
      location: ConfigLocation::find(explicit_path)?,
      action_count,
    })
  }

  /// Borrows the selected path for watching and diagnostics.
  pub(crate) fn path(&self) -> &Path {
    self.location.path()
  }

  /// Loads the startup configuration or validates the built-in defaults.
  pub(crate) fn initial_config(&self) -> Result<MenuConfig, Box<dyn Error>> {
    let source = self.location.source();
    eprintln!(
      "{}",
      match source {
        ConfigSource::Explicit =>
          "Using the given explicit path to a config file",
        ConfigSource::Home =>
          "Found config file in the user configuration directory",
        ConfigSource::Default =>
          "Unable to locate config file; falling back to default path",
      }
    );
    if source == ConfigSource::Default {
      Ok(defaults(self.action_count)?)
    } else {
      Ok(load(self.path(), self.action_count)?)
    }
  }

  /// Reloads the selected file, falling back to defaults after deletion.
  pub(crate) fn reload(&self) -> Result<MenuConfig, Box<dyn Error>> {
    match load(self.path(), self.action_count) {
      Ok(config) => Ok(config),
      Err(ConfigFileError::Read { source, .. })
        if source.kind() == io::ErrorKind::NotFound =>
      {
        eprintln!("config file remained absent; using built-in defaults");
        Ok(defaults(self.action_count)?)
      },
      Err(err) => Err(err.into()),
    }
  }
}
