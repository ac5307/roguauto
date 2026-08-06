// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

/// Commands accepted by the running daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonCommand {
  /// Verify that a daemon is listening.
  Ping,

  /// Make the power menu visible.
  Show,

  /// Hide the power menu.
  Hide,

  /// Toggle the menu's visibility.
  Toggle,

  /// Terminate the daemon.
  Quit,
}

impl DaemonCommand {
  const PING: u8 = b'P';
  const SHOW: u8 = b'S';
  const HIDE: u8 = b'H';
  const TOGGLE: u8 = b'T';
  const QUIT: u8 = b'Q';

  /// Encode the command as its one-byte wire representation.
  pub const fn encode(self) -> u8 {
    match self {
      Self::Ping => Self::PING,
      Self::Show => Self::SHOW,
      Self::Hide => Self::HIDE,
      Self::Toggle => Self::TOGGLE,
      Self::Quit => Self::QUIT,
    }
  }

  /// Decode one command byte.
  pub const fn decode(byte: u8) -> Option<Self> {
    match byte {
      Self::PING => Some(Self::Ping),
      Self::SHOW => Some(Self::Show),
      Self::HIDE => Some(Self::Hide),
      Self::TOGGLE => Some(Self::Toggle),
      Self::QUIT => Some(Self::Quit),
      _ => None,
    }
  }

  /// Returns the name of the command.
  pub const fn name(self) -> &'static str {
    match self {
      Self::Ping => "ping",
      Self::Show => "show",
      Self::Hide => "hide",
      Self::Toggle => "toggle",
      Self::Quit => "quit",
    }
  }
}

impl fmt::Display for DaemonCommand {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    formatter.write_str(self.name())
  }
}
