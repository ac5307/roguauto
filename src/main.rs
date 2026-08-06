// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

//! **Roguauto** is a lightweight and configurable power-menu daemon for Wayland, written in Rust.
//!
//! The application runs as a persistent per-user daemon and displays a native
//! [`wlr-layer-shell`](https://wayland.app/protocols/wlr-layer-shell-unstable-v1)
//! overlay when requested through its local ipc interface.
//!
//! # App features
//!
//! - Lock, suspend, hibernate, reboot, shut down, and log out.
//! - Native Wayland rendering without GTK or inheritance from the system theme.
//! - A fully application-controlled visual style.
//! - TOML configuration with filesystem-based hot reloading.
//! - Keyboard and pointer navigation.
//! - A persistent system D-Bus connection to systemd-logind.
//! - A simple Unix-domain-socket daemon interface.
//!
//! # Compatibility
//!
//! The compositor must support the `wlr-layer-shell` protocol.
//!
//! The application was developed and tested primarily on [Niri](https://github.com/niri-wm/niri).
//! Other compositors implementing `wlr-layer-shell`, including
//! Hyprland, Sway, Labwc, KWin, etc, are expected to work.
//!
//! GNOME Shell does not currently expose `wlr-layer-shell`, so it's not supported.
//!
//! Power operations additionally require **Linux**, a working system D-Bus,
//! and systemd-logind. Some actions may require authorization through
//! PolicyKit.
//!
//! # Usage
//!
//! The daemon is started with:
//!
//! ```text
//! rogu daemon [-c <file> | --config <file>]
//! ```
//!
//! A second invocation controls the running daemon:
//!
//! ```text
//! rogu show
//! rogu hide
//! rogu toggle
//! rogu quit
//! ```
//!
//! # Architecture
//!
//! The main thread runs a Calloop event loop responsible for:
//!
//! - Wayland and Smithay Client Toolkit events,
//! - keyboard and pointer input,
//! - shared-memory buffer management,
//! - rendering,
//! - configuration reload notifications.
//!
//! A Tokio runtime handles asynchronous work such as:
//!
//! - systemd-logind calls through [`zbus`],
//! - daemon IPC,
//! - other background I/O.
//!
//! Results from asynchronous tasks are returned to the main thread
//! through a Calloop channel. Wayland objects remain confined to the
//! Calloop thread.
//!
//! # Rendering
//!
//! The menu is rendered using shared-memory buffers with [`Tiny Skia`](tiny_skia).
//! [`Cosmic Text`](cosmic_text) performs text shaping and glyph rasterization, while
//! embedded SVG assets provide scalable action icons.
//!
//! The renderer does not use GTK and therefore does not inherit the
//! user's GTK theme. Colors, dimensions, typography, spacing, and corner
//! radii are controlled by the application's configuration.

#![allow(clippy::too_many_arguments)]

mod daemon {
  pub mod socket;
  pub mod command;
  pub mod ipc;
}
mod config {
  pub mod locate;
  pub mod deserialize;
  pub mod watch;
}

mod app;
mod async_event;

mod power;

mod render;
mod cli;
mod init;

use std::{env, fs, io, process, path::Path};

const NAMESPACE: &str = env!("CARGO_PKG_NAME");

fn main() -> process::ExitCode {
  use cli::{parse_cli, RunMode};
  use init::startup;
  use daemon::{socket::socket_path, ipc::send_command};

  let run = || match parse_cli() {
    RunMode::Daemon { config_path } => startup(config_path.as_deref()),
    RunMode::Send(command) => {
      send_command(&socket_path()?, command)?;
      Ok(())
    },
  };

  match run() {
    Ok(_) => process::ExitCode::SUCCESS,
    Err(err) => {
      eprintln!("{err}");
      process::ExitCode::FAILURE
    },
  }
}
