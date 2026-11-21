//! Adapts menu power actions to systemd-logind calls on the system bus.

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use tokio::runtime::Handle;
use zbus::{proxy, Connection, Result as ZRslt};

use crate::{domain::PowerAction, controller::app_event::AppEvent};

/// Operations on the caller's logind session.
#[proxy(
  interface = "org.freedesktop.login1.Session",
  default_service = "org.freedesktop.login1",
  default_path = "/org/freedesktop/login1/session/auto"
)]
trait Session {
  /// Locks the session.
  fn lock(&self) -> ZRslt<()>;

  /// Terminates the session.
  fn terminate(&self) -> ZRslt<()>;
}

/// System-wide power operations exported by logind.
#[proxy(
  interface = "org.freedesktop.login1.Manager",
  default_service = "org.freedesktop.login1",
  default_path = "/org/freedesktop/login1"
)]
trait Manager {
  /// Suspends the system.
  fn suspend(&self, interactive: bool) -> ZRslt<()>;

  /// Hibernates the system.
  fn hibernate(&self, interactive: bool) -> ZRslt<()>;

  /// Reboots the system.
  fn reboot(&self, interactive: bool) -> ZRslt<()>;

  /// Powers off the system.
  fn power_off(&self, interactive: bool) -> ZRslt<()>;
}

/// Holds the system bus connection used by logind requests.
#[derive(Clone)]
#[repr(transparent)]
struct LogindConnection(Connection);

impl LogindConnection {
  /// Opens a connection to the system bus.
  async fn new() -> ZRslt<Self> {
    Ok(Self(Connection::system().await?))
  }

  /// Calls the logind method for the requested action.
  async fn run_action(&self, action: PowerAction) -> ZRslt<()> {
    match action {
      PowerAction::Lock => self.lock().await,
      PowerAction::Logout => self.logout().await,
      PowerAction::Suspend => self.suspend().await,
      PowerAction::Hibernate => self.hibernate().await,
      PowerAction::Reboot => self.reboot().await,
      PowerAction::Shutdown => self.shutdown().await,
    }
  }

  /// Requests a lock of the current session.
  async fn lock(&self) -> ZRslt<()> {
    let proxy = SessionProxy::new(&self.0).await?;
    proxy.lock().await
  }

  /// Requests termination of the current session.
  async fn logout(&self) -> ZRslt<()> {
    let proxy = SessionProxy::new(&self.0).await?;
    proxy.terminate().await
  }

  /// Requests suspend with interactive authorization.
  async fn suspend(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.suspend(true).await
  }

  /// Requests hibernation with interactive authorization.
  async fn hibernate(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.hibernate(true).await
  }

  /// Requests reboot with interactive authorization.
  async fn reboot(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.reboot(true).await
  }

  /// Requests power off with interactive authorization.
  async fn shutdown(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.power_off(true).await
  }
}

/// Schedules logind requests and delivers their results to the main loop.
pub(crate) struct LogindPowerExecutor {
  connection: LogindConnection,
  runtime: Handle,
  sender: Sender<AppEvent>,
}

impl LogindPowerExecutor {
  /// Opens the connection shared by tasks on the supplied runtime.
  pub(crate) async fn new(
    runtime: Handle,
    sender: Sender<AppEvent>,
  ) -> ZRslt<Self> {
    Ok(Self {
      connection: LogindConnection::new().await?,
      runtime,
      sender,
    })
  }

  /// Submits the action and sends its completion when the request finishes.
  pub(crate) fn execute(&self, action: PowerAction) {
    let connection = self.connection.clone();
    let sender = self.sender.clone();

    self.runtime.spawn(async move {
      let result = connection
        .run_action(action)
        .await
        .map_err(|err| Box::from(err.to_string()));

      let _ = sender.send(AppEvent::PowerActionFinished { action, result });
    });
  }
}
