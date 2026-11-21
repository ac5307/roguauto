//! Checked dimensions and byte lengths for a premultiplied RGBA frame.

/// Size of a pixel with four eight-bit channels.
pub(crate) const BYTES_PER_PIXEL: usize = size_of::<u32>();

/// Maximum pixel storage for one frame.
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Explains why frame dimensions cannot be used.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FrameSizeError {
  #[error("frame dimensions must be nonzero")]
  Empty,
  #[error("frame dimensions or stride exceed the supported range")]
  OutOfRange,
  #[error("frame exceeds the 64 MiB pixel limit")]
  TooLarge,
}

/// Dimensions checked for frame allocation and presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameSize {
  width: i32,
  height: i32,
  stride: i32,
  byte_len: usize,
}

impl FrameSize {
  /// Checks dimensions, stride, and the total allocation without creating pixels.
  pub(crate) fn new(
    width: u32,
    height: u32,
  ) -> Result<Self, FrameSizeError> {
    if width == 0 || height == 0 {
      return Err(FrameSizeError::Empty);
    }
    let width =
      i32::try_from(width).map_err(|_| FrameSizeError::OutOfRange)?;
    let height =
      i32::try_from(height).map_err(|_| FrameSizeError::OutOfRange)?;
    let stride = width
      .checked_mul(BYTES_PER_PIXEL as i32)
      .ok_or(FrameSizeError::OutOfRange)?;
    let byte_len = (stride as usize)
      .checked_mul(height as usize)
      .ok_or(FrameSizeError::TooLarge)?;
    if byte_len > MAX_FRAME_BYTES {
      return Err(FrameSizeError::TooLarge);
    }
    Ok(Self {
      width,
      height,
      stride,
      byte_len,
    })
  }

  /// Returns the logical width and height in pixels.
  pub(crate) fn dimensions(self) -> (u32, u32) {
    (self.width as u32, self.height as u32)
  }

  /// Returns the signed width, height, and row stride.
  pub(crate) fn buffer_shape(self) -> (i32, i32, i32) {
    (self.width, self.height, self.stride)
  }

  /// Returns the unpadded pixel byte count.
  pub(crate) fn byte_len(self) -> usize {
    self.byte_len
  }
}
