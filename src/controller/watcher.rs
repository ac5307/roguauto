//! Native file notifications and the configuration reload timer.

use notify::{Config, Event, EventKind, INotifyWatcher, RecursiveMode, Watcher};

use smithay_client_toolkit::reexports::calloop::{
  channel::{channel, Event as ChannelEvent, Sender},
  timer::{Timer, TimeoutAction},
  Error as LoopError, LoopHandle,
};

use std::{
  fs, io, path,
  path::Path,
  time::{Duration, Instant},
};

/// Quiet period before emitting a reload after a matching file notification.
const CONFIG_DEBOUNCE: Duration = Duration::from_millis(100);

/// Explains why configuration monitoring could not be started.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigWatchError {
  /// The supplied path could not be resolved relative to the current directory.
  #[error("failed to resolve config path `{path}`: {source}")]
  ResolvePath {
    path: Box<Path>,
    #[source]
    source: io::Error,
  },

  /// The configuration path did not name a file inside a directory.
  #[error("config path has no parent directory: {0}")]
  MissingParent(Box<Path>),

  /// The parent directory could not be created.
  #[error("failed to create config directory `{path}`: {source}")]
  CreateDir {
    path: Box<Path>,
    #[source]
    source: io::Error,
  },

  /// Native file monitoring could not be initialized.
  #[error("failed to create config watcher: {0}")]
  CreateWatcher(#[source] notify::Error),

  /// The parent directory could not be watched.
  #[error("failed to watch config directory '{path}': {source}")]
  WatchDir {
    path: Box<Path>,
    #[source]
    source: notify::Error,
  },

  /// The event loop could not receive notifications from the watcher.
  #[error("failed to register config watcher events: {0}")]
  RegisterSource(#[source] LoopError),
}

/// Notification delivered to the main thread after the configuration settles.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ConfigWatchEvent {
  /// The configuration file should be read again.
  Change,
}

/// Owns the native watch for the selected configuration file's directory.
pub(crate) struct ConfigWatcher {
  /// Dropping the watcher stops delivery of native file notifications.
  _watcher: INotifyWatcher,
}

impl ConfigWatcher {
  /// Watches the file's parent and registers a one-shot reload timer on demand.
  pub(crate) fn new<'ev, State: 'ev>(
    cfg_path: &(impl AsRef<Path> + ?Sized),
    loop_handle: &LoopHandle<'ev, State>,
    sender: Sender<ConfigWatchEvent>,
  ) -> Result<Self, ConfigWatchError> {
    let cfg_path = cfg_path.as_ref();
    let watched_path = path::absolute(cfg_path).map_err(|source| {
      ConfigWatchError::ResolvePath {
        path: cfg_path.into(),
        source,
      }
    })?;
    let watched_dir = watched_path
      .parent()
      .ok_or_else(|| ConfigWatchError::MissingParent(cfg_path.into()))?;

    fs::create_dir_all(watched_dir).map_err(|source| {
      ConfigWatchError::CreateDir {
        path: watched_dir.into(),
        source,
      }
    })?;

    let (event_sender, event_source) = channel();
    let watched_file = Box::<Path>::from(watched_path.as_path());
    let mut watcher = INotifyWatcher::new(
      move |event: notify::Result<Event>| match event {
        Ok(event) if affects_config(&event, &watched_file) => {
          if event_sender.send(Instant::now() + CONFIG_DEBOUNCE).is_err() {
            eprintln!("failed to forward config file notification");
          }
        },
        Ok(_) => {},
        Err(err) => eprintln!("config watcher error: {err}"),
      },
      Config::default(),
    )
    .map_err(ConfigWatchError::CreateWatcher)?;

    watcher
      .watch(watched_dir, RecursiveMode::NonRecursive)
      .map_err(|source| ConfigWatchError::WatchDir {
        path: watched_dir.into(),
        source,
      })?;

    // A source callback must not keep its own event loop alive.
    let weak_handle = loop_handle.downgrade();
    let mut last_timer = None;
    loop_handle
      .insert_source(event_source, move |event, _, _| {
        let Some(handle) = weak_handle.upgrade() else {
          return;
        };
        if let Some(token) = last_timer.take() {
          handle.remove(token);
        }

        let ChannelEvent::Msg(deadline) = event else {
          return;
        };
        let sender = sender.clone();
        match handle.insert_source(
          Timer::from_deadline(deadline),
          move |_, _, _| {
            if sender.send(ConfigWatchEvent::Change).is_err() {
              eprintln!("failed to forward config reload event");
            }
            TimeoutAction::Drop
          },
        ) {
          Ok(token) => last_timer = Some(token),
          Err(err) => eprintln!("failed to schedule config reload: {err}"),
        }
      })
      .map_err(|err| ConfigWatchError::RegisterSource(err.error))?;

    Ok(Self { _watcher: watcher })
  }
}

/// Matches either side of a create, modify, rename, or remove notification.
fn affects_config(event: &Event, cfg_path: &Path) -> bool {
  matches!(
    event.kind,
    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
  ) && event.paths.iter().any(|path| path == cfg_path)
}
