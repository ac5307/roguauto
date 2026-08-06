// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use tokio::{
  net::UnixDatagram as TokioUnixDatagram, runtime::Handle, task::JoinHandle,
};

use smithay_client_toolkit::reexports::calloop::channel::Sender;

use std::os::unix::{net::UnixDatagram as StdUnixDatagram, fs::PermissionsExt};

use crate::{
  fs, io, Path, async_event::AppEvent, daemon::command::DaemonCommand,
};

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_SOCKET_MODE: u32 = 0o600;

/// The receive buffer is two bytes so malformed multi-byte commands can
/// be rejected instead of being silently truncated to one byte.
const COMMAND_BUFFER_SIZE: usize = 2;

#[derive(Debug, thiserror::Error)]
pub enum DaemonServerError {
  #[error("another daemon instance is already running")]
  AlreadyRunning,

  #[error("failed to prepare daemon runtime directory: {0}")]
  PrepareDirectory(#[source] io::Error),

  #[error("failed to bind daemon socket: {0}")]
  Bind(#[source] io::Error),

  #[error("failed to set daemon socket permissions: {0}")]
  SetPermissions(#[source] io::Error),

  #[error("failed to remove stale daemon socket: {0}")]
  RemoveStaleSocket(#[source] io::Error),
}

/// Owns the background server task and socket path.
///
/// Dropping this value aborts the Tokio task and removes the socket file.
pub struct DaemonServer {
  task: JoinHandle<()>,
  socket_path: Box<Path>,
}

impl DaemonServer {
  pub async fn start(
    runtime: &Handle,
    socket_path: Box<Path>,
    event_tx: Sender<AppEvent>,
  ) -> Result<Self, DaemonServerError> {
    let socket = bind_socket(socket_path.as_ref())?;

    let task = runtime.spawn(async move {
      let mut buf = [0u8; COMMAND_BUFFER_SIZE];

      loop {
        // Length of the command received.
        let length = match socket.recv_from(&mut buf).await {
          Ok((len, _addr)) => len,
          Err(err) => {
            break eprintln!("daemon socket receive failed: {err}");
          },
        };

        // Each command should be one byte.
        if length != 1 {
          eprintln!(
            "Ignoring malformed daemon command containing {length} bytes"
          );
          continue;
        }

        let Some(cmd) = DaemonCommand::decode(buf[0]) else {
          eprintln!("Ignoring unknown daemon command byte: {}", buf[0]);
          continue;
        };

        if event_tx.send(AppEvent::DaemonCommand(cmd)).is_err() {
          // The Calloop receiver was dropped, meaning the UI thread is
          // shutting down.
          break;
        }

        // Successfully sending a `quit` command also stops the daemon.
        if cmd == DaemonCommand::Quit {
          break;
        }
      }
    });

    Ok(Self { task, socket_path })
  }

  /// Aborts the Tokio task.
  #[inline]
  pub fn abort(&self) {
    self.task.abort();
  }
}

impl Drop for DaemonServer {
  fn drop(&mut self) {
    self.abort();

    match fs::remove_file(&self.socket_path) {
      Ok(()) => {},
      Err(err) => match err.kind() {
        // Fine if the file is already gone.
        io::ErrorKind::NotFound => {},
        // Otherwise, report the error.
        _ => eprintln!(
          "failed to remove daemon socket {}: {err}",
          self.socket_path.display()
        ),
      },
    };
  }
}

fn bind_socket(
  socket_path: &Path,
) -> Result<TokioUnixDatagram, DaemonServerError> {
  use io::{Error, ErrorKind};

  let parent = socket_path.parent().ok_or_else(|| {
    DaemonServerError::PrepareDirectory(Error::new(
      ErrorKind::InvalidInput,
      "daemon socket has no parent directory",
    ))
  })?;

  fs::create_dir_all(parent).map_err(DaemonServerError::PrepareDirectory)?;

  fs::set_permissions(
    parent,
    fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
  )
  .map_err(DaemonServerError::PrepareDirectory)?;

  let socket = match TokioUnixDatagram::bind(socket_path) {
    Ok(socket) => socket,
    Err(err) => match err.kind() {
      ErrorKind::AddrInUse => {
        if existing_daemon_responds(socket_path) {
          return Err(DaemonServerError::AlreadyRunning);
        }

        remove_stale_socket(socket_path)?;

        TokioUnixDatagram::bind(socket_path)
          .map_err(DaemonServerError::Bind)?
      },
      _ => return Err(DaemonServerError::Bind(err)),
    },
  };

  fs::set_permissions(
    socket_path,
    fs::Permissions::from_mode(PRIVATE_SOCKET_MODE),
  )
  .map_err(DaemonServerError::SetPermissions)?;

  Ok(socket)
}

fn existing_daemon_responds(socket_path: &Path) -> bool {
  let Ok(probe) = StdUnixDatagram::unbound() else {
    return false;
  };

  probe
    .send_to(&[DaemonCommand::Ping.encode()], socket_path)
    .is_ok()
}

fn remove_stale_socket(socket_path: &Path) -> Result<(), DaemonServerError> {
  match fs::remove_file(socket_path) {
    Ok(_) => Ok(()),
    Err(err) => match err.kind() {
      io::ErrorKind::NotFound => Ok(()),
      _ => Err(DaemonServerError::RemoveStaleSocket(err)),
    },
  }
}

/// A function for the client-sde to send a [`command`](DaemonCommand) to
/// the given `socket_path`.
pub fn send_command(
  socket_path: &(impl AsRef<Path> + ?Sized),
  cmd: DaemonCommand,
) -> io::Result<()> {
  let socket = StdUnixDatagram::unbound()?;

  if socket.send_to(&[cmd.encode()], socket_path)? == 0 {
    use io::{Error, ErrorKind};
    return Err(Error::new(
      ErrorKind::WriteZero,
      "daemon command was not fully sent",
    ));
  }
  Ok(())
}
