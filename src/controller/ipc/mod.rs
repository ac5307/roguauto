//! Converts one-byte Unix datagrams into daemon commands.

mod command;
mod transport;
mod socket;

pub(crate) use crate::controller::ipc::{
  socket::socket_path,
  transport::{send_command, DaemonServer},
  command::DaemonCommand,
};
