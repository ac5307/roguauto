//! Menu state and validated values shared by the controller and view.

pub(crate) mod frame_size;
pub(crate) mod config;
mod menu_model;

pub(crate) use menu_model::{
  ActionCompletion, ActionStart, MenuModel, NavigationDirection,
  PowerAction, SelectionChange,
};
