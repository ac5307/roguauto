// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Deserializer};

use crate::{fs, Path};

#[derive(Debug, thiserror::Error)]
pub enum ConfigFileError {
  /// Error caused from reading the file.
  #[error("failed to read config `{path}`: {source}")]
  Read {
    path: Box<Path>,

    #[source]
    source: std::io::Error,
  },

  /// Error caused from pasrsing the file.
  #[error("failed to parse config `{path}`: {source}")]
  Parse {
    path: Box<Path>,

    #[source]
    source: toml::de::Error,
  },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Configuration {
  pub window: WindowConfig,
  pub item: ItemConfig,
  pub text: TextConfig,
  pub icon: IconConfig,
  pub behavior: BehaviorConfig,
}

impl Configuration {
  /// Attempts to load a configuration file using the given `path`.
  pub fn load(
    path: &(impl AsRef<Path> + ?Sized),
  ) -> Result<Self, ConfigFileError> {
    let path = path.as_ref();

    let contents =
      fs::read_to_string(path).map_err(|source| ConfigFileError::Read {
        path: Box::from(path),
        source,
      })?;

    toml::from_str(&contents).map_err(|source| ConfigFileError::Parse {
      path: Box::from(path),
      source,
    })
  }

  pub fn window_width(&self) -> u32 {
    let item_count = crate::power::PowerAction::ALL.len() as u32;

    let items_width = self.item.width.saturating_mul(item_count);

    let gaps = item_count.saturating_sub(1).saturating_mul(self.item.gap);

    self
      .window
      .padding
      .saturating_mul(2)
      .saturating_add(items_width)
      .saturating_add(gaps)
  }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
  pub background: Color,
  pub height: u32,
  pub padding: u32,
  pub corner_radius: f32,
}

impl Default for WindowConfig {
  fn default() -> Self {
    Self {
      background: Color::rgba(0x21, 0x23, 0x25, 0xFA),
      height: 250,
      padding: 25,
      corner_radius: 9.0,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ItemConfig {
  pub background: Color,
  pub selected_background: Color,
  pub width: u32,
  pub gap: u32,
  pub corner_radius: f32,
}

impl Default for ItemConfig {
  fn default() -> Self {
    Self {
      background: Color::rgba(0xEA, 0xEC, 0xE0, 0xFF),
      selected_background: Color::rgba(0x7A, 0x7A, 0x7C, 0xFF),
      width: 225,
      gap: 16,
      corner_radius: 4.0,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct TextConfig {
  pub color: Color,
  pub selected_color: Color,
  pub font_family: Box<str>,
  pub font_size: f32,
  pub font_weight: u16,
  pub line_height: f32,
}

impl Default for TextConfig {
  fn default() -> Self {
    Self {
      color: Color::rgba(0x7A, 0x7A, 0x7C, 0xFF),
      selected_color: Color::rgba(0xEA, 0xEC, 0xE0, 0xFF),
      font_family: "sans-serif".into(),
      font_size: 18.0,
      font_weight: 500,
      line_height: 34.0,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct IconConfig {
  pub size: u32,
  /// Vertical distance between the icon and label.
  pub label_gap: f32,
}

impl Default for IconConfig {
  fn default() -> Self {
    Self {
      size: 100,
      label_gap: 36.0,
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
  pub close_after_action: bool,
}

impl Default for BehaviorConfig {
  fn default() -> Self {
    Self {
      close_after_action: true,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
  pub red: u8,
  pub green: u8,
  pub blue: u8,
  pub alpha: u8,
}

impl Color {
  pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
    Self {
      red,
      blue,
      green,
      alpha,
    }
  }

  pub const fn argb8888(self) -> u32 {
    ((self.alpha as u32) << 24)
      | ((self.red as u32) << 16)
      | ((self.green as u32) << 8)
      | self.blue as u32
  }
}

impl<'de> Deserialize<'de> for Color {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let value = String::deserialize(deserializer)?;
    parse_color(&value).map_err(serde::de::Error::custom)
  }
}

fn parse_color(value: &str) -> Result<Color, String> {
  let code = match value.strip_prefix('#') {
    Some(hex) => hex,
    _ => return Err("color must start with '#'".to_string()),
  };

  let parse_byte = |offset: usize| {
    let hex = &code[offset..offset + 2];
    match u8::from_str_radix(hex, 16) {
      Ok(byte) => Ok(byte),
      _ => Err(format!("invalid color: {value}")),
    }
  };

  match code.len() {
    6 => Ok(Color::rgba(
      parse_byte(0)?,
      parse_byte(2)?,
      parse_byte(4)?,
      0xFF,
    )),
    8 => Ok(Color::rgba(
      parse_byte(0)?,
      parse_byte(2)?,
      parse_byte(4)?,
      parse_byte(6)?,
    )),
    _ => Err(format!("color must use #RRGGBB or #RRGGBBAA: {value}")),
  }
}
