//! Assembles the daemon's event sources and keeps them alive during dispatch.

use smithay_client_toolkit::reexports::{
  calloop::{
    EventLoop, LoopHandle,
    channel::{channel, Event as ChannelEvent, Sender},
  },
  calloop_wayland_source::WaylandSource,
  client::{Connection, QueueHandle, globals::registry_queue_init},
};
use tokio::runtime::Runtime;

use std::{error::Error, path::Path};

use crate::{
  controller::{
    watcher::{ConfigWatchEvent, ConfigWatcher},
    ipc::{socket_path, DaemonServer},
    logind::LogindPowerExecutor,
    app_event::AppEvent,
  },
  domain::{config::ConfigFile, PowerAction},
  wayland::WaylandHost,
};

/// Keeps the socket task alive and stops it before dropping its runtime.
struct BackgroundServices {
  _server: DaemonServer,
  _runtime: Runtime,
}

impl BackgroundServices {
  /// Starts the socket listener and the executor on a dedicated Tokio worker.
  fn start(
    sender: Sender<AppEvent>,
  ) -> Result<(Self, LogindPowerExecutor), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
      .worker_threads(1)
      .thread_name(format!("{}-async", crate::NAMESPACE))
      .enable_io()
      .build()?;
    let server = runtime.block_on(DaemonServer::start(
      runtime.handle(),
      socket_path()?,
      sender.clone(),
    ))?;
    let executor = runtime.block_on(LogindPowerExecutor::new(
      runtime.handle().clone(),
      sender,
    ))?;

    Ok((
      Self {
        _server: server,
        _runtime: runtime,
      },
      executor,
    ))
  }
}

/// Binds the platform adapters and dispatches events until the host stops.
pub(crate) fn run_daemon(
  explicit_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
  let mut event_loop = EventLoop::<WaylandHost>::try_new()?;
  let loop_handle = event_loop.handle();
  let (sender, receiver) = channel::<AppEvent>();
  let (_background, power_executor) = BackgroundServices::start(sender)?;

  let connection = Connection::connect_to_env()?;
  let (globals, queue) = registry_queue_init::<WaylandHost>(&connection)?;
  let queue_handle = queue.handle();

  let config_file =
    ConfigFile::locate(explicit_path, PowerAction::ALL.len())?;
  let config = config_file.initial_config()?;
  let mut host =
    WaylandHost::new(globals, &queue_handle, power_executor, config)?;

  loop_handle.insert_source(
    WaylandSource::new(connection, queue),
    |_, queue, host| queue.dispatch_pending(host),
  )?;
  let event_queue = queue_handle.clone();
  loop_handle.insert_source(
    receiver,
    move |event, _, host| match event {
      ChannelEvent::Msg(event) => host.handle_app_event(event, &event_queue),
      ChannelEvent::Closed => host.request_shutdown(),
    },
  )?;
  let _watcher =
    register_settings_reload(&loop_handle, config_file, queue_handle)?;

  while host.is_running() {
    event_loop.dispatch(None, &mut host)?;
  }
  Ok(())
}

/// Connects the settings watcher to the host's transactional reload operation.
fn register_settings_reload(
  loop_handle: &LoopHandle<'_, WaylandHost>,
  config_file: ConfigFile,
  queue_handle: QueueHandle<WaylandHost>,
) -> Result<ConfigWatcher, Box<dyn Error>> {
  let (sender, receiver) = channel::<ConfigWatchEvent>();
  let watcher = ConfigWatcher::new(config_file.path(), loop_handle, sender)?;

  eprintln!("Watching {}", config_file.path().display());

  loop_handle.insert_source(
    receiver,
    move |event, _, host| match event {
      ChannelEvent::Msg(ConfigWatchEvent::Change) => {
        let config = match config_file.reload() {
          Ok(config) => config,
          Err(err) => return eprintln!("failed to reload config: {err}"),
        };
        match host.apply_config(config, &queue_handle) {
          Ok(()) => eprintln!("configuration reloaded!"),
          Err(err) => eprintln!(
            "failed to apply config `{}`: {err}",
            config_file.path().display()
          ),
        }
      },
      ChannelEvent::Closed => eprintln!("config watcher channel closed"),
    },
  )?;
  Ok(watcher)
}
