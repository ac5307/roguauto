//! Valid menu settings and the rules for constructing them.

use std::num::NonZeroU32 as NZU32;

use crate::domain::frame_size::{BYTES_PER_PIXEL, FrameSize};

const ICON_MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// Identifies a setting and the rule its value failed.
#[derive(Debug, thiserror::Error)]
#[error("{setting}: {reason}")]
pub struct ConfigValidationError {
  setting: &'static str,
  reason: Box<str>,
}

impl ConfigValidationError {
  /// Associates the field or related fields with a diagnostic.
  pub(crate) fn new(
    setting: &'static str,
    reason: impl Into<Box<str>>,
  ) -> Self {
    Self {
      setting,
      reason: reason.into(),
    }
  }
}

/// The validated settings for a complete menu.
#[derive(Debug)]
pub struct MenuConfig {
  window: WindowConfig,
  item: ItemConfig,
  text: TextConfig,
  icon: IconConfig,
  close_after_action: CloseAfterAction,
  frame_size: FrameSize,
}

impl MenuConfig {
  /// Returns the window's validated visual settings.
  pub fn window(&self) -> &WindowConfig {
    &self.window
  }

  /// Returns the visual settings each menu item shares.
  pub fn item(&self) -> &ItemConfig {
    &self.item
  }

  /// Returns the label style.
  pub fn text(&self) -> &TextConfig {
    &self.text
  }

  /// Returns the icon size and spacing settings.
  pub fn icon(&self) -> &IconConfig {
    &self.icon
  }

  /// Returns how the menu responds to a successful power action.
  pub fn close_after_action(&self) -> CloseAfterAction {
    self.close_after_action
  }

  /// Returns the requested frame size derived from the configured menu.
  pub fn frame_size(&self) -> FrameSize {
    self.frame_size
  }

  /// Validates the frame and icon sizes implied by the settings.
  pub(crate) fn new(
    window: WindowConfig,
    item: ItemConfig,
    text: TextConfig,
    icon: IconConfig,
    close_after_action: CloseAfterAction,
    item_count: usize,
  ) -> Result<Self, ConfigValidationError> {
    let width = menu_width(item_count, &window, &item)?;
    let frame_size =
      FrameSize::new(width, window.height()).map_err(|error| {
        ConfigValidationError::new(
          "window.height, window.padding, item.width, and item.gap",
          error.to_string(),
        )
      })?;
    icon.validate_memory(item_count)?;

    Ok(Self {
      window,
      item,
      text,
      icon,
      close_after_action,
      frame_size,
    })
  }
}

/// Derives the width from the item widths, inter-item gaps, and both margins.
fn menu_width(
  item_count: usize,
  window: &WindowConfig,
  item: &ItemConfig,
) -> Result<u32, ConfigValidationError> {
  let invalid_width = || {
    ConfigValidationError::new(
      "window.padding, item.width, and item.gap",
      "the derived window width is too large",
    )
  };
  let item_count = u32::try_from(item_count).map_err(|_| invalid_width())?;
  let gap_count = item_count.checked_sub(1).ok_or_else(|| {
    ConfigValidationError::new(
      "item.width",
      "at least one menu item is required",
    )
  })?;
  let items = item
    .width()
    .checked_mul(item_count)
    .ok_or_else(invalid_width)?;
  let gaps = item
    .gap()
    .checked_mul(gap_count)
    .ok_or_else(invalid_width)?;
  let margins = window.padding().checked_mul(2).ok_or_else(invalid_width)?;

  items
    .checked_add(gaps)
    .and_then(|width| width.checked_add(margins))
    .ok_or_else(invalid_width)
}

/// A nonzero integer length measured in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelLength(NZU32);

impl PixelLength {
  /// Checks that a required dimension is at least one pixel.
  fn new(
    value: u32,
    setting: &'static str,
  ) -> Result<Self, ConfigValidationError> {
    NZU32::new(value).map(Self).ok_or_else(|| {
      ConfigValidationError::new(setting, "must be at least one pixel")
    })
  }

  /// Returns the length in pixels.
  fn get(self) -> u32 {
    self.0.get()
  }
}

/// A nonnegative integer gap or margin measured in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelSpacing(u32);

/// A finite, nonnegative pixel radius.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CornerRadius(f32);

impl CornerRadius {
  /// Checks that a corner radius is finite and nonnegative.
  fn new(
    value: f32,
    setting: &'static str,
  ) -> Result<Self, ConfigValidationError> {
    Ok(Self(check_pixel_metric(value, 0.0, setting)?))
  }
}

/// A font family name, with installed-font fallback resolved by the view.
#[derive(Debug, PartialEq, Eq)]
struct FontFamily(Box<str>);

impl FontFamily {
  /// Accepts a nonblank name and preserves the entered spelling.
  fn new(value: Box<str>) -> Result<Self, ConfigValidationError> {
    if value.trim().is_empty() {
      return Err(ConfigValidationError::new(
        "text.font_family",
        "must not be blank",
      ));
    }
    Ok(Self(value))
  }
}

/// A finite label font size of at least one pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FontSize(f32);

impl FontSize {
  /// Validates a configured label font size.
  fn new(value: f32) -> Result<Self, ConfigValidationError> {
    Ok(Self(check_pixel_metric(value, 1.0, "text.font_size")?))
  }
}

/// The requested weight passed to installed-font matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FontWeight(u16);

/// A finite label line height at least as large as its font size.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LineHeight(f32);

impl LineHeight {
  /// Checks a label's line height against its validated font size.
  fn new(
    value: f32,
    font_size: FontSize,
  ) -> Result<Self, ConfigValidationError> {
    Ok(Self(check_pixel_metric(
      value,
      font_size.0,
      "text.line_height",
    )?))
  }
}

/// A finite, nonnegative gap between the icon and its label.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LabelGap(f32);

impl LabelGap {
  /// Validates the configured space between an icon and a label.
  fn new(value: f32) -> Result<Self, ConfigValidationError> {
    Ok(Self(check_pixel_metric(value, 0.0, "icon.label_gap")?))
  }
}

/// Checks a floating-point pixel measurement against its inclusive minimum.
fn check_pixel_metric(
  value: f32,
  minimum: f32,
  setting: &'static str,
) -> Result<f32, ConfigValidationError> {
  if !value.is_finite() {
    return Err(ConfigValidationError::new(setting, "must be finite"));
  }
  if value < minimum {
    return Err(ConfigValidationError::new(
      setting,
      format!("must be at least {minimum} pixels"),
    ));
  }
  Ok(value)
}

/// The validated window appearance and dimensions.
#[derive(Debug)]
pub struct WindowConfig {
  background: Color,
  height: PixelLength,
  padding: PixelSpacing,
  corner_radius: CornerRadius,
}

impl WindowConfig {
  /// Constructs window settings from a color and pixel measurements.
  pub(crate) fn new(
    background: Color,
    height: u32,
    padding: u32,
    corner_radius: f32,
  ) -> Result<Self, ConfigValidationError> {
    Ok(Self {
      background,
      height: PixelLength::new(height, "window.height")?,
      padding: PixelSpacing(padding),
      corner_radius: CornerRadius::new(
        corner_radius,
        "window.corner_radius",
      )?,
    })
  }

  /// Returns the window background color.
  pub fn background(&self) -> Color {
    self.background
  }

  /// Returns the requested window height in pixels.
  pub fn height(&self) -> u32 {
    self.height.get()
  }

  /// Returns the space between the window edge and its content in pixels.
  pub fn padding(&self) -> u32 {
    self.padding.0
  }

  /// Returns the requested corner radius in pixels.
  pub fn corner_radius(&self) -> f32 {
    self.corner_radius.0
  }
}

/// The validated appearance and dimensions shared by menu items.
#[derive(Debug)]
pub struct ItemConfig {
  background: Color,
  selected_background: Color,
  width: PixelLength,
  gap: PixelSpacing,
  corner_radius: CornerRadius,
}

impl ItemConfig {
  /// Constructs the dimensions and appearance shared by menu items.
  pub(crate) fn new(
    background: Color,
    selected_background: Color,
    width: u32,
    gap: u32,
    corner_radius: f32,
  ) -> Result<Self, ConfigValidationError> {
    Ok(Self {
      background,
      selected_background,
      width: PixelLength::new(width, "item.width")?,
      gap: PixelSpacing(gap),
      corner_radius: CornerRadius::new(corner_radius, "item.corner_radius")?,
    })
  }

  /// Returns the background color of an unselected item.
  pub fn background(&self) -> Color {
    self.background
  }

  /// Returns the background color of the selected item.
  pub fn selected_background(&self) -> Color {
    self.selected_background
  }

  /// Returns the width of each item in pixels.
  pub fn width(&self) -> u32 {
    self.width.get()
  }

  /// Returns the space between adjacent items in pixels.
  pub fn gap(&self) -> u32 {
    self.gap.0
  }

  /// Returns the requested corner radius of an item in pixels.
  pub fn corner_radius(&self) -> f32 {
    self.corner_radius.0
  }
}

/// The validated typography and foreground colors.
#[derive(Debug)]
pub struct TextConfig {
  color: Color,
  selected_color: Color,
  font_family: FontFamily,
  font_size: FontSize,
  font_weight: FontWeight,
  line_height: LineHeight,
}

impl TextConfig {
  /// Constructs label settings and checks that each line can hold its font size.
  pub(crate) fn new(
    color: Color,
    selected_color: Color,
    font_family: Box<str>,
    font_size: f32,
    font_weight: u16,
    line_height: f32,
  ) -> Result<Self, ConfigValidationError> {
    let font_size = FontSize::new(font_size)?;

    Ok(Self {
      color,
      selected_color,
      font_family: FontFamily::new(font_family)?,
      font_size,
      font_weight: FontWeight(font_weight),
      line_height: LineHeight::new(line_height, font_size)?,
    })
  }

  /// Returns the foreground color of an unselected item.
  pub fn color(&self) -> Color {
    self.color
  }

  /// Returns the foreground color of the selected item.
  pub fn selected_color(&self) -> Color {
    self.selected_color
  }

  /// Returns the configured font family.
  pub fn font_family(&self) -> &str {
    &self.font_family.0
  }

  /// Returns the font size in pixels.
  pub fn font_size(&self) -> f32 {
    self.font_size.0
  }

  /// Returns the requested font weight.
  pub fn font_weight(&self) -> u16 {
    self.font_weight.0
  }

  /// Returns the label line height in pixels.
  pub fn line_height(&self) -> f32 {
    self.line_height.0
  }
}

/// The validated icon size and space between the icon and the label.
#[derive(Debug)]
pub struct IconConfig {
  size: PixelLength,
  label_gap: LabelGap,
}

impl IconConfig {
  /// Constructs icon settings from a source size and label gap.
  pub(crate) fn new(
    size: u32,
    label_gap: f32,
  ) -> Result<Self, ConfigValidationError> {
    Ok(Self {
      size: PixelLength::new(size, "icon.size")?,
      label_gap: LabelGap::new(label_gap)?,
    })
  }

  /// Returns the requested icon width and height in pixels.
  pub fn size(&self) -> u32 {
    self.size.get()
  }

  /// Returns the space between the icon and label in pixels.
  pub fn label_gap(&self) -> f32 {
    self.label_gap.0
  }

  /// Bounds both tinted variants in each of two caches and one temporary mask.
  fn validate_memory(
    &self,
    item_count: usize,
  ) -> Result<(), ConfigValidationError> {
    let layout = || {
      let pixels_per_side = usize::try_from(self.size()).ok()?;
      let image_bytes = pixels_per_side
        .checked_mul(pixels_per_side)?
        .checked_mul(BYTES_PER_PIXEL)?;
      let image_count = item_count.checked_mul(4)?.checked_add(1)?;

      image_bytes.checked_mul(image_count)
    };

    if layout().is_some_and(|bytes| bytes <= ICON_MEMORY_LIMIT) {
      return Ok(());
    }

    Err(ConfigValidationError::new(
      "icon.size",
      "the active and replacement icon caches and temporary mask must fit within 64 MiB",
    ))
  }
}

/// Determines whether the menu stays open after a successful power action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAfterAction {
  /// Closes the menu after success.
  Close,
  /// Leaves the menu open after success.
  KeepOpen,
}

/// A non-premultiplied RGBA color with one byte per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color([u8; 4]);

impl Color {
  /// Constructs a color from its red, green, blue, and alpha channels.
  pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
    Self([red, green, blue, alpha])
  }

  /// Returns the red channel.
  pub const fn red(self) -> u8 {
    self.0[0]
  }

  /// Returns the green channel.
  pub const fn green(self) -> u8 {
    self.0[1]
  }

  /// Returns the blue channel.
  pub const fn blue(self) -> u8 {
    self.0[2]
  }

  /// Returns the alpha channel.
  pub const fn alpha(self) -> u8 {
    self.0[3]
  }
}
