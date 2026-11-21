//! Unix datagram client and background command listener.

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use tokio::{
  net::UnixDatagram as TokioUnixDatagram, runtime::Handle, task::JoinHandle,
};

use std::{
  fs, io,
  path::Path,
  os::unix::{fs::PermissionsExt, net::UnixDatagram as StdUnixDatagram},
};

use crate::controller::{ipc::DaemonCommand, app_event::AppEvent};

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_SOCKET_MODE: u32 = 0o600;

/// The receive buffer is two bytes so malformed multi-byte commands can
/// be rejected instead of being silently truncated to one byte.
const COMMAND_BUFFER_SIZE: usize = 2;

/// Describes a failure to start the daemon command listener.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DaemonServerError {
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
pub(crate) struct DaemonServer {
  task: JoinHandle<()>,
  socket_path: Box<Path>,
}

impl DaemonServer {
  /// Binds the control socket and forwards valid commands to the main loop.
  pub(crate) async fn start(
    runtime: &Handle,
    socket_path: Box<Path>,
    event_tx: Sender<AppEvent>,
  ) -> Result<Self, DaemonServerError> {
    let socket = bind_socket(socket_path.as_ref())?;

    let task = runtime.spawn(async move {
      let mut buf = [0u8; COMMAND_BUFFER_SIZE];

      loop {
        let length = match socket.recv_from(&mut buf).await {
          Ok((len, _addr)) => len,
          Err(err) => {
            break eprintln!("daemon socket receive failed: {err}");
          },
        };

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

        // Stop accepting commands once Quit reaches the main loop.
        if cmd == DaemonCommand::Quit {
          break;
        }
      }
    });

    Ok(Self { task, socket_path })
  }
}

impl Drop for DaemonServer {
  fn drop(&mut self) {
    self.task.abort();

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

/// Binds the private socket, recovering a stale path when necessary.
fn bind_socket(
  socket_path: &Path,
) -> Result<TokioUnixDatagram, DaemonServerError> {
  let parent = socket_path.parent().ok_or_else(|| {
    DaemonServerError::PrepareDirectory(io::Error::new(
      io::ErrorKind::InvalidInput,
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
      io::ErrorKind::AddrInUse => {
        if socket_accepts_datagrams(socket_path) {
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

/// Probes the socket by sending a ping datagram.
fn socket_accepts_datagrams(socket_path: &Path) -> bool {
  let Ok(probe) = StdUnixDatagram::unbound() else {
    return false;
  };

  probe
    .send_to(&[DaemonCommand::Ping.encode()], socket_path)
    .is_ok()
}

/// Removes an old socket path that can no longer receive commands.
fn remove_stale_socket(socket_path: &Path) -> Result<(), DaemonServerError> {
  match fs::remove_file(socket_path) {
    Ok(_) => Ok(()),
    Err(err) => match err.kind() {
      io::ErrorKind::NotFound => Ok(()),
      _ => Err(DaemonServerError::RemoveStaleSocket(err)),
    },
  }
}

/// Sends one encoded command to the daemon's control socket.
pub(crate) fn send_command(
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
