use crate::{domain::PowerAction, controller::ipc::DaemonCommand};

/// Events delivered to the main event loop by background adapters.
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
