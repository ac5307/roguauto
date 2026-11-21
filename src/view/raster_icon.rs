//! Decodes bundled PNG artwork and scales it into an icon mask.

use tiny_skia::{
  FilterQuality, Paint, Pattern, Pixmap, PixmapMut, Rect, SpreadMode,
  Transform,
};

use crate::view::{AllocationError, drawing::RgbaImage};

/// Decoded artwork and the viewport dimensions used to place it.
pub(super) struct IconSource {
  pixels: Pixmap,
  viewport_width: f32,
}

/// Height shared by the artwork's original viewports.
pub(super) const ICON_VIEWPORT_HEIGHT: f32 = 546.1333;

/// Width of the narrower lock artwork in its original viewport.
pub(super) const LOCK_ICON_VIEWPORT_WIDTH: f32 = 477.86667;

impl IconSource {
  /// Decodes the bundled PNG into premultiplied pixels.
  pub(super) fn decode(png: &[u8], viewport_width: f32) -> Self {
    let pixels =
      Pixmap::decode_png(png).expect("bundled icon must be a valid PNG");
    Self {
      pixels,
      viewport_width,
    }
  }

  /// Samples the original viewport into a square mask for tinting.
  pub(super) fn mask(
    &self,
    size: u32,
  ) -> Result<RgbaImage, AllocationError> {
    let mut mask = RgbaImage::new(size, size)?;
    let fit = (size as f32 / self.viewport_width)
      .min(size as f32 / ICON_VIEWPORT_HEIGHT);
    let offset_x = (size as f32 - self.viewport_width * fit) * 0.5;
    let offset_y = (size as f32 - ICON_VIEWPORT_HEIGHT * fit) * 0.5;

    // Preserve the order of the original viewport and image transforms.
    let transform = Transform::from_translate(offset_x, offset_y)
      .post_scale(fit, fit)
      .pre_scale(
        self.viewport_width / self.pixels.width() as f32,
        ICON_VIEWPORT_HEIGHT / self.pixels.height() as f32,
      );
    let rect = Rect::from_xywh(
      0.0,
      0.0,
      self.pixels.width() as f32,
      self.pixels.height() as f32,
    )
    .expect("decoded PNG dimensions must describe a rectangle");
    let paint = Paint {
      shader: Pattern::new(
        self.pixels.as_ref(),
        SpreadMode::Pad,
        FilterQuality::Bicubic,
        1.0,
        Transform::identity(),
      ),
      ..Paint::default()
    };
    PixmapMut::from_bytes(mask.as_rgba_bytes_mut(), size, size)
      .expect("checked mask dimensions and pixel storage must agree")
      .fill_rect(rect, &paint, transform, None);
    Ok(mask)
  }
}
