// Copyright (C) 2026 ac5307
// SPDX-License-Identifier: GPL-3.0-or-later

use cosmic_text::{
  Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics,
  Shaping, SwashCache, Weight, Wrap,
};

use tiny_skia::{
  BYTES_PER_PIXEL, Color as SkiaColor, FillRule, FilterQuality, Paint, Path,
  PathBuilder, Pixmap, PixmapPaint, Rect, Transform,
};

use resvg::usvg;

use crate::{
  power::PowerAction,
  config::deserialize::{Color, Configuration},
};

/// Cubic Bézier approximation constant for one quarter of a circle.
const CIRCLE_KAPPA: f32 = 0.552_284_8;

/// Geometry for one rendered menu item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemRect {
  pub x: f32,
  pub y: f32,
  pub width: f32,
  pub height: f32,
}

impl ItemRect {
  #[inline]
  pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
    Self {
      x,
      y,
      width,
      height,
    }
  }

  /// Calculate one item’s geometry in the horizontal row.
  ///
  ///  This must be the shared geometry source used by both rendering and
  /// pointer hit testing. `window_height` is needed because the item height
  /// is derived from the independent window height.
  #[inline]
  pub fn calculate(
    index: usize,
    window_height: u32,
    config: &Configuration,
  ) -> Self {
    let padding = config.window.padding as f32;

    let item_width = config.item.width.max(1) as f32;

    let gap = config.item.gap as f32;

    let x = padding + index as f32 * (item_width + gap);

    let item_height = (window_height as f32 - padding * 2.0).max(1.0);

    Self::new(x, padding, item_width, item_height)
  }

  #[inline]
  pub fn right(self) -> f32 {
    self.x + self.width
  }

  #[inline]
  pub fn bottom(self) -> f32 {
    self.y + self.height
  }

  #[inline]
  pub fn contains(self, x: f64, y: f64) -> bool {
    let x = x as f32;
    let y = y as f32;

    x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
  }
}

struct IconRect {
  x: f32,
  y: f32,
  size: f32,
}

impl IconRect {
  /// Calculate the icon’s destination area.
  ///
  /// The icon is centered horizontally in the region
  /// above the bottom-aligned label.
  fn calculate(item: ItemRect, config: &Configuration) -> Option<Self> {
    let line_height =
      config.text.line_height.max(config.text.font_size.max(1.0));

    let label_gap = config.icon.label_gap.max(0.0);

    // The label occupies the bottom line-height of the item.
    // The icon is placed immediately above it, separated by exactly
    // label_gap pixels.
    let label_top = item.bottom() - line_height;

    let available_height = label_top - label_gap - item.y;

    if available_height <= 0.0 {
      return None;
    }

    let configured_size = config.icon.size.max(1) as f32;

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

struct IconTrees(Box<[usvg::Tree]>);

impl IconTrees {
  fn new() -> Self {
    let options = usvg::Options::default();

    let trees = PowerAction::ALL
      .iter()
      .map(|action| {
        usvg::Tree::from_data(action.icon(), &options).unwrap_or_else(
          |err| {
            panic!("failed to parse `{}` icon: {err}", action.label());
          },
        )
      })
      .collect();

    Self(trees)
  }

  fn get(&self, action: PowerAction) -> &usvg::Tree {
    &self.0[action as usize]
  }
}

/// Cached icons at the configured icon size and foreground colors.
///
/// Each icon is cached twice:
///
/// - normal foreground color;
/// - selected foreground color.
///
/// Ordinary redraws therefore need only composite an existing pixmap.
struct IconCache {
  /// Cached size of icon.
  size: u32,
  /// Cached normal color of icon.
  normal_color: Color,
  /// Cached selected color of icon.
  selected_color: Color,
  /// A vec containing elements of [normal, selected].
  pixels: Vec<[Pixmap; 2]>,
}

impl IconCache {
  fn new(trees: &IconTrees, config: &Configuration) -> Self {
    let mut cache = Self {
      size: 0,
      normal_color: config.text.color,
      selected_color: config.text.selected_color,
      pixels: Vec::with_capacity(PowerAction::ALL.len()),
    };

    cache.rebuild(trees, config);

    cache
  }

  fn update(&mut self, trees: &IconTrees, config: &Configuration) {
    let size = config.icon.size.max(1);

    let normal_color = config.text.color;

    let selected_color = config.text.selected_color;

    if self.size == size
      && self.normal_color == normal_color
      && self.selected_color == selected_color
    {
      return;
    }

    self.rebuild(trees, config);
  }

  fn rebuild(&mut self, trees: &IconTrees, config: &Configuration) {
    self.size = config.icon.size.max(1);

    self.normal_color = config.text.color;

    self.selected_color = config.text.selected_color;

    self.pixels.clear();

    let new_pixels = PowerAction::ALL.iter().map(|&action| {
      let tree = trees.get(action);

      let mask = render_icon_mask(tree, self.size);

      let normal = tint_icon_mask(&mask, self.normal_color);
      let selected = tint_icon_mask(&mask, self.selected_color);
      [normal, selected]
    });

    self.pixels.extend(new_pixels);
  }

  #[inline]
  fn get(&self, action: PowerAction, selected: bool) -> &Pixmap {
    let index = if selected { 1 } else { 0 };
    &self.pixels[action as usize][index]
  }
}

/// Software renderer for the power-menu surface.
///
/// `FontSystem` and `SwashCache` are retained across frames so font
/// discovery and glyph rasterization caches survive redraws.
pub struct Renderer {
  font_system: FontSystem,
  swash_cache: SwashCache,
  txt_buffers: Box<[Buffer]>,
  icon_trees: IconTrees,
  icon_cache: IconCache,
}

impl Renderer {
  /// Create a renderer.
  pub fn new(config: &Configuration) -> Self {
    let mut font_system = FontSystem::new();

    let swash_cache = SwashCache::new();

    let metrics = text_metrics(config);

    let txt_buffers = PowerAction::ALL
      .iter()
      .map(|action| {
        let mut buf = Buffer::new(&mut font_system, metrics);

        // Menu labels should remain on one line.
        // This also avoids unnecessary multiline layout if a label is
        // unexpectedly wider than the available space.
        buf.set_wrap(Wrap::None);

        set_buffer_text(&mut buf, action.label(), config);

        buf
      })
      .collect();

    let icon_trees = IconTrees::new();

    let icon_cache = IconCache::new(&icon_trees, config);

    Self {
      font_system,
      swash_cache,
      txt_buffers,
      icon_trees,
      icon_cache,
    }
  }

  /// Apply configuration-dependent cache changes.
  ///
  /// Icon pixmaps are regenerated only when their size or either
  /// foreground color changes. Text is replaced because family and
  /// weight may have changed.
  pub fn update_config(&mut self, config: &Configuration) {
    self.icon_cache.update(&self.icon_trees, config);

    for (buf, action) in self.txt_buffers.iter_mut().zip(PowerAction::ALL) {
      buf.set_wrap(Wrap::None);

      set_buffer_text(buf, action.label(), config);
    }
  }

  /// Render the complete menu into a premultiplied RGBA pixmap.
  ///
  /// Returns `None` when `width` or `height` cannot be represented by a
  /// `tiny-skia` pixmap.
  pub fn render(
    &mut self,
    width: u32,
    height: u32,
    selected: usize,
    _action_in_flight: bool,
    config: &Configuration,
  ) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;

    draw_rounded_rect(
      &mut pixmap,
      ItemRect::new(0.0, 0.0, width as f32, height as f32),
      config.window.corner_radius,
      config.window.background,
    );

    for (index, &action) in PowerAction::ALL.iter().enumerate() {
      let item = ItemRect::calculate(index, height, config);

      let is_selected = index == selected;

      let background = if is_selected {
        config.item.selected_background
      } else {
        config.item.background
      };

      let foreground = if is_selected {
        config.text.selected_color
      } else {
        config.text.color
      };

      draw_rounded_rect(
        &mut pixmap,
        item,
        config.item.corner_radius,
        background,
      );

      self.draw_icon(&mut pixmap, action, item, is_selected, config);

      self.draw_text(
        &mut pixmap,
        index,
        action.label(),
        item,
        foreground,
        config,
      );
    }

    Some(pixmap)
  }

  fn draw_icon(
    &self,
    destination: &mut Pixmap,
    action: PowerAction,
    item: ItemRect,
    selected: bool,
    config: &Configuration,
  ) {
    let Some(rect) = IconRect::calculate(item, config) else {
      return;
    };

    let icon = self.icon_cache.get(action, selected);

    let source_size = icon.width() as f32;

    if source_size <= 0.0 {
      return;
    }

    let scale = rect.size / source_size;

    if !scale.is_finite() || scale <= 0.0 {
      return;
    }

    let paint = PixmapPaint {
      quality: FilterQuality::Bicubic,
      ..Default::default()
    };

    destination.draw_pixmap(
      rect.x.round() as i32,
      rect.y.round() as i32,
      icon.as_ref(),
      &paint,
      Transform::from_scale(scale, scale),
      None,
    );
  }

  fn draw_text(
    &mut self,
    pixmap: &mut Pixmap,
    buf_idx: usize,
    _text: &str,
    item: ItemRect,
    color: Color,
    config: &Configuration,
  ) {
    let metrics = text_metrics(config);

    let text_width = item.width.max(1.0);

    let text_height = metrics.line_height.max(1.0);

    let text_x = item.x;

    let text_y = (item.bottom() - text_height).max(item.y);

    let default_color = TextColor::from(color);

    let Self {
      font_system,
      swash_cache,
      txt_buffers,
      ..
    } = self;

    let buf = &mut txt_buffers[buf_idx];

    buf.set_metrics_and_size(metrics, Some(text_width), Some(text_height));

    buf.shape_until_scroll(font_system, false);

    // Normally, labels don't change.
    //set_buffer_text(buf, text, config);

    buf.draw(
      font_system,
      swash_cache,
      default_color,
      |x, y, glyph_width, glyph_height, glyph_color| {
        blend_glyph_rect(
          pixmap,
          text_x + x as f32,
          text_y + y as f32,
          glyph_width,
          glyph_height,
          Color::rgba(
            glyph_color.r(),
            glyph_color.g(),
            glyph_color.b(),
            glyph_color.a(),
          ),
        );
      },
    );
  }
}

/// Returns the horizontal menu item under the supplied coordinates.
pub fn item_at(
  x: f64,
  y: f64,
  window_height: u32,
  config: &Configuration,
) -> Option<usize> {
  if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
    return None;
  }

  // Using `enumerate` uses the index of the list, instead of the index
  // when using `as usize` on an enum member.
  PowerAction::ALL.iter().enumerate().find_map(|(idx, _)| {
    ItemRect::calculate(idx, window_height, config)
      .contains(x, y)
      .then_some(idx)
  })
}

fn text_metrics(config: &Configuration) -> Metrics {
  let font_size = config.text.font_size.max(1.0);

  let line_height = config.text.line_height.max(font_size);

  Metrics::new(font_size, line_height)
}

fn set_buffer_text(buffer: &mut Buffer, text: &str, config: &Configuration) {
  let attrs = Attrs::new()
    .family(Family::Name(config.text.font_family.as_ref()))
    .weight(Weight(config.text.font_weight));

  buffer.set_text(text, &attrs, Shaping::Advanced, Some(Align::Center));
}

fn draw_rounded_rect(
  pixmap: &mut Pixmap,
  rect: ItemRect,
  radius: f32,
  color: Color,
) {
  if rect.width <= 0.0 || rect.height <= 0.0 {
    return;
  }

  let Some(rect) = Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
  else {
    return;
  };

  let path = rounded_rect_path(rect, radius);

  let mut paint = Paint {
    anti_alias: true,
    ..Paint::default()
  };

  paint.set_color(SkiaColor::from(color));

  pixmap.fill_path(
    &path,
    &paint,
    FillRule::Winding,
    Transform::identity(),
    None,
  );
}

fn rounded_rect_path(rect: Rect, radius: f32) -> Path {
  let left = rect.left();
  let top = rect.top();
  let right = rect.right();
  let bottom = rect.bottom();

  let radius = radius
    .max(0.0)
    .min(rect.width() * 0.5)
    .min(rect.height() * 0.5);

  if radius <= f32::EPSILON {
    return PathBuilder::from_rect(rect);
  }

  let control = radius * CIRCLE_KAPPA;

  let mut builder = PathBuilder::new();

  builder.move_to(left + radius, top);

  builder.line_to(right - radius, top);

  builder.cubic_to(
    right - radius + control,
    top,
    right,
    top + radius - control,
    right,
    top + radius,
  );

  builder.line_to(right, bottom - radius);

  builder.cubic_to(
    right,
    bottom - radius + control,
    right - radius + control,
    bottom,
    right - radius,
    bottom,
  );

  builder.line_to(left + radius, bottom);

  builder.cubic_to(
    left + radius - control,
    bottom,
    left,
    bottom - radius + control,
    left,
    bottom - radius,
  );

  builder.line_to(left, top + radius);

  builder.cubic_to(
    left,
    top + radius - control,
    left + radius - control,
    top,
    left + radius,
    top,
  );

  builder.close();

  // All generated coordinates are finite because `Rect` guarantees valid
  // finite geometry and the radius is clamped to the rectangle.
  builder
    .finish()
    .expect("rounded rectangle path must be valid")
}

/// Render an SVG into a square alpha mask.
///
/// The SVG’s aspect ratio is preserved and centered inside the square.
fn render_icon_mask(tree: &usvg::Tree, size: u32) -> Pixmap {
  let size = size.max(1);

  let mut pixmap = Pixmap::new(size, size)
    .expect("configured icon dimensions must be valid");

  let svg_size = tree.size();

  let svg_width = svg_size.width();

  let svg_height = svg_size.height();

  let scale = (size as f32 / svg_width).min(size as f32 / svg_height);

  let rendered_width = svg_width * scale;

  let rendered_height = svg_height * scale;

  let offset_x = (size as f32 - rendered_width) * 0.5;

  let offset_y = (size as f32 - rendered_height) * 0.5;

  let transform = Transform::from_translate(offset_x, offset_y);

  resvg::render(
    tree,
    transform.post_scale(scale, scale),
    &mut pixmap.as_mut(),
  );

  pixmap
}

/// Treat an SVG rendering as an alpha mask and tint it.
///
/// tiny-skia pixels are premultiplied RGBA, so RGB channels are multiplied
/// by the resulting alpha.
fn tint_icon_mask(mask: &Pixmap, color: Color) -> Pixmap {
  let mut pixmap = Pixmap::new(mask.width(), mask.height())
    .expect("mask dimensions must be valid");

  for (source, destination) in mask
    .data()
    .chunks_exact(BYTES_PER_PIXEL)
    .zip(pixmap.data_mut().chunks_exact_mut(BYTES_PER_PIXEL))
  {
    let mask_alpha = source[3] as u16;

    let configured_alpha = color.alpha as u16;

    let alpha = mask_alpha.saturating_mul(configured_alpha) / 255;

    destination[0] = (color.red as u16 * alpha / 255) as u8;

    destination[1] = (color.green as u16 * alpha / 255) as u8;

    destination[2] = (color.blue as u16 * alpha / 255) as u8;

    destination[3] = alpha as u8;
  }

  pixmap
}

fn blend_glyph_rect(
  pixmap: &mut Pixmap,
  x: f32,
  y: f32,
  width: u32,
  height: u32,
  color: Color,
) {
  if width == 0 || height == 0 || color.alpha == 0 {
    return;
  }

  let Some(rect) = Rect::from_xywh(x, y, width as f32, height as f32) else {
    return;
  };

  let mut paint = Paint {
    /*
     * The glyph rasterizer has already calculated pixel coverage in
     * the callback's alpha channel. Additional geometric antialiasing
     * is unnecessary for these pixel-aligned rectangles.
     */
    anti_alias: false,
    ..Paint::default()
  };

  paint.set_color(SkiaColor::from(color));

  pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

pub fn rgba_to_argb8888(src: &[u8], dest: &mut [u8]) {
  debug_assert_eq!(
    src.len(),
    dest.len(),
    "source and destination pixel-buffers must have equal lengths"
  );
  debug_assert_eq!(
    src.len() % 4,
    0,
    "pixel-buffer length must be divisible by four"
  );

  for (rgba, argb) in src
    .chunks_exact(BYTES_PER_PIXEL)
    .zip(dest.chunks_exact_mut(BYTES_PER_PIXEL))
  {
    let pixel = Color {
      red: rgba[0],
      green: rgba[1],
      blue: rgba[2],
      alpha: rgba[3],
    };

    // Construct the protocol's packed ARGB value and then store it
    // using the machine's native byte order.
    argb.copy_from_slice(&pixel.argb8888().to_ne_bytes());
  }
}

impl From<Color> for SkiaColor {
  #[inline]
  fn from(color: Color) -> Self {
    Self::from_rgba8(color.red, color.green, color.blue, color.alpha)
  }
}

impl From<Color> for TextColor {
  #[inline]
  fn from(color: Color) -> Self {
    Self::rgba(color.red, color.green, color.blue, color.alpha)
  }
}
