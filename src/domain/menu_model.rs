use crate::domain::config::menu_config::{CloseAfterAction, MenuConfig};

/// Identifies a power operation independently of its label, icon, or service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerAction {
  Lock,
  Logout,
  Suspend,
  Hibernate,
  Reboot,
  Shutdown,
}

impl PowerAction {
  /// The order used for layout, arrow navigation, and number keys.
  pub(crate) const ALL: [Self; 6] = [
    Self::Lock,
    Self::Logout,
    Self::Suspend,
    Self::Hibernate,
    Self::Reboot,
    Self::Shutdown,
  ];
}

/// The direction to move through the action order.
pub(crate) enum NavigationDirection {
  Previous,
  Next,
}

/// Reports whether an operation changed the selection.
pub(crate) enum SelectionChange {
  Unchanged,
  Changed,
}

/// Reports whether an action can start.
pub(crate) enum ActionStart {
  Blocked,
  Started(PowerAction),
}

/// Describes what completing an action means for the current menu.
pub(crate) enum ActionCompletion {
  CloseMenu,
  KeepMenuOpen,
  Failed(Box<str>),
}

/// Tracks whether the model is waiting for the current action to finish.
enum ActionState {
  Ready,
  InFlight,
}

/// Owns menu settings and interaction state across surface changes.
pub(crate) struct MenuModel {
  config: MenuConfig,
  selected: PowerAction,
  action_state: ActionState,
}

impl MenuModel {
  /// Initializes the selection and makes the menu ready for its first action.
  pub(crate) fn new(config: MenuConfig) -> Self {
    Self {
      config,
      selected: PowerAction::Lock,
      action_state: ActionState::Ready,
    }
  }

  /// Borrows the current settings for layout, drawing, and action completion.
  pub(crate) fn config(&self) -> &MenuConfig {
    &self.config
  }

  /// Replaces settings while keeping the selection and action progress.
  pub(crate) fn replace_config(&mut self, config: MenuConfig) {
    self.config = config;
  }

  /// Returns the action currently selected in the menu.
  pub(crate) fn selected(&self) -> PowerAction {
    self.selected
  }

  /// Moves to the adjacent action and wraps at either end of the menu.
  pub(crate) fn move_selection(
    &mut self,
    direction: NavigationDirection,
  ) -> SelectionChange {
    let Some(index) = PowerAction::ALL
      .iter()
      .position(|action| *action == self.selected)
    else {
      return SelectionChange::Unchanged;
    };
    let count = PowerAction::ALL.len();
    let next = match direction {
      NavigationDirection::Previous => {
        index.checked_sub(1).unwrap_or(count - 1)
      },
      NavigationDirection::Next => (index + 1) % count,
    };
    self.select(PowerAction::ALL[next])
  }

  /// Selects the action when the menu is ready for input.
  pub(crate) fn select(&mut self, action: PowerAction) -> SelectionChange {
    if matches!(self.action_state, ActionState::InFlight)
      || self.selected == action
    {
      return SelectionChange::Unchanged;
    }
    self.selected = action;
    SelectionChange::Changed
  }

  /// Selects and starts an action together, even if it is already selected.
  pub(crate) fn start_action(&mut self, action: PowerAction) -> ActionStart {
    match self.action_state {
      ActionState::InFlight => ActionStart::Blocked,
      ActionState::Ready => {
        self.selected = action;
        self.action_state = ActionState::InFlight;
        ActionStart::Started(action)
      },
    }
  }

  /// Accepts input again and applies the current close setting.
  pub(crate) fn finish_action(
    &mut self,
    result: Result<(), Box<str>>,
  ) -> ActionCompletion {
    self.action_state = ActionState::Ready;
    match result {
      Ok(()) => match self.config.close_after_action() {
        CloseAfterAction::Close => ActionCompletion::CloseMenu,
        CloseAfterAction::KeepOpen => ActionCompletion::KeepMenuOpen,
      },
      Err(error) => ActionCompletion::Failed(error),
    }
  }
}
