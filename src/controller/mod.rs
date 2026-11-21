//! Routes typed menu commands and groups the daemon's input and effect adapters.

pub(crate) mod watcher;
pub(crate) mod ipc;
pub(crate) mod logind;
pub(crate) mod app_event;

use crate::{
  domain::{
    config::menu_config::MenuConfig, ActionCompletion, ActionStart,
    MenuModel, NavigationDirection, PowerAction, SelectionChange,
  },
};

/// A request or completion delivered by a platform event source.
pub(crate) enum MenuCommand {
  Show,
  Hide,
  Toggle,
  Quit,
  Previous,
  Next,
  Select(PowerAction),
  ActivateSelected,
  Activate(PowerAction),
  ActionFinished {
    action: PowerAction,
    result: Result<(), Box<str>>,
  },
}

/// Work for the platform to perform after a controller update.
pub(crate) enum MenuEffect {
  None,
  ShowMenu,
  HideMenu,
  ToggleMenu,
  RequestShutdown,
  Redraw,
  /// Redraws the selection before submitting the power request.
  StartPowerAction(PowerAction),
  /// Reports the action error and redraws a configured layer.
  ReportFailureAndRedraw {
    action: PowerAction,
    error: Box<str>,
  },
}

/// Interprets menu commands and updates the model before returning an effect.
pub(crate) struct MenuController {
  model: MenuModel,
}

impl MenuController {
  /// Creates the controller with the startup settings.
  pub(crate) fn new(config: MenuConfig) -> Self {
    Self {
      model: MenuModel::new(config),
    }
  }

  /// Borrows settings for a layout or frame.
  pub(crate) fn config(&self) -> &MenuConfig {
    self.model.config()
  }

  /// Replaces the active settings used by the model and view.
  pub(crate) fn replace_config(&mut self, config: MenuConfig) {
    self.model.replace_config(config);
  }

  /// Returns the action to highlight in the next frame.
  pub(crate) fn selected(&self) -> PowerAction {
    self.model.selected()
  }

  /// Applies one command and returns work to run after the update.
  pub(crate) fn update(&mut self, command: MenuCommand) -> MenuEffect {
    match command {
      MenuCommand::Show => MenuEffect::ShowMenu,
      MenuCommand::Hide => MenuEffect::HideMenu,
      MenuCommand::Toggle => MenuEffect::ToggleMenu,
      MenuCommand::Quit => MenuEffect::RequestShutdown,
      MenuCommand::Previous => self
        .model
        .move_selection(NavigationDirection::Previous)
        .into(),
      MenuCommand::Next => {
        self.model.move_selection(NavigationDirection::Next).into()
      },
      MenuCommand::Select(action) => self.model.select(action).into(),
      MenuCommand::ActivateSelected => {
        self.model.start_action(self.model.selected()).into()
      },
      MenuCommand::Activate(action) => {
        self.model.start_action(action).into()
      },
      MenuCommand::ActionFinished { action, result } => {
        match self.model.finish_action(result) {
          ActionCompletion::CloseMenu => MenuEffect::HideMenu,
          ActionCompletion::KeepMenuOpen => MenuEffect::Redraw,
          ActionCompletion::Failed(error) => {
            MenuEffect::ReportFailureAndRedraw { action, error }
          },
        }
      },
    }
  }
}

impl From<SelectionChange> for MenuEffect {
  fn from(change: SelectionChange) -> Self {
    match change {
      SelectionChange::Unchanged => Self::None,
      SelectionChange::Changed => Self::Redraw,
    }
  }
}

impl From<ActionStart> for MenuEffect {
  fn from(start: ActionStart) -> Self {
    match start {
      ActionStart::Blocked => Self::None,
      ActionStart::Started(action) => Self::StartPowerAction(action),
    }
  }
}
