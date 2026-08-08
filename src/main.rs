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
//! # A path to a config file can be optionally given.
//! rogu daemon [-c <file> | --config <file>]
//! ```
//!
//! In a second terminal window while the daemon is still running:
//!
//! ```text
//! rogu show
//! rogu hide
//! rogu toggle
//! rogu quit
//! ```
//!
//! However, normally, you would want to start the daemon as part of your
//! compositor's startup or autostart processes. And then, bind a keyboard
//! shortcut to `rogu toggle` and/or the remaining commands.
//!
//! # Configuration
//!
//! The app will try looking for a config file in 3 locations:
//! - the path given when starting the daemon,
//! - `$XDG_CONFIG_HOME/roguauto/config.toml`,
//! - `$HOME/.config/roguauto/config.toml`.
//!
//! Below is an example of what could go inside a configuration file:
//!
//! ```toml
//! [window]
//! background = "#192264"
//! height = 160
//! padding = 18
//! corner_radius = 25.0
//!
//! [item]
//! background = "#a1a4a9"
//! selected_background = "#a0d0f0"
//! width = 196
//! gap = 8
//! corner_radius = 10.5
//!
//! [text]
//! color = "#2d303a"
//! selected_color = "#f1f4f9"
//! font_family = "JetBrains Mono"      # use fonts that are installed on your machine.
//! font_size = 20
//! font_weight = 300
//! line_height = 25.0
//!
//! [icon]
//! size = 49
//! label_gap = 21.0
//!
//! [behavior]
//! close_after_action = true           # default value is already true.
//! ```
//!
//! The app itself has built-in defaults. Omitting any of the fields shown above
//! will use the default value for that particular field. Hence, if a config file
//! is not found, the defaults would all be used.
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
//! Text shaping and glyph rasterization are performed using
//! [`Cosmic Text`](cosmic_text), while embedded SVG assets provide scalable action icons.
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
