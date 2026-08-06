// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{env, Path};

const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigPathError {
  #[error("config file does not exist: {0}")]
  PathNotFound(Box<Path>),

  #[error(
    "cannot determine the user config directory: neither XDG_CONFIG_HOME nor HOME is available"
  )]
  HomeUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
  Explicit,
  Home,
  Default,
}

#[derive(Debug, Clone)]
pub struct ConfigLocation {
  path: Box<Path>,
  source: ConfigSource,
}

impl ConfigLocation {
  pub fn find(
    explicit: Option<&(impl AsRef<Path> + ?Sized)>,
  ) -> Result<Self, ConfigPathError> {
    // Check, If a given path was explicitly given.
    if let Some(path) = explicit {
      let path = path.as_ref();
      if !path.is_file() {
        return Err(ConfigPathError::PathNotFound(Box::from(path)));
      }

      return Ok(Self {
        path: path.into(),
        source: ConfigSource::Explicit,
      });
    }

    if let Some(path) = env::var_os("XDG_CONFIG_HOME")
      .filter(|val| !val.is_empty())
      .as_deref()
      .map(Path::new)
      .filter(|path| path.is_absolute())
      .map(|path| path.join(crate::NAMESPACE).join(CONFIG_FILE_NAME))
      && path.is_file()
    {
      // Return if a config file was found with XDG_CONFIG_HOME.
      return Ok(Self {
        path: path.into(),
        source: ConfigSource::Home,
      });
    }

    match env::var_os("HOME")
      .filter(|val| !val.is_empty())
      .as_deref()
      .map(Path::new)
    {
      // Some user home directory exist:
      Some(home) => {
        // Construct the rest of the path to the config file.
        let path = home
          .join(".config")
          .join(crate::NAMESPACE)
          .join(CONFIG_FILE_NAME);

        // If a config file exist.
        if path.is_file() {
          return Ok(Self {
            path: path.into(),
            source: ConfigSource::Home,
          });
        }

        // No file currently exist. Return the default/normal/fallback
        // path so the daemon can watch for the file being created later.
        Ok(Self {
          path: path.into(),
          source: ConfigSource::Default,
        })
      },
      // Otherwise, user home directory does not exist.
      None => Err(ConfigPathError::HomeUnavailable),
    }
  }

  pub fn path(&self) -> &Path {
    self.path.as_ref()
  }

  pub fn source(&self) -> ConfigSource {
    self.source
  }
}
