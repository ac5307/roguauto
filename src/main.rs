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
//! and systemd-logind. Some actions may require permission from the system
//! authorization service.
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
//! Window height, item width, and icon size must be greater than zero. Font
//! size must be finite and at least one pixel. Line height must be finite and
//! at least the font size. Spacing and corner radii must be finite and
//! nonnegative. The font family must contain a non-whitespace character.
//! Colors accept `#RRGGBB` and `#RRGGBBAA`.
//!
//! A menu frame may use at most 64 MiB of pixel storage. The icon budget is
//! also 64 MiB and includes both sets of tinted icons during a reload and the
//! temporary mask. Invalid settings stop startup or leave the current settings
//! active during a reload. Removing the file restores defaults.
//!
//! # Architecture
//!
//! The menu follows Model–View–Controller. The domain is the model. It owns
//! validated settings, selection, and action progress. The controller forwards
//! typed commands to the model and returns an instruction for the host. The
//! view borrows settings and selection after each update and uses one layout
//! for both drawing and pointer hit testing.
//!
//! The Wayland host runs on the main Calloop event loop. It translates input
//! into commands and owns the layer surface and its shared-memory buffers.
//! A Tokio runtime receives daemon socket messages and runs power requests
//! through [`zbus`]. A Calloop channel delivers those events to the main thread.
//! The config file adapter reads TOML and supplies validated values. Native file
//! notifications restart a one-shot timer so a burst of writes reloads once.
//! The entry point, CLI, runtime, and Wayland host live directly under `src`,
//! alongside the `domain`, `controller`, and `view` directories.
//!
//! ## Rendering
//!
//! The view draws shapes, labels, and action icons through a drawing interface.
//! A [`Tiny Skia`](tiny_skia) adapter paints the frame, and the Wayland host
//! copies the pixels into a buffer shared with the compositor.
//! [`fontdb`] finds installed fonts, and [`Swash`](swash) shapes the
//! single-line labels and rasterizes their glyphs.
//! The view scales and tints the bundled PNG icons as needed.
//! Configuration controls color, dimensions, typography, spacing, and corner
//! radii.

#![allow(clippy::too_many_arguments)]

mod controller;
mod domain;
mod cli;
mod runtime;
mod wayland;
mod view;

use std::process;

const NAMESPACE: &str = env!("CARGO_PKG_NAME");

fn main() -> process::ExitCode {
  use cli::{parse_cli, RunMode};
  use controller::ipc::{send_command, socket_path};
  use runtime::run_daemon;

  let run = || match parse_cli() {
    RunMode::Daemon { config_path } => run_daemon(config_path.as_deref()),
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
