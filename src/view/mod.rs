//! Menu tiles, their shared layout, and software drawing.

mod drawing;
mod labels;
mod raster_icon;

use std::{array, collections::TryReserveError};

use crate::{
  domain::config::menu_config::{Color, MenuConfig},
  domain::frame_size::BYTES_PER_PIXEL,
  domain::PowerAction,
  view::{
    drawing::{DrawingTarget, RgbaImage, TinySkiaCanvas},
    labels::{LabelRun, TextPainter},
    raster_icon::{
      ICON_VIEWPORT_HEIGHT, LOCK_ICON_VIEWPORT_WIDTH, IconSource,
    },
  },
};

/// Number of tiles in the model's action order.
const TILE_COUNT: usize = PowerAction::ALL.len();

/// Geometry in logical surface coordinates.
#[derive(Clone, Copy)]
struct ItemRect {
  x: f32,
  y: f32,
  width: f32,
  height: f32,
}

impl ItemRect {
  /// Returns the lower edge of the rectangle.
  fn bottom(self) -> f32 {
    self.y + self.height
  }

  /// Checks the rectangle's half-open bounds.
  fn contains(self, x: f64, y: f64) -> bool {
    let x = x as f32;
    let y = y as f32;
    x >= self.x
      && x < self.x + self.width
      && y >= self.y
      && y < self.bottom()
  }
}

/// Destination of an icon in a tile.
struct IconRect {
  x: f32,
  y: f32,
  size: f32,
}

impl IconRect {
  /// Fits the icon above its label and centers it horizontally.
  fn calculate(
    item: ItemRect,
    line_height: f32,
    config: &MenuConfig,
  ) -> Option<Self> {
    let label_gap = config.icon().label_gap();
    let label_top = item.bottom() - line_height;
    let available_height = label_top - label_gap - item.y;
    if available_height <= 0.0 {
      return None;
    }

    let configured_size = config.icon().size() as f32;
    let size = configured_size
      .min(item.width)
      .min(available_height)
      .max(1.0);
    Some(Self {
      x: item.x + (item.width - size) * 0.5,
      y: label_top - label_gap - size,
      size,
    })
  }
}

/// Positions the background, icon, and label of one tile.
struct TileLayout {
  bounds: ItemRect,
  icon: Option<IconRect>,
  label: ItemRect,
}

impl TileLayout {
  /// Places a tile in the horizontal row for the current surface height.
  fn new(
    index: usize,
    surface_height: u32,
    line_height: f32,
    config: &MenuConfig,
  ) -> Self {
    let padding = config.window().padding() as f32;
    let width = config.item().width() as f32;
    let gap = config.item().gap() as f32;
    let bounds = ItemRect {
      x: padding + index as f32 * (width + gap),
      y: padding,
      width,
      height: (surface_height as f32 - padding * 2.0).max(1.0),
    };
    let label = ItemRect {
      x: bounds.x,
      y: (bounds.bottom() - line_height).max(bounds.y),
      width: bounds.width,
      height: line_height,
    };
    Self {
      bounds,
      icon: IconRect::calculate(bounds, line_height, config),
      label,
    }
  }
}

/// Keeps one set of tile positions for painting and pointer hit testing.
struct MenuLayout {
  width: u32,
  height: u32,
  tiles: [TileLayout; TILE_COUNT],
}

impl MenuLayout {
  /// Calculates geometry from the active settings and known surface size.
  fn new(config: &MenuConfig, width: u32, height: u32) -> Self {
    let line_height = config.text().line_height();
    Self {
      width,
      height,
      tiles: array::from_fn(|index| {
        TileLayout::new(index, height, line_height, config)
      }),
    }
  }

  /// Returns the bounds of the full drawable surface.
  fn bounds(&self) -> ItemRect {
    ItemRect {
      x: 0.0,
      y: 0.0,
      width: self.width as f32,
      height: self.height as f32,
    }
  }

  /// Checks logical coordinates against the actual surface dimensions.
  fn contains(&self, x: f64, y: f64) -> bool {
    x.is_finite()
      && y.is_finite()
      && x >= 0.0
      && y >= 0.0
      && x < self.width as f64
      && y < self.height as f64
  }
}

/// Settings that determine the rendered icon pixels.
#[derive(Clone, Copy, PartialEq, Eq)]
struct IconStyle {
  size: u32,
  normal: Color,
  selected: Color,
}

impl IconStyle {
  /// Collects the size and both foreground colors from the active settings.
  fn from_config(config: &MenuConfig) -> Self {
    Self {
      size: config.icon().size(),
      normal: config.text().color(),
      selected: config.text().selected_color(),
    }
  }

  /// Produces both tinted variants from the same alpha mask.
  fn tint(self, source: &IconSource) -> Result<IconPixels, AllocationError> {
    let mask = source.mask(self.size)?;
    Ok(IconPixels {
      normal: tint_icon_mask(&mask, self.normal)?,
      selected: tint_icon_mask(&mask, self.selected)?,
    })
  }
}

/// The normal and selected pixels generated from one icon mask.
struct IconPixels {
  normal: RgbaImage,
  selected: RgbaImage,
}

/// The source artwork and cached pixels for one action.
struct TileIcon {
  source: IconSource,
  pixels: IconPixels,
}

impl TileIcon {
  /// Decodes a bundled icon and caches both foreground variants.
  fn new(
    png: &'static [u8],
    viewport_width: f32,
    style: IconStyle,
  ) -> Result<Self, AllocationError> {
    let source = IconSource::decode(png, viewport_width);
    let pixels = style.tint(&source)?;
    Ok(Self { source, pixels })
  }

  /// Draws a cached image at its current layout size.
  fn draw(
    &self,
    destination: &mut impl DrawingTarget,
    rect: &IconRect,
    appearance: TileAppearance,
  ) {
    let icon = match appearance {
      TileAppearance::Normal => &self.pixels.normal,
      TileAppearance::Selected => &self.pixels.selected,
    };
    destination.draw_image(icon, rect);
  }
}

/// Chooses the foreground and background of a tile.
#[derive(Clone, Copy)]
enum TileAppearance {
  Normal,
  Selected,
}

/// A menu tile owns its action, artwork, and single-line label buffer.
struct ActionTile {
  action: PowerAction,
  label: &'static str,
  text: LabelRun,
  icon: TileIcon,
}

impl ActionTile {
  /// Creates the tile and the drawing resources for this action.
  fn new(
    action: PowerAction,
    config: &MenuConfig,
    icon_style: IconStyle,
    text_painter: &mut TextPainter,
  ) -> Result<Self, AllocationError> {
    let (label, png): (&str, &[u8]) = match action {
      PowerAction::Lock => {
        ("Lock", include_bytes!("../../assets/icons/lock.png"))
      },
      PowerAction::Logout => {
        ("Log out", include_bytes!("../../assets/icons/logout.png"))
      },
      PowerAction::Suspend => {
        ("Suspend", include_bytes!("../../assets/icons/suspend.png"))
      },
      PowerAction::Hibernate => (
        "Hibernate",
        include_bytes!("../../assets/icons/hibernate.png"),
      ),
      PowerAction::Reboot => {
        ("Reboot", include_bytes!("../../assets/icons/reboot.png"))
      },
      PowerAction::Shutdown => (
        "Shut down",
        include_bytes!("../../assets/icons/shutdown.png"),
      ),
    };
    let text = text_painter.shape(label, config);
    let viewport_width = match action {
      PowerAction::Lock => LOCK_ICON_VIEWPORT_WIDTH,
      _ => ICON_VIEWPORT_HEIGHT,
    };
    let icon = TileIcon::new(png, viewport_width, icon_style)?;
    Ok(Self {
      action,
      label,
      text,
      icon,
    })
  }

  /// Reapplies the configured font settings to the tile's label.
  fn update_text(&mut self, config: &MenuConfig, painter: &mut TextPainter) {
    self.text = painter.shape(self.label, config);
  }

  /// Paints the tile's background, icon, and label in order.
  fn draw(
    &mut self,
    destination: &mut impl DrawingTarget,
    layout: &TileLayout,
    appearance: TileAppearance,
    config: &MenuConfig,
    text_painter: &mut TextPainter,
  ) {
    let (background, foreground) = match appearance {
      TileAppearance::Normal => {
        (config.item().background(), config.text().color())
      },
      TileAppearance::Selected => (
        config.item().selected_background(),
        config.text().selected_color(),
      ),
    };
    destination.fill_rounded_rect(
      layout.bounds,
      config.item().corner_radius(),
      background,
    );
    if let Some(rect) = &layout.icon {
      self.icon.draw(destination, rect, appearance);
    }
    text_painter.draw(
      destination,
      &self.text,
      layout.label,
      config,
      foreground,
    );
  }
}

/// A drawn menu frame with premultiplied RGBA pixels.
pub(crate) struct MenuFrame(RgbaImage);

impl MenuFrame {
  /// Borrows the frame pixels for presentation by the Wayland host.
  pub(crate) fn as_rgba_bytes(&self) -> &[u8] {
    self.0.as_rgba_bytes()
  }
}

/// A size or reservation failure while preparing menu drawing resources.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AllocationError {
  /// The drawing backend cannot use the requested dimensions.
  #[error("invalid pixmap dimensions {width}x{height}")]
  Dimensions { width: u32, height: u32 },

  /// There was insufficient capacity for the pixmap's pixels.
  #[error("failed to allocate pixels for a {width}x{height} pixmap")]
  Pixels {
    width: u32,
    height: u32,
    #[source]
    source: TryReserveError,
  },

  /// There was insufficient capacity for the menu's tile resources.
  #[error("failed to allocate menu tile resources")]
  TileResources(#[source] TryReserveError),
}

/// Owns the tiles and layout for one power menu.
pub(crate) struct MenuView {
  layout: MenuLayout,
  tiles: Box<[ActionTile]>,
  icon_style: IconStyle,
  text_painter: TextPainter,
}

impl MenuView {
  /// Creates a view for the supplied settings and initial surface dimensions.
  pub(crate) fn new(
    config: &MenuConfig,
    width: u32,
    height: u32,
  ) -> Result<Self, AllocationError> {
    let mut text_painter = TextPainter::new(config);
    let icon_style = IconStyle::from_config(config);
    let mut tiles = Vec::new();
    tiles
      .try_reserve_exact(TILE_COUNT)
      .map_err(AllocationError::TileResources)?;
    for action in PowerAction::ALL {
      tiles.push(ActionTile::new(
        action,
        config,
        icon_style,
        &mut text_painter,
      )?);
    }
    Ok(Self {
      layout: MenuLayout::new(config, width, height),
      tiles: tiles.into_boxed_slice(),
      icon_style,
      text_painter,
    })
  }

  /// Recalculates positions after the known surface dimensions change.
  pub(crate) fn update_layout(
    &mut self,
    config: &MenuConfig,
    width: u32,
    height: u32,
  ) {
    self.layout = MenuLayout::new(config, width, height);
  }

  /// Applies new settings after preparing all replacement icon pixels.
  ///
  /// A failed reservation leaves the active layout and tile resources intact.
  pub(crate) fn update_config(
    &mut self,
    config: &MenuConfig,
    width: u32,
    height: u32,
  ) -> Result<(), AllocationError> {
    let style = IconStyle::from_config(config);
    if self.icon_style != style {
      let mut replacement_pixels = Vec::new();
      replacement_pixels
        .try_reserve_exact(self.tiles.len())
        .map_err(AllocationError::TileResources)?;
      for tile in &self.tiles {
        replacement_pixels.push(style.tint(&tile.icon.source)?);
      }
      for (tile, pixels) in self.tiles.iter_mut().zip(replacement_pixels) {
        tile.icon.pixels = pixels;
      }
      self.icon_style = style;
    }
    self.update_layout(config, width, height);
    self.text_painter.update_config(config);
    for tile in &mut self.tiles {
      tile.update_text(config, &mut self.text_painter);
    }
    Ok(())
  }

  /// Returns the action at a pointer position inside the surface.
  pub(crate) fn hit_test(&self, (x, y): (f64, f64)) -> Option<PowerAction> {
    if !self.layout.contains(x, y) {
      return None;
    }
    self
      .tiles
      .iter()
      .zip(&self.layout.tiles)
      .find_map(|(tile, layout)| {
        layout.bounds.contains(x, y).then_some(tile.action)
      })
  }

  /// Draws the surface and its tiles, returning their premultiplied pixels.
  pub(crate) fn render(
    &mut self,
    selected: PowerAction,
    config: &MenuConfig,
  ) -> Result<MenuFrame, AllocationError> {
    let mut frame = RgbaImage::new(self.layout.width, self.layout.height)?;
    let mut canvas = TinySkiaCanvas::new(&mut frame)?;
    self.paint(&mut canvas, selected, config);
    Ok(MenuFrame(frame))
  }

  /// Composes the menu and its tiles through the drawing contract.
  fn paint(
    &mut self,
    destination: &mut impl DrawingTarget,
    selected: PowerAction,
    config: &MenuConfig,
  ) {
    destination.fill_rounded_rect(
      self.layout.bounds(),
      config.window().corner_radius(),
      config.window().background(),
    );
    for (tile, layout) in self.tiles.iter_mut().zip(&self.layout.tiles) {
      let appearance = if tile.action == selected {
        TileAppearance::Selected
      } else {
        TileAppearance::Normal
      };
      tile.draw(
        destination,
        layout,
        appearance,
        config,
        &mut self.text_painter,
      );
    }
  }
}

/// Tints the icon using the source alpha mask.
///
/// The image uses premultiplied RGBA, so RGB is multiplied by the resulting alpha.
fn tint_icon_mask(
  mask: &RgbaImage,
  color: Color,
) -> Result<RgbaImage, AllocationError> {
  let (width, height) = mask.dimensions();
  let mut image = RgbaImage::new(width, height)?;

  for (source, destination) in mask
    .as_rgba_bytes()
    .chunks_exact(BYTES_PER_PIXEL)
    .zip(image.as_rgba_bytes_mut().chunks_exact_mut(BYTES_PER_PIXEL))
  {
    let mask_alpha = source[3] as u16;

    let configured_alpha = color.alpha() as u16;

    let alpha = mask_alpha.saturating_mul(configured_alpha) / 255;

    destination[0] = (color.red() as u16 * alpha / 255) as u8;

    destination[1] = (color.green() as u16 * alpha / 255) as u8;

    destination[2] = (color.blue() as u16 * alpha / 255) as u8;

    destination[3] = alpha as u8;
  }

  Ok(image)
}
