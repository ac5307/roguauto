// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use zbus::{proxy, Result as ZRslt, Connection};
use tokio::runtime::Handle;
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::{async_event::AppEvent};

#[proxy(
  interface = "org.freedesktop.login1.Session",
  default_service = "org.freedesktop.login1",
  default_path = "/org/freedesktop/login1/session/auto"
)]
trait Session {
  fn lock(&self) -> ZRslt<()>;
  fn terminate(&self) -> ZRslt<()>;
}

#[proxy(
  interface = "org.freedesktop.login1.Manager",
  default_service = "org.freedesktop.login1",
  default_path = "/org/freedesktop/login1"
)]
trait Manager {
  fn suspend(&self, interactive: bool) -> ZRslt<()>;
  fn hibernate(&self, interactive: bool) -> ZRslt<()>;
  fn reboot(&self, interactive: bool) -> ZRslt<()>;
  fn power_off(&self, interactive: bool) -> ZRslt<()>;
}

#[derive(Debug, Clone, Copy)]
pub enum PowerAction {
  Lock,
  Logout,
  Suspend,
  Hibernate,
  Reboot,
  Shutdown,
}

const _: () = assert!(!PowerAction::ALL.is_empty());

impl PowerAction {
  pub const ALL: [Self; 6] = [
    Self::Lock,
    Self::Logout,
    Self::Suspend,
    Self::Hibernate,
    Self::Reboot,
    Self::Shutdown,
  ];

  pub const fn label(self) -> &'static str {
    match self {
      Self::Lock => "Lock",
      Self::Logout => "Log out",
      Self::Suspend => "Suspend",
      Self::Hibernate => "Hibernate",
      Self::Reboot => "Reboot",
      Self::Shutdown => "Shut down",
    }
  }

  pub const fn icon(self) -> &'static [u8] {
    match self {
      Self::Lock => include_bytes!("../assets/icons/lock.svg"),
      Self::Logout => include_bytes!("../assets/icons/logout.svg"),
      Self::Suspend => include_bytes!("../assets/icons/suspend.svg"),
      Self::Hibernate => include_bytes!("../assets/icons/hibernate.svg"),
      Self::Reboot => include_bytes!("../assets/icons/reboot.svg"),
      Self::Shutdown => include_bytes!("../assets/icons/shutdown.svg"),
    }
  }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct PowerConnection(Connection);

impl PowerConnection {
  pub async fn new() -> ZRslt<Self> {
    Ok(Self(Connection::system().await?))
  }

  pub async fn lock(&self) -> ZRslt<()> {
    let proxy = SessionProxy::new(&self.0).await?;
    proxy.lock().await
  }

  pub async fn logout(&self) -> ZRslt<()> {
    let proxy = SessionProxy::new(&self.0).await?;
    proxy.terminate().await
  }

  pub async fn suspend(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.suspend(true).await
  }

  pub async fn hibernate(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.hibernate(true).await
  }

  pub async fn reboot(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.reboot(true).await
  }

  pub async fn shutdown(&self) -> ZRslt<()> {
    let proxy = ManagerProxy::new(&self.0).await?;
    proxy.power_off(true).await
  }
}

pub struct PowerHandler {
  pwr_conn: PowerConnection,
  runtime: Handle,
  sender: Sender<AppEvent>,
}

impl PowerHandler {
  pub async fn new(
    runtime: Handle,
    sender: Sender<AppEvent>,
  ) -> ZRslt<Self> {
    Ok(Self {
      pwr_conn: PowerConnection::new().await?,
      runtime,
      sender,
    })
  }

  pub fn execute(&self, action: PowerAction) {
    let pwr_conn = self.pwr_conn.clone();
    let sender = self.sender.clone();

    self.runtime.spawn(async move {
      let result = match action {
        PowerAction::Lock => pwr_conn.lock().await,
        PowerAction::Logout => pwr_conn.logout().await,
        PowerAction::Suspend => pwr_conn.suspend().await,
        PowerAction::Hibernate => pwr_conn.hibernate().await,
        PowerAction::Reboot => pwr_conn.reboot().await,
        PowerAction::Shutdown => pwr_conn.shutdown().await,
      }
      .map_err(|err| Box::from(err.to_string()));

      let _ = sender.send(AppEvent::PowerActionFinished { action, result });
    });
  }
}
