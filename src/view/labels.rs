//! Single-line label shaping and rasterization for the power menu.
//!
//! The menu has six fixed English labels. The layout uses one left-to-right
//! line and retains glyph masks across redraws for the active font settings.

use fontdb::{Database, FaceInfo, Family, ID, Query, Stretch, Style, Weight};
use swash::{
  CacheKey, FontRef, GlyphId, Tag,
  scale::{
    Render, ScaleContext, Source, StrikeWith,
    image::{Content, Image},
  },
  shape::ShapeContext,
  text::Script,
  zeno::{Format, Vector},
};

use std::collections::{BTreeMap, BTreeSet};

use crate::{
  domain::config::menu_config::{Color, MenuConfig},
  view::{drawing::DrawingTarget, ItemRect},
};

const WEIGHT: Tag = Tag::from_be_bytes(*b"wght");
const FALLBACK_FAMILIES: [&str; 9] = [
  "Noto Sans",
  "DejaVu Sans",
  "FreeSans",
  "Noto Sans Mono",
  "DejaVu Sans Mono",
  "FreeMono",
  "Noto Sans Symbols",
  "Noto Sans Symbols2",
  "Noto Color Emoji",
];

/// Owns one font face and keeps its cache key stable across label draws.
struct Face {
  bytes: Box<[u8]>,
  offset: u32,
  key: CacheKey,
}

impl Face {
  /// Loads an owned copy of the selected face without retaining a file mapping.
  fn load(database: &Database, id: ID) -> Option<Self> {
    database.with_face_data(id, |data, index| {
      let face = FontRef::from_index(data, index as usize)?;
      Some(Self {
        bytes: data.into(),
        offset: face.offset,
        key: face.key,
      })
    })?
  }

  /// Borrows this face and its stable key for shaping or rasterization.
  fn font(&self) -> FontRef<'_> {
    FontRef {
      data: &self.bytes,
      offset: self.offset,
      key: self.key,
    }
  }
}

/// A shaped glyph and its position relative to the start of the label.
struct Glyph {
  face: ID,
  id: GlyphId,
  x: f32,
  y: f32,
}

/// The shaped glyphs and measurements for one single-line label.
pub(in crate::view) struct LabelRun {
  glyphs: Box<[Glyph]>,
  width: f32,
  ascent: f32,
  descent: f32,
}

/// Identifies one rasterization of a glyph, including its fractional offset.
type RasterKey = (ID, GlyphId, u32, u16, u8);

/// Font settings that determine cached label resources.
struct LabelStyle {
  family: Box<str>,
  weight: u16,
  size: f32,
  line_height: f32,
}

impl LabelStyle {
  /// Records the settings used to shape and place the current labels.
  fn from_config(config: &MenuConfig) -> Self {
    let text = config.text();
    Self {
      family: text.font_family().into(),
      weight: text.font_weight(),
      size: text.font_size(),
      line_height: text.line_height(),
    }
  }

  /// Checks whether new settings use the current label font and metrics.
  fn matches(&self, config: &MenuConfig) -> bool {
    let text = config.text();
    self.family.as_ref() == text.font_family()
      && self.weight == text.font_weight()
      && self.size == text.font_size()
      && self.line_height == text.line_height()
  }
}

/// Discovers installed fonts and shares shaping and glyph caches across labels.
pub(in crate::view) struct TextPainter {
  style: LabelStyle,
  database: Database,
  face_ids: Box<[ID]>,
  faces: BTreeMap<ID, Face>,
  images: BTreeMap<RasterKey, Option<Image>>,
  shaping: ShapeContext,
  scaling: ScaleContext,
}

impl TextPainter {
  /// Loads the system font database for the initial label settings.
  pub(in crate::view) fn new(config: &MenuConfig) -> Self {
    let mut database = Database::new();
    database.load_system_fonts();
    let style = LabelStyle::from_config(config);
    let face_ids = ordered_faces(&database, &style);
    Self {
      style,
      database,
      face_ids,
      faces: BTreeMap::new(),
      images: BTreeMap::new(),
      shaping: ShapeContext::new(),
      scaling: ScaleContext::new(),
    }
  }

  /// Applies new font settings and discards caches tied to the previous style.
  pub(in crate::view) fn update_config(&mut self, config: &MenuConfig) {
    if self.style.matches(config) {
      return;
    }
    self.style = LabelStyle::from_config(config);
    self.face_ids = ordered_faces(&self.database, &self.style);
    self.faces.clear();
    self.images.clear();
    self.shaping = ShapeContext::new();
    self.scaling = ScaleContext::new();
  }

  /// Resolves missing glyphs through fallback faces and shapes adjacent runs.
  pub(in crate::view) fn shape(
    &mut self,
    label: &str,
    config: &MenuConfig,
  ) -> LabelRun {
    let mut runs = Vec::<(ID, std::ops::Range<usize>)>::new();
    for (offset, ch) in label.char_indices() {
      let Some(id) = self
        .face_ids
        .iter()
        .copied()
        .find(|id| {
          if let std::collections::btree_map::Entry::Vacant(entry) =
            self.faces.entry(*id)
            && let Some(face) = Face::load(&self.database, *id)
          {
            entry.insert(face);
          }
          self
            .faces
            .get(id)
            .is_some_and(|face| face.font().charmap().map(ch) != 0)
        })
        .or_else(|| self.face_ids.first().copied())
      else {
        continue;
      };
      let end = offset + ch.len_utf8();
      if let Some((previous, range)) = runs.last_mut()
        && *previous == id
        && range.end == offset
      {
        range.end = end;
      } else {
        runs.push((id, offset..end));
      }
    }

    let mut pen = 0.0;
    let mut ascent = 0.0_f32;
    let mut descent = 0.0_f32;
    let mut glyphs = Vec::new();
    for (id, range) in runs {
      let Some(face) = self.faces.get(&id) else {
        continue;
      };
      let font = face.font();
      let mut shaper = self
        .shaping
        .builder(font)
        .script(Script::Latin)
        .size(config.text().font_size())
        .normalized_coords(font.variations().normalized_coords([(
          WEIGHT,
          f32::from(config.text().font_weight()),
        )]))
        .build();
      shaper.add_str(&label[range]);
      let metrics = shaper.metrics();
      ascent = ascent.max(metrics.ascent);
      descent = descent.max(metrics.descent);
      shaper.shape_with(|cluster| {
        for glyph in cluster.glyphs {
          glyphs.push(Glyph {
            face: id,
            id: glyph.id,
            x: pen + glyph.x,
            y: glyph.y,
          });
          pen += glyph.advance;
        }
      });
    }
    LabelRun {
      glyphs: glyphs.into_boxed_slice(),
      width: pen,
      ascent,
      descent,
    }
  }

  /// Centers the label and paints cached glyphs with quarter-pixel offsets.
  pub(in crate::view) fn draw(
    &mut self,
    target: &mut impl DrawingTarget,
    label: &LabelRun,
    bounds: ItemRect,
    config: &MenuConfig,
    color: Color,
  ) {
    let center = (bounds.width - label.width).max(0.0) / 2.0;
    let baseline =
      (bounds.height - (label.ascent + label.descent)) / 2.0 + label.ascent;
    let size = config.text().font_size();
    let weight = config.text().font_weight();
    for glyph in &label.glyphs {
      let quantized_x = ((center + glyph.x) * 4.0).round() as i32;
      let whole_x = quantized_x.div_euclid(4);
      let bin = quantized_x.rem_euclid(4) as u8;
      let key = (glyph.face, glyph.id, size.to_bits(), weight, bin);
      let image = self.images.entry(key).or_insert_with(|| {
        let font = self.faces.get(&glyph.face)?.font();
        let mut scaler = self
          .scaling
          .builder(font)
          .size(size)
          .hint(true)
          .normalized_coords(
            font
              .variations()
              .normalized_coords([(WEIGHT, f32::from(weight))]),
          )
          .build();
        Render::new(&[
          Source::ColorOutline(0),
          Source::ColorBitmap(StrikeWith::BestFit),
          Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(Vector::new(f32::from(bin) * 0.25, 0.0))
        .render(&mut scaler, glyph.id)
      });
      let Some(image) = image else {
        continue;
      };
      let x = bounds.x + whole_x as f32 + image.placement.left as f32;
      let y =
        bounds.y + (baseline - glyph.y).trunc() - image.placement.top as f32;
      paint_image(target, image, x, y, color);
    }
  }
}

/// Orders installed faces so exact configured and common weights take priority.
///
/// If no such face can draw a character, the closest installed faces are
/// still available. The configured family's best CSS match leads that group.
fn ordered_faces(database: &Database, style: &LabelStyle) -> Box<[ID]> {
  let desired = style.weight;
  let family = style.family.as_ref();
  let mut faces = database.faces().collect::<Vec<_>>();
  faces.sort_unstable_by_key(|face| {
    let stretch = face.stretch.to_number();
    (
      !face.post_script_name.contains("Emoji"),
      desired.abs_diff(face.weight.0),
      Stretch::Normal.to_number().abs_diff(stretch),
      face.style != Style::Normal,
      face.weight.0,
      stretch,
      face.id,
    )
  });
  if let Some(id) = database.query(&Query {
    families: &[Family::Name(family)],
    weight: Weight(desired),
    ..Query::default()
  }) && let Some(index) = faces.iter().position(|face| face.id == id)
  {
    faces[..=index].rotate_right(1);
  }

  let matching_family = |face: &&FaceInfo, name| {
    face.families.iter().any(|(family, _)| family == name)
      && supports_weight(database, face, desired)
  };
  let mut ordered = Vec::new();
  if let Some(face) = faces.iter().find(|face| matching_family(face, family))
  {
    ordered.push(face.id);
  }
  for family in FALLBACK_FAMILIES {
    ordered.extend(
      faces
        .iter()
        .filter(|face| matching_family(face, family))
        .map(|face| face.id),
    );
  }
  ordered.extend(faces.iter().map(|face| face.id));
  let mut seen = BTreeSet::new();
  ordered.retain(|id| seen.insert(*id));
  ordered.into_boxed_slice()
}

/// Checks whether the face has the requested static or variable weight.
fn supports_weight(
  database: &Database,
  face: &FaceInfo,
  desired: u16,
) -> bool {
  face.weight.0 == desired
    || database
      .with_face_data(face.id, |data, index| {
        let Some(font) = FontRef::from_index(data, index as usize) else {
          return false;
        };
        font.variations().find_by_tag(WEIGHT).is_some_and(|axis| {
          (axis.min_value()..=axis.max_value()).contains(&f32::from(desired))
        })
      })
      .unwrap_or(false)
}

/// Draws an alpha mask in the label color or a glyph's own color pixels.
fn paint_image(
  target: &mut impl DrawingTarget,
  image: &Image,
  x: f32,
  y: f32,
  color: Color,
) {
  let width = image.placement.width as usize;
  if width == 0 {
    return;
  }
  match image.content {
    Content::Mask => {
      for (row, pixels) in image.data.chunks_exact(width).enumerate() {
        for (column, &alpha) in pixels.iter().enumerate() {
          let color =
            Color::rgba(color.red(), color.green(), color.blue(), alpha);
          target.blend_glyph_rect(
            ItemRect {
              x: x + column as f32,
              y: y + row as f32,
              width: 1.0,
              height: 1.0,
            },
            color,
          );
        }
      }
    },
    Content::Color => {
      for (row, pixels) in image.data.chunks_exact(width * 4).enumerate() {
        for (column, pixel) in pixels.chunks_exact(4).enumerate() {
          let color = Color::rgba(pixel[0], pixel[1], pixel[2], pixel[3]);
          target.blend_glyph_rect(
            ItemRect {
              x: x + column as f32,
              y: y + row as f32,
              width: 1.0,
              height: 1.0,
            },
            color,
          );
        }
      }
    },
    Content::SubpixelMask => {},
  }
}
