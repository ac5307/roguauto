//! Reads menu settings from TOML and supplies their built-in defaults.

use serde::Deserialize;

use std::{fs, path::Path};

use crate::domain::config::menu_config::{
  CloseAfterAction, Color, MenuConfig, IconConfig, ItemConfig,
  ConfigValidationError, TextConfig, WindowConfig,
};

/// A file that could not be read, decoded, or validated.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigFileError {
  /// Reading the file failed.
  #[error("failed to read config `{path}`: {source}")]
  Read {
    path: Box<Path>,

    #[source]
    source: std::io::Error,
  },
  /// The TOML document could not be decoded.
  #[error("failed to parse config `{path}`: {source}")]
  Parse {
    path: Box<Path>,

    #[source]
    source: toml::de::Error,
  },
  /// The decoded settings could not be used together or failed a value check.
  #[error("invalid config `{path}`: {source}")]
  Validate {
    path: Box<Path>,

    #[source]
    source: ConfigValidationError,
  },
}

/// Loads TOML and validates it for the supplied number of menu items.
pub(crate) fn load(
  path: &(impl AsRef<Path> + ?Sized),
  item_count: usize,
) -> Result<MenuConfig, ConfigFileError> {
  let path = path.as_ref();
  let contents =
    fs::read_to_string(path).map_err(|source| ConfigFileError::Read {
      path: path.into(),
      source,
    })?;
  let input: ConfigInput =
    toml::from_str(&contents).map_err(|source| ConfigFileError::Parse {
      path: path.into(),
      source,
    })?;

  input
    .validate(item_count)
    .map_err(|source| ConfigFileError::Validate {
      path: path.into(),
      source,
    })
}

/// Validates the built-in defaults with the same rules as file settings.
pub(crate) fn defaults(
  item_count: usize,
) -> Result<MenuConfig, ConfigValidationError> {
  ConfigInput::default().validate(item_count)
}

/// Reads ASCII hex digits, using full opacity when the alpha pair is absent.
fn parse_color(
  value: &str,
  setting: &'static str,
) -> Result<Color, ConfigValidationError> {
  let invalid_color = || {
    ConfigValidationError::new(
      setting,
      "must use #RRGGBB or #RRGGBBAA with ASCII hex digits",
    )
  };
  let [b'#', hex @ ..] = value.as_bytes() else {
    return Err(invalid_color());
  };
  if !matches!(hex.len(), 6 | 8) {
    return Err(invalid_color());
  }

  let mut channels = [0, 0, 0, u8::MAX];
  for (channel, pair) in channels.iter_mut().zip(hex.chunks_exact(2)) {
    let high = hex_value(pair[0]).ok_or_else(invalid_color)?;
    let low = hex_value(pair[1]).ok_or_else(invalid_color)?;
    *channel = high * 16 + low;
  }
  Ok(Color::rgba(
    channels[0],
    channels[1],
    channels[2],
    channels[3],
  ))
}

/// Converts an ASCII hex digit to its numeric value.
fn hex_value(byte: u8) -> Option<u8> {
  match byte.to_ascii_lowercase() {
    digit @ b'0'..=b'9' => Some(digit - b'0'),
    digit @ b'a'..=b'f' => Some(digit - b'a' + 10),
    _ => None,
  }
}

/// The TOML sections before value validation.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ConfigInput {
  window: WindowInput,
  item: ItemInput,
  text: TextInput,
  icon: IconInput,
  behavior: BehaviorInput,
}

impl ConfigInput {
  /// Converts a decoded settings file into validated menu settings.
  fn validate(
    self,
    item_count: usize,
  ) -> Result<MenuConfig, ConfigValidationError> {
    let window = WindowConfig::new(
      parse_color(&self.window.background, "window.background")?,
      self.window.height,
      self.window.padding,
      self.window.corner_radius,
    )?;
    let item = ItemConfig::new(
      parse_color(&self.item.background, "item.background")?,
      parse_color(
        &self.item.selected_background,
        "item.selected_background",
      )?,
      self.item.width,
      self.item.gap,
      self.item.corner_radius,
    )?;
    let text = TextConfig::new(
      parse_color(&self.text.color, "text.color")?,
      parse_color(&self.text.selected_color, "text.selected_color")?,
      self.text.font_family,
      self.text.font_size,
      self.text.font_weight,
      self.text.line_height,
    )?;
    let icon = IconConfig::new(self.icon.size, self.icon.label_gap)?;
    let close_after_action = if self.behavior.close_after_action {
      CloseAfterAction::Close
    } else {
      CloseAfterAction::KeepOpen
    };

    MenuConfig::new(window, item, text, icon, close_after_action, item_count)
  }
}

/// The TOML window section and its built-in fallback values.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct WindowInput {
  background: Box<str>,
  height: u32,
  padding: u32,
  corner_radius: f32,
}

impl Default for WindowInput {
  fn default() -> Self {
    Self {
      background: "#212325FA".into(),
      height: 250,
      padding: 25,
      corner_radius: 9.0,
    }
  }
}

/// The TOML item section and its built-in fallback values.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct ItemInput {
  background: Box<str>,
  selected_background: Box<str>,
  width: u32,
  gap: u32,
  corner_radius: f32,
}

impl Default for ItemInput {
  fn default() -> Self {
    Self {
      background: "#EAECE0".into(),
      selected_background: "#7A7A7C".into(),
      width: 225,
      gap: 16,
      corner_radius: 4.0,
    }
  }
}

/// The TOML text section and its built-in fallback values.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct TextInput {
  color: Box<str>,
  selected_color: Box<str>,
  font_family: Box<str>,
  font_size: f32,
  font_weight: u16,
  line_height: f32,
}

impl Default for TextInput {
  fn default() -> Self {
    Self {
      color: "#7A7A7C".into(),
      selected_color: "#EAECE0".into(),
      font_family: "sans-serif".into(),
      font_size: 18.0,
      font_weight: 500,
      line_height: 34.0,
    }
  }
}

/// The TOML icon section and its built-in fallback values.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct IconInput {
  size: u32,
  label_gap: f32,
}

impl Default for IconInput {
  fn default() -> Self {
    Self {
      size: 100,
      label_gap: 36.0,
    }
  }
}

/// The TOML behavior section, including the existing boolean spelling.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct BehaviorInput {
  close_after_action: bool,
}

impl Default for BehaviorInput {
  fn default() -> Self {
    Self {
      close_after_action: true,
    }
  }
}
