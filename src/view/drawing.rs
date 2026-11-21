//! The menu's drawing contract and its Tiny Skia adapter.

use tiny_skia::{
  Color as SkiaColor, FillRule, FilterQuality, Paint, Path, PathBuilder,
  PixmapMut, PixmapPaint, PixmapRef, Rect, Transform,
};

use crate::{
  domain::frame_size::FrameSize,
  domain::config::menu_config::Color,
  view::{AllocationError, IconRect, ItemRect},
};

/// Cubic Bézier approximation constant for one quarter of a circle.
const CIRCLE_KAPPA: f32 = 0.552_284_8;

/// The shapes, glyphs, and images needed to paint the menu.
pub(in crate::view) trait DrawingTarget {
  /// Fills an antialiased rounded rectangle.
  fn fill_rounded_rect(
    &mut self,
    bounds: ItemRect,
    radius: f32,
    color: Color,
  );

  /// Blends a glyph rectangle using the coverage already included in its color.
  fn blend_glyph_rect(&mut self, bounds: ItemRect, color: Color);

  /// Composites a premultiplied image in a square destination.
  fn draw_image(&mut self, image: &RgbaImage, destination: &IconRect);
}

/// Owned premultiplied RGBA pixels in tightly packed rows.
pub(in crate::view) struct RgbaImage {
  size: FrameSize,
  pixels: Box<[u8]>,
}

impl RgbaImage {
  /// Allocates transparent pixel storage for the given dimensions.
  pub(in crate::view) fn new(
    width: u32,
    height: u32,
  ) -> Result<Self, AllocationError> {
    let size = FrameSize::new(width, height)
      .map_err(|_| AllocationError::Dimensions { width, height })?;
    let mut pixels = Vec::new();
    pixels
      .try_reserve_exact(size.byte_len())
      .map_err(|source| AllocationError::Pixels {
        width,
        height,
        source,
      })?;
    pixels.resize(size.byte_len(), 0);
    Ok(Self {
      size,
      pixels: pixels.into_boxed_slice(),
    })
  }

  /// Returns the image width and height in pixels.
  pub(in crate::view) fn dimensions(&self) -> (u32, u32) {
    self.size.dimensions()
  }

  /// Borrows the tightly packed premultiplied RGBA pixels.
  pub(in crate::view) fn as_rgba_bytes(&self) -> &[u8] {
    &self.pixels
  }

  /// Borrows pixel storage for a producer of premultiplied RGBA.
  pub(in crate::view) fn as_rgba_bytes_mut(&mut self) -> &mut [u8] {
    &mut self.pixels
  }
}

/// Adapts the menu operations to Tiny Skia over borrowed RGBA storage.
pub(in crate::view) struct TinySkiaCanvas<'frame>(PixmapMut<'frame>);

impl<'frame> TinySkiaCanvas<'frame> {
  /// Creates a canvas that paints directly into the frame.
  pub(in crate::view) fn new(
    frame: &'frame mut RgbaImage,
  ) -> Result<Self, AllocationError> {
    let (width, height) = frame.dimensions();
    PixmapMut::from_bytes(frame.as_rgba_bytes_mut(), width, height)
      .map(Self)
      .ok_or(AllocationError::Dimensions { width, height })
  }
}

impl DrawingTarget for TinySkiaCanvas<'_> {
  fn fill_rounded_rect(
    &mut self,
    bounds: ItemRect,
    radius: f32,
    color: Color,
  ) {
    draw_rounded_rect(&mut self.0, bounds, radius, color);
  }

  fn blend_glyph_rect(&mut self, bounds: ItemRect, color: Color) {
    blend_glyph_rect(&mut self.0, bounds, color);
  }

  fn draw_image(&mut self, image: &RgbaImage, destination: &IconRect) {
    let (width, height) = image.dimensions();
    let scale = destination.size / width as f32;
    if !scale.is_finite() || scale <= 0.0 {
      return;
    }
    let Some(source) =
      PixmapRef::from_bytes(image.as_rgba_bytes(), width, height)
    else {
      return;
    };
    let paint = PixmapPaint {
      quality: FilterQuality::Bicubic,
      ..Default::default()
    };
    self.0.draw_pixmap(
      destination.x.round() as i32,
      destination.y.round() as i32,
      source,
      &paint,
      Transform::from_scale(scale, scale),
      None,
    );
  }
}

/// Paints a filled rectangle with antialiased corners.
fn draw_rounded_rect(
  pixmap: &mut PixmapMut<'_>,
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

/// Builds a path with a radius that fits the rectangle.
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

/// Blends one rasterized glyph rectangle into the frame.
fn blend_glyph_rect(
  pixmap: &mut PixmapMut<'_>,
  bounds: ItemRect,
  color: Color,
) {
  if bounds.width == 0.0 || bounds.height == 0.0 || color.alpha() == 0 {
    return;
  }

  let Some(rect) =
    Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
  else {
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

impl From<Color> for SkiaColor {
  #[inline]
  fn from(color: Color) -> Self {
    Self::from_rgba8(color.red(), color.green(), color.blue(), color.alpha())
  }
}
