// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

//! The command-line is parsed using [`clap`], but the implementation details are not exposed.
//! Instead, the [`parse_cli()`] function, which returns a [`RunMode`], is all that's necessary.

use clap::{Parser, Subcommand};

use crate::{Path, daemon::command::DaemonCommand};

#[derive(Parser, Debug, Default)]
#[command(about, long_about = None, version, propagate_version = true)]
struct Cli {
  #[command(subcommand)]
  pub command: CliCommand,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
  /// Start the daemon.
  Daemon {
    /// Optional configuation file path.
    #[arg(short = 'c', long = "config")]
    config_path: Option<Box<Path>>,
  },

  /// Check the health of daemon.
  Ping,

  /// Show the power menu.
  Show,

  /// Hide the power menu.
  Hide,

  /// Toggle the power menu.
  Toggle,

  /// Stop the daemon.
  Quit,
}

impl Default for CliCommand {
  fn default() -> Self {
    Self::Daemon { config_path: None }
  }
}

/// The modes that the program can run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
  /// Start the daemon.
  Daemon { config_path: Option<Box<Path>> },
  /// Send a command to the daemon.
  Send(DaemonCommand),
}

impl From<CliCommand> for RunMode {
  fn from(cmd: CliCommand) -> Self {
    match cmd {
      CliCommand::Daemon { config_path } => RunMode::Daemon { config_path },
      CliCommand::Ping => RunMode::Send(DaemonCommand::Ping),
      CliCommand::Show => RunMode::Send(DaemonCommand::Show),
      CliCommand::Hide => RunMode::Send(DaemonCommand::Hide),
      CliCommand::Toggle => RunMode::Send(DaemonCommand::Toggle),
      CliCommand::Quit => RunMode::Send(DaemonCommand::Quit),
    }
  }
}

/// Parses the command-line and returns a [`RunMode`].
pub fn parse_cli() -> RunMode {
  RunMode::from(Cli::parse().command)
}
