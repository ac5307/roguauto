// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use smithay_client_toolkit::reexports::{
  calloop::{
    EventLoop,
    channel::{channel, Event as ChannelEvent},
  },
  calloop_wayland_source::WaylandSource,
  client::{Connection, globals::registry_queue_init},
};

use crate::{
  io, Path,
  app::WaylandApp,
  async_event::AppEvent,
  config::{
    locate::{ConfigSource, ConfigLocation},
    deserialize::{ConfigFileError, Configuration},
    watch::{ConfigWatchEvent, ConfigWatcher},
  },
  daemon::{ipc::DaemonServer, socket::socket_path},
  power::PowerHandler,
};

pub fn startup(
  explicit_path: Option<&(impl AsRef<Path> + ?Sized)>,
) -> Result<(), Box<dyn std::error::Error>> {
  // Create a Tokio runtime.
  let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(1)
    .thread_name(format!("{}-async", crate::NAMESPACE))
    .enable_io()
    .build()?;

  // Create an event loop for the app.
  let mut event_loop = EventLoop::<WaylandApp>::try_new()?;
  let loop_handle = event_loop.handle();

  let (event_tx, event_rx) = channel::<AppEvent>();

  // Start the daemon.
  let daemon_server = runtime.block_on(
    // Try starting the daemon.
    DaemonServer::start(runtime.handle(), socket_path()?, event_tx.clone()),
  )?;

  // Initialize a new handler for executing power actions.
  let power_handler = runtime.block_on(
    // Create a new power-handler.
    PowerHandler::new(runtime.handle().clone(), event_tx),
  )?;

  /*
   * Wayland and app related setup process.
   */

  let conn = Connection::connect_to_env()?;
  let (globals, event_queue) = registry_queue_init::<WaylandApp>(&conn)?;
  let app_q_handle = event_queue.handle();
  let cfg_q_handle = event_queue.handle();

  // Locate the config file and load it.
  let config_location = ConfigLocation::find(explicit_path)?;
  eprintln!(
    "{}",
    match config_location.source() {
      ConfigSource::Explicit => {
        "Using the given explicit path to a config file"
      },
      ConfigSource::Home => {
        "Found config file in the user configuration directory"
      },
      ConfigSource::Default => {
        "Unable to locate config file; falling back to default path"
      },
    }
  );
  let config = if config_location.source() == ConfigSource::Default {
    // Default source means the file was not found.
    Configuration::default()
  } else {
    Configuration::load(config_location.path())?
  };

  let mut app =
    WaylandApp::new(globals, &event_queue.handle(), power_handler, config)?;
  loop_handle.insert_source(
    WaylandSource::new(conn, event_queue),
    |_, queue, app| queue.dispatch_pending(app),
  )?;

  loop_handle.insert_source(event_rx, move |event, _, app| match event {
    ChannelEvent::Msg(event) => {
      app.handle_app_event(event, &app_q_handle);
    },
    ChannelEvent::Closed => app.running = false,
  })?;

  let (cfg_tx, cfg_rx) = channel::<ConfigWatchEvent>();
  let _watcher = ConfigWatcher::new(config_location.path(), cfg_tx)?;
  eprintln!("Watching {}", config_location.path().display());
  loop_handle.insert_source(cfg_rx, move |event, _, app| match event {
    ChannelEvent::Msg(ConfigWatchEvent::Change) => {
      let new_config = match Configuration::load(config_location.path()) {
        Ok(config) => config,
        Err(ConfigFileError::Read { source, .. })
          if source.kind() == io::ErrorKind::NotFound =>
        {
          eprintln!("config file remained absent; using built-in defaults");
          Configuration::default()
        },
        Err(err) => {
          return eprintln!("failed to reload config: {err}");
        },
      };

      app.apply_config(new_config, &cfg_q_handle);
      eprintln!("configuration reloaded!");
    },
    ChannelEvent::Closed => {
      eprintln!("config watcher channel closed");
    },
  })?;

  while app.running {
    event_loop.dispatch(None, &mut app)?;
  }

  // Stop the daemon socket task before dropping the runtime.
  // `daemon_server` is dropped here and removes the socket.
  // `runtime` is dropped afterward.
  drop(daemon_server);
  drop(runtime);

  Ok(())
}
