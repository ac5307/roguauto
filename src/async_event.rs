// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{daemon::command::DaemonCommand, power::PowerAction};

#[derive(Debug)]
pub enum AppEvent {
  /// A command received through the daemon socket
  DaemonCommand(DaemonCommand),

  /// Completion of an asynchronous power action.
  PowerActionFinished {
    action: PowerAction,
    result: Result<(), Box<str>>,
  },
}
