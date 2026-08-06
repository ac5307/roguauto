// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use notify_debouncer_full::{
  new_debouncer, DebouncedEvent, Debouncer, RecommendedCache,
  notify::{self, RecommendedWatcher, RecursiveMode, EventKind},
};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use std::time::Duration;

use crate::{fs, io, Path};

/// 1/10 of a second.
const CONFIG_DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum ConfgWatchError {
  #[error("config path has no parent directory: {0}")]
  MissingParent(Box<Path>),

  #[error("failed to create config directory `{path}`: {source}")]
  CreateDir {
    path: Box<Path>,

    #[source]
    source: io::Error,
  },

  #[error("failed to create config watcher: {0}")]
  CreateWatcher(#[source] notify::Error),

  #[error("failed to watch config directory '{path}': {source}")]
  WatchDir {
    path: Box<Path>,

    #[source]
    source: notify::Error,
  },
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigWatchEvent {
  Change,
}

pub struct ConfigWatcher {
  /// Keeps the watcher alive. Dropping this stops filesystem monitoring.
  _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl ConfigWatcher {
  pub fn new(
    cfg_path: &(impl AsRef<Path> + ?Sized),
    sender: Sender<ConfigWatchEvent>,
  ) -> Result<Self, ConfgWatchError> {
    let cfg_path = cfg_path.as_ref();

    let watched_dir = cfg_path
      .parent()
      .ok_or_else(|| ConfgWatchError::MissingParent(Box::from(cfg_path)))?;

    // Create the config's parent directory and its parent components.
    fs::create_dir_all(watched_dir).map_err(|source| {
      ConfgWatchError::CreateDir {
        path: Box::from(watched_dir),
        source,
      }
    })?;

    let watched_path = Box::from(cfg_path);

    // Create a debouncer.
    let mut _debouncer =
      new_debouncer(CONFIG_DEBOUNCE, None, move |result| {
        let events = match result {
          Ok(events) => events,
          Err(errors) => {
            for err in errors {
              eprintln!("config watcher error: {err}");
            }
            return;
          },
        };

        if should_reload(events, &watched_path)
          && sender.send(ConfigWatchEvent::Change).is_err()
        {
          eprintln!("failed to forward config reload event");
        }
      })
      .map_err(ConfgWatchError::CreateWatcher)?;

    _debouncer
      .watch(watched_dir, RecursiveMode::NonRecursive)
      .map_err(|source| ConfgWatchError::WatchDir {
        path: Box::from(watched_dir),
        source,
      })?;

    Ok(Self { _debouncer })
  }
}

fn should_reload(events: Vec<DebouncedEvent>, cfg_path: &Path) -> bool {
  events.iter().any(|event| {
    if !matches!(
      event.kind,
      EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
      return false;
    }

    event.paths.iter().any(|path| path == cfg_path)
  })
}
