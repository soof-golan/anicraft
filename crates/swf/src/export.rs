//! Frame by frame export: one [`DisplayList`] per frame becomes one SWF frame.
//!
//! Every `Fill`, `Stroke` and `Bitmap` node becomes a `DefineShape4` whose coordinates are the node's own local
//! space; the node's transform goes into the `PlaceObject3` that places it, so a shape that only moves between
//! frames is defined once and moved with a small `PlaceObject3`. Groups become sprites (`DefineSprite`) with the
//! group's transform, colour transform, blend mode and filters on their placement; group masks become a `ClipDepth`
//! mask. Identical shapes and sprites are shared (see [`Movie`]), and between frames only the depths that changed
//! produce tags.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use anim_core::kurbo::{self, BezPath, PathEl, Point, Shape};
use anim_core::{Affine, BlendMode, Rgba8, num};
use anim_display::{Bitmap, DisplayList, Fill, Gradient, Group, ImageId, ImageStore, Mesh, Node, Paint, Stroke, StrokeScale, StrokeStyle};
use serde::{Deserialize, Serialize};

use crate::error::SwfError;
use crate::movie::{ACTION_STOP, Frame, FrameItem, Movie, MovieSettings};
use crate::place::{Placement, blend_code};
use crate::shape::{PathStyle, ShapeBuilder, ShapeDef, clean_path, fill_side_is_right};
use crate::style::{FillStyle, LinePaint, LineStyle};
use crate::types::{CharacterId, CxForm, Matrix};

/// How deeply nested groups the exporter follows before giving up.
pub const MAX_NESTING: usize = 64;

/// Dashed strokes needing more dashes than this are drawn solid.
const MAX_DASHES: f64 = 100_000.0;

/// Export settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub movie: MovieSettings,
    /// Loop forever (the SWF default). When false the last frame carries a `stop()` action.
    pub loop_playback: bool,
}

impl ExportOptions {
    /// Looping playback with the given movie settings.
    #[must_use]
    pub const fn new(movie: MovieSettings) -> Self {
        Self { movie, loop_playback: true }
    }

    /// Settings for a stage the size of `list`, with its background (white when the list has none).
    #[must_use]
    pub fn for_list(list: &DisplayList, frame_rate: f64) -> Self {
        Self::new(MovieSettings {
            width: list.size.width,
            height: list.size.height,
            frame_rate,
            background: list.background.unwrap_or(Rgba8::WHITE),
            ..MovieSettings::default()
        })
    }
}

/// Something the exporter could not carry over exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportNote {
    /// SWF has no such blend mode; the nearest one was used.
    BlendApproximated(BlendMode),
    /// A gradient had more than 15 stops; the least important ones were dropped.
    GradientStopsReduced,
    /// A dashed stroke would need an unreasonable number of dashes and was drawn solid.
    DashesDropped,
    /// A mesh was emulated with one masked, transformed copy of its content per triangle.
    MeshEmulated,
    /// A bitmap fill named a bitmap that was not available and was painted grey.
    MissingBitmap,
}

/// What an export produced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub frames: u32,
    /// Characters in the dictionary (shapes, bitmaps and sprites).
    pub characters: u32,
    pub bitmaps: u32,
    pub notes: Vec<ExportNote>,
}

/// The finished file and its report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOutput {
    pub swf: Vec<u8>,
    pub report: ExportReport,
}

/// One object on the stage at one depth.
#[derive(Debug, Clone, PartialEq)]
struct Slot {
    character: CharacterId,
    matrix: Matrix,
    color: CxForm,
    blend: BlendMode,
    filters: Vec<anim_display::Filter>,
    clip_depth: Option<u16>,
}

/// How a slot differs from what was at its depth before.
enum Diff {
    Same,
    Move(Placement),
    Replace,
}

impl Slot {
    const fn plain(character: CharacterId, matrix: Matrix) -> Self {
        Self { character, matrix, color: CxForm::IDENTITY, blend: BlendMode::Normal, filters: Vec::new(), clip_depth: None }
    }

    /// A fresh `PlaceObject3` carrying only what differs from the defaults.
    fn place_new(&self, depth: u16) -> Placement {
        Placement {
            depth,
            character: Some(self.character),
            matrix: Some(self.matrix),
            color: (!self.color.is_identity()).then_some(self.color),
            clip_depth: self.clip_depth,
            filters: (!self.filters.is_empty()).then(|| self.filters.clone()),
            blend: (blend_code(self.blend).0 != 1).then_some(self.blend),
            ..Placement::default()
        }
    }

    fn diff(&self, old: &Self, depth: u16) -> Diff {
        if self.character != old.character || self.clip_depth != old.clip_depth {
            return Diff::Replace;
        }
        // A move keeps whatever it does not mention, so only the properties that changed are written.
        let matrix = (self.matrix != old.matrix).then_some(self.matrix);
        let color = (self.color != old.color).then_some(self.color);
        let blend = (blend_code(self.blend).0 != blend_code(old.blend).0).then_some(self.blend);
        let filters = (self.filters != old.filters).then(|| self.filters.clone());
        if matrix.is_none() && color.is_none() && blend.is_none() && filters.is_none() {
            return Diff::Same;
        }
        Diff::Move(Placement { depth, is_move: true, matrix, color, blend, filters, ..Placement::default() })
    }
}

/// Whether a path painted with `paint` can show anything at all.
fn is_visible(path: &BezPath, paint: &Paint) -> bool {
    !path.elements().is_empty() && !matches!(paint, Paint::Solid(c) if c.a == 0)
}

/// The depth (1-based) of the object at `index` in a display list.
fn depth_of(index: usize) -> Result<u16, SwfError> {
    index.checked_add(1).and_then(|d| u16::try_from(d).ok()).ok_or(SwfError::DepthOutOfRange(index))
}

/// Turns display lists into a SWF, one frame at a time.
#[derive(Debug)]
pub struct DisplayListExporter {
    movie: Movie,
    loop_playback: bool,
    images: HashMap<ImageId, CharacterId>,
    /// Image ids that defined a bitmap, by a hash of their pixels.
    by_content: HashMap<u64, Vec<ImageId>>,
    bitmaps: u32,
    previous: Vec<Slot>,
    pending: Option<Frame>,
    notes: Vec<ExportNote>,
}

impl DisplayListExporter {
    /// Starts an export.
    ///
    /// # Errors
    /// Invalid movie settings, see [`Movie::new`].
    pub fn new(options: ExportOptions) -> Result<Self, SwfError> {
        Ok(Self {
            movie: Movie::new(options.movie)?,
            loop_playback: options.loop_playback,
            images: HashMap::new(),
            by_content: HashMap::new(),
            bitmaps: 0,
            previous: Vec::new(),
            pending: None,
            notes: Vec::new(),
        })
    }

    fn note(&mut self, note: ExportNote) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    /// Appends one frame.
    ///
    /// # Errors
    /// [`SwfError::MissingImage`] for a bitmap the store does not hold, [`SwfError::TooDeep`] for runaway nesting, and
    /// any limit of the format (65535 characters, frames or depths).
    pub fn push_frame(&mut self, list: &DisplayList, images: &ImageStore) -> Result<(), SwfError> {
        // Write the previous frame first, so the definitions this frame needs come after it.
        if let Some(frame) = self.pending.take() {
            self.movie.add_frame(&frame)?;
        }
        let slots = self.convert_nodes(&list.nodes, images, 0)?;
        let mut items = Vec::new();
        for index in 0..self.previous.len().max(slots.len()) {
            let depth = depth_of(index)?;
            match (self.previous.get(index), slots.get(index)) {
                (Some(old), Some(slot)) => match slot.diff(old, depth) {
                    Diff::Same => {}
                    Diff::Move(placement) => items.push(FrameItem::Place(placement)),
                    Diff::Replace => {
                        items.push(FrameItem::Remove(depth));
                        items.push(FrameItem::Place(slot.place_new(depth)));
                    }
                },
                (None, Some(slot)) => items.push(FrameItem::Place(slot.place_new(depth))),
                (Some(_), None) => items.push(FrameItem::Remove(depth)),
                (None, None) => {}
            }
        }
        self.previous = slots;
        self.pending = Some(Frame::new(items));
        Ok(())
    }

    /// Finishes the file.
    ///
    /// # Errors
    /// See [`Movie::finish`].
    pub fn finish(mut self) -> Result<ExportOutput, SwfError> {
        if let Some(mut frame) = self.pending.take() {
            if !self.loop_playback {
                frame.items.push(FrameItem::DoAction(ACTION_STOP.to_vec()));
            }
            self.movie.add_frame(&frame)?;
        }
        let report = ExportReport { frames: self.movie.frame_count(), characters: self.movie.character_count(), bitmaps: self.bitmaps, notes: self.notes };
        Ok(ExportOutput { swf: self.movie.finish()?, report })
    }

    fn convert_nodes(&mut self, nodes: &[Node], images: &ImageStore, nesting: usize) -> Result<Vec<Slot>, SwfError> {
        if nesting > MAX_NESTING {
            return Err(SwfError::TooDeep(MAX_NESTING));
        }
        let mut slots = Vec::with_capacity(nodes.len());
        let mut remaining = nodes.iter().peekable();
        while let Some(node) = remaining.next() {
            let slot = match node {
                // a fill immediately followed by a stroke of the same path is one SWF shape
                Node::Fill(fill) => match remaining.next_if(|next| matches!(next, Node::Stroke(s) if s.path == fill.path && s.transform == fill.transform)) {
                    Some(Node::Stroke(stroke)) => self.paint_slot(Some(fill), Some(stroke), images)?,
                    _ => self.paint_slot(Some(fill), None, images)?,
                },
                Node::Stroke(stroke) => self.paint_slot(None, Some(stroke), images)?,
                Node::Bitmap(bitmap) => self.bitmap_slot(bitmap, images)?,
                Node::Group(group) => self.group_slot(group, images, nesting)?,
                Node::Mesh(mesh) => self.mesh_slot(mesh, images, nesting)?,
            };
            slots.extend(slot);
        }
        Ok(slots)
    }

    /// The bitmap character for an image: one per image id, and images with identical pixels share a character.
    fn bitmap_character(&mut self, id: ImageId, images: &ImageStore) -> Result<CharacterId, SwfError> {
        if let Some(character) = self.images.get(&id) {
            return Ok(*character);
        }
        let image = images.get(id).ok_or(SwfError::MissingImage(id))?;
        let mut hasher = DefaultHasher::new();
        (image.width, image.height).hash(&mut hasher);
        image.premultiplied_rgba.hash(&mut hasher);
        let fingerprint = hasher.finish();
        let twin = self
            .by_content
            .get(&fingerprint)
            .into_iter()
            .flatten()
            .find(|other| images.get(**other) == Some(image))
            .and_then(|other| self.images.get(other))
            .copied();
        let character = if let Some(character) = twin {
            character
        } else {
            self.bitmaps = self.bitmaps.saturating_add(1);
            let character = self.movie.define_bitmap(image)?;
            self.by_content.entry(fingerprint).or_default().push(id);
            character
        };
        self.images.insert(id, character);
        Ok(character)
    }

    fn fill_style(&mut self, paint: &Paint, images: &ImageStore) -> Result<FillStyle, SwfError> {
        Ok(match paint {
            Paint::Solid(color) => FillStyle::Solid(*color),
            Paint::Linear(gradient) => {
                let (matrix, gradient) = self.gradient(gradient);
                FillStyle::Linear { matrix, gradient }
            }
            Paint::Radial(gradient) => {
                let focal = gradient.focal;
                let (matrix, swf_gradient) = self.gradient(gradient);
                if focal.abs() > 1e-4 {
                    FillStyle::Focal { matrix, gradient: swf_gradient, focal }
                } else {
                    FillStyle::Radial { matrix, gradient: swf_gradient }
                }
            }
            Paint::Bitmap { image, transform, repeat, smoothing } => {
                let id = self.bitmap_character(*image, images)?;
                FillStyle::Bitmap { id: id.0, matrix: Matrix::from_bitmap_affine(*transform), repeat: *repeat, smoothed: *smoothing }
            }
        })
    }

    fn gradient(&mut self, gradient: &Gradient) -> (Matrix, crate::style::Gradient) {
        let (swf, reduced) = crate::style::Gradient::from_stops(&gradient.stops, gradient.spread, gradient.interpolation);
        if reduced {
            self.note(ExportNote::GradientStopsReduced);
        }
        (Matrix::from_affine(gradient.transform), swf)
    }

    fn define_slot(&mut self, shape: &ShapeDef, transform: Affine) -> Result<Option<Slot>, SwfError> {
        let character = self.movie.define_shape(shape)?;
        Ok(Some(Slot::plain(character, Matrix::from_affine(transform))))
    }

    /// One shape holding a fill, a stroke, or a fill with the stroke drawn over it (SWF draws strokes over fills
    /// within a shape, which is exactly the order of a `Fill` node followed by a `Stroke` node).
    fn paint_slot(&mut self, fill: Option<&Fill>, stroke: Option<&Stroke>, images: &ImageStore) -> Result<Option<Slot>, SwfError> {
        let fill = fill.filter(|f| is_visible(&f.path, &f.paint));
        let stroke = stroke.filter(|s| is_visible(&s.path, &s.paint));
        let transform = match (fill, stroke) {
            (Some(f), _) => f.transform,
            (None, Some(s)) => s.transform,
            (None, None) => return Ok(None),
        };
        let mut builder = ShapeBuilder::new();
        if let Some(fill) = fill {
            builder.set_fill_rule(fill.rule);
            let index = builder.add_fill(self.fill_style(&fill.paint, images)?)?;
            let path = clean_path(&fill.path);
            let style =
                if fill_side_is_right(&path) { PathStyle { fill1: index, ..PathStyle::default() } } else { PathStyle { fill0: index, ..PathStyle::default() } };
            builder.add_path(&path, style);
        }
        if let Some(stroke) = stroke {
            let paint = match &stroke.paint {
                Paint::Solid(color) => LinePaint::Solid(*color),
                other => LinePaint::Fill(self.fill_style(other, images)?),
            };
            let index = builder.add_line(line_style(&stroke.style, paint))?;
            let path = clean_path(&stroke.path);
            let style = PathStyle { line: index, ..PathStyle::default() };
            match self.dashed(&path, &stroke.style) {
                Some(dashes) => builder.add_elements(&dashes, style),
                None => builder.add_path(&path, style),
            }
        }
        self.define_slot(&builder.finish(), transform)
    }

    /// SWF has no dashes; split the (cleaned) path into dash-long open pieces instead.
    fn dashed(&mut self, path: &BezPath, style: &StrokeStyle) -> Option<Vec<PathEl>> {
        let period: f64 = style.dashes.iter().sum();
        if style.dashes.is_empty() || !style.dashes.iter().all(|d| d.is_finite() && *d >= 0.0) || !(period.is_finite() && period > 0.0) {
            return None;
        }
        let length = path.perimeter(0.1);
        let dashes = length / period * num::usize_to_f64(style.dashes.len());
        if !length.is_finite() || dashes > MAX_DASHES {
            self.note(ExportNote::DashesDropped);
            return None;
        }
        // the dash iterator does not promise to start with a MoveTo, which `BezPath` insists on, so keep loose elements
        Some(kurbo::dash(path.iter(), 0.0, &style.dashes).collect())
    }

    fn bitmap_slot(&mut self, bitmap: &Bitmap, images: &ImageStore) -> Result<Option<Slot>, SwfError> {
        let character = self.bitmap_character(bitmap.image, images)?;
        let (width, height) = images.get(bitmap.image).map_or((0, 0), |i| (i.width, i.height));
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let rect = kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let mut builder = ShapeBuilder::new();
        builder.set_fill_rule(anim_display::FillRule::NonZero);
        let index = builder.add_fill(FillStyle::Bitmap {
            id: character.0,
            matrix: Matrix::from_bitmap_affine(Affine::IDENTITY),
            repeat: false,
            smoothed: bitmap.smoothing,
        })?;
        builder.add_path(&rect.to_path(0.1), PathStyle { fill1: index, ..PathStyle::default() });
        self.define_slot(&builder.finish(), bitmap.transform)
    }

    fn group_slot(&mut self, group: &Group, images: &ImageStore, nesting: usize) -> Result<Option<Slot>, SwfError> {
        let children = self.convert_nodes(&group.children, images, nesting.saturating_add(1))?;
        if children.is_empty() {
            return Ok(None);
        }
        let mask = match &group.mask {
            Some(nodes) => {
                let slots = self.convert_nodes(nodes, images, nesting.saturating_add(1))?;
                match slots.as_slice() {
                    // an empty mask hides everything
                    [] => return Ok(None),
                    [only] => Some(only.clone()),
                    many => Some(Slot::plain(self.sprite_of(many, None)?, Matrix::IDENTITY)),
                }
            }
            None => None,
        };
        let character = self.sprite_of(&children, mask)?;
        if !blend_code(group.blend).1 {
            self.note(ExportNote::BlendApproximated(group.blend));
        }
        Ok(Some(Slot {
            character,
            matrix: Matrix::from_affine(group.transform),
            color: CxForm::from_color_transform(&group.color),
            blend: group.blend,
            filters: group.filters.clone(),
            clip_depth: None,
        }))
    }

    /// A one-frame sprite showing `slots` bottom to top, with `mask` (if any) clipping all of them.
    fn sprite_of(&mut self, slots: &[Slot], mask: Option<Slot>) -> Result<CharacterId, SwfError> {
        let mut items = Vec::with_capacity(slots.len().saturating_add(1));
        let first = usize::from(mask.is_some());
        if let Some(mut mask) = mask {
            // the mask sits at depth 1 and clips everything up to the last child's depth
            mask.clip_depth = Some(depth_of(slots.len())?);
            items.push(FrameItem::Place(mask.place_new(1)));
        }
        for (index, slot) in slots.iter().enumerate() {
            items.push(FrameItem::Place(slot.place_new(depth_of(index.saturating_add(first))?)));
        }
        self.movie.define_sprite(&[Frame::new(items)])
    }

    /// A mesh warps its content triangle by triangle: each triangle is a sprite with the triangle as a clip mask
    /// over a copy of the content moved by the affine map that carries the source triangle onto the target one.
    fn mesh_slot(&mut self, mesh: &Mesh, images: &ImageStore, nesting: usize) -> Result<Option<Slot>, SwfError> {
        let content = self.convert_nodes(&mesh.content, images, nesting.saturating_add(1))?;
        if content.is_empty() {
            return Ok(None);
        }
        let content_sprite = self.sprite_of(&content, None)?;
        let mut triangles = Vec::new();
        for &[a, b, c] in &mesh.triangles {
            let corner = |points: &[Point], i: u32| usize::try_from(i).ok().and_then(|i| points.get(i).copied());
            let (Some(s0), Some(s1), Some(s2), Some(d0), Some(d1), Some(d2)) = (
                corner(&mesh.source, a),
                corner(&mesh.source, b),
                corner(&mesh.source, c),
                corner(&mesh.target, a),
                corner(&mesh.target, b),
                corner(&mesh.target, c),
            ) else {
                continue;
            };
            let Some(map) = triangle_map([s0, s1, s2], [d0, d1, d2]) else { continue };
            let mask = {
                let mut path = BezPath::new();
                let [e0, e1, e2] = grow_triangle([d0, d1, d2], 0.5);
                path.move_to(e0);
                path.line_to(e1);
                path.line_to(e2);
                path.close_path();
                let mut builder = ShapeBuilder::new();
                let index = builder.add_fill(FillStyle::Solid(Rgba8::BLACK))?;
                builder.add_path(
                    &path,
                    if fill_side_is_right(&path) {
                        PathStyle { fill1: index, ..PathStyle::default() }
                    } else {
                        PathStyle { fill0: index, ..PathStyle::default() }
                    },
                );
                self.movie.define_shape(&builder.finish())?
            };
            let mut mask_slot = Slot::plain(mask, Matrix::IDENTITY);
            mask_slot.clip_depth = Some(2);
            let items = vec![FrameItem::Place(mask_slot.place_new(1)), FrameItem::Place(Slot::plain(content_sprite, Matrix::from_affine(map)).place_new(2))];
            let sprite = self.movie.define_sprite(&[Frame::new(items)])?;
            triangles.push(Slot::plain(sprite, Matrix::IDENTITY));
        }
        if triangles.is_empty() {
            return Ok(None);
        }
        self.note(ExportNote::MeshEmulated);
        let character = self.sprite_of(&triangles, None)?;
        Ok(Some(Slot::plain(character, Matrix::from_affine(mesh.transform))))
    }
}

/// The `LINESTYLE2` for a display-list stroke.
pub(crate) fn line_style(style: &StrokeStyle, paint: LinePaint) -> LineStyle {
    let width = u16::try_from(num::round_f64_to_i32(style.width.max(0.0) * crate::types::TWIPS_PER_PX).clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX);
    // (thickness stays fixed horizontally, thickness stays fixed vertically)
    let (fixed_x, fixed_y) = match style.scale {
        StrokeScale::Normal => (false, false),
        // "horizontal": the thickness scales only when the object is scaled horizontally
        StrokeScale::Horizontal => (false, true),
        StrokeScale::Vertical => (true, false),
        StrokeScale::None => (true, true),
    };
    LineStyle {
        width,
        paint,
        start_cap: style.cap,
        end_cap: style.cap,
        join: style.join,
        miter_limit: style.miter_limit,
        no_h_scale: fixed_x,
        no_v_scale: fixed_y,
        pixel_hinting: false,
        no_close: false,
    }
}

/// The affine map taking triangle `from` onto `to`, or `None` when `from` is degenerate.
fn triangle_map(from: [Point; 3], to: [Point; 3]) -> Option<Affine> {
    let [s0, s1, s2] = from;
    let [d0, d1, d2] = to;
    let (u1, u2) = (s1 - s0, s2 - s0);
    let det = u1.x.mul_add(u2.y, -(u1.y * u2.x));
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (v1, v2) = (d1 - d0, d2 - d0);
    // linear = [v1 v2] * inverse([u1 u2])
    let xx = v1.x.mul_add(u2.y, -(v2.x * u1.y)) / det;
    let xy = v2.x.mul_add(u1.x, -(v1.x * u2.x)) / det;
    let yx = v1.y.mul_add(u2.y, -(v2.y * u1.y)) / det;
    let yy = v2.y.mul_add(u1.x, -(v1.y * u2.x)) / det;
    let tx = xx.mul_add(-s0.x, xy.mul_add(-s0.y, d0.x));
    let ty = yx.mul_add(-s0.x, yy.mul_add(-s0.y, d0.y));
    let map = Affine::new([xx, yx, xy, yy, tx, ty]);
    map.as_coeffs().iter().all(|v| v.is_finite()).then_some(map)
}

/// Pushes the corners of a triangle `by` pixels away from its centre, so neighbouring masks overlap slightly and no
/// seams show between them.
fn grow_triangle(corners: [Point; 3], by: f64) -> [Point; 3] {
    let [p0, p1, p2] = corners;
    let centre = Point::new((p0.x + p1.x + p2.x) / 3.0, (p0.y + p1.y + p2.y) / 3.0);
    corners.map(|p| {
        let away = p - centre;
        let len = away.hypot();
        if len > 1e-9 { p + away * (by / len) } else { p }
    })
}

/// Exports a sequence of display lists with the given options.
///
/// # Errors
/// See [`DisplayListExporter::push_frame`] and [`DisplayListExporter::finish`].
pub fn export_display_lists<'a>(
    frames: impl IntoIterator<Item = &'a DisplayList>,
    images: &ImageStore,
    options: ExportOptions,
) -> Result<ExportOutput, SwfError> {
    let mut exporter = DisplayListExporter::new(options)?;
    for list in frames {
        exporter.push_frame(list, images)?;
    }
    exporter.finish()
}

/// Exports a single display list as a one-frame SWF.
///
/// # Errors
/// See [`export_display_lists`].
pub fn export_still(list: &DisplayList, images: &ImageStore) -> Result<ExportOutput, SwfError> {
    export_display_lists([list], images, ExportOptions::for_list(list, 24.0))
}

#[cfg(test)]
mod tests {
    use anim_core::Size;
    use anim_display::{Cap, FillRule, Join};

    use super::*;

    #[test]
    fn triangle_maps_are_exact_on_the_corners() {
        let from = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)];
        let to = [Point::new(5.0, 5.0), Point::new(5.0, 25.0), Point::new(-15.0, 5.0)];
        let map = triangle_map(from, to).unwrap();
        for (s, d) in from.iter().zip(to) {
            let p = map * *s;
            assert!((p.x - d.x).abs() < 1e-9 && (p.y - d.y).abs() < 1e-9, "{p:?} vs {d:?}");
        }
        assert!(triangle_map([Point::ORIGIN, Point::new(1.0, 1.0), Point::new(2.0, 2.0)], to).is_none());
    }

    #[test]
    fn triangles_grow_away_from_their_centre() {
        let grown = grow_triangle([Point::new(0.0, 0.0), Point::new(6.0, 0.0), Point::new(0.0, 6.0)], 1.0);
        assert!(grown[0].x < 0.0 && grown[0].y < 0.0);
        assert!(grown[1].x > 6.0);
    }

    #[test]
    fn line_style_conversion() {
        let style = StrokeStyle { width: 2.5, cap: Cap::Square, join: Join::Miter, miter_limit: 4.0, scale: StrokeScale::Horizontal, dashes: vec![] };
        let line = line_style(&style, LinePaint::Solid(Rgba8::BLACK));
        assert_eq!(line.width, 50);
        assert!(!line.no_h_scale && line.no_v_scale);
        assert_eq!(line.start_cap, Cap::Square);
    }

    #[test]
    fn invisible_nodes_produce_nothing() {
        let list = DisplayList {
            size: Size::new(10.0, 10.0),
            background: None,
            nodes: vec![Node::Fill(Fill { transform: Affine::IDENTITY, path: BezPath::new(), rule: FillRule::NonZero, paint: Paint::Solid(Rgba8::BLACK) })],
        };
        let out = export_still(&list, &ImageStore::new()).unwrap();
        assert_eq!(out.report.frames, 1);
        assert_eq!(out.report.characters, 0);
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut node = Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: kurbo::Rect::new(0.0, 0.0, 1.0, 1.0).to_path(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid(Rgba8::BLACK),
        });
        for _ in 0..(MAX_NESTING + 5) {
            node = Node::Group(Group { children: vec![node], ..Group::default() });
        }
        let list = DisplayList { size: Size::new(10.0, 10.0), background: None, nodes: vec![node] };
        assert!(matches!(export_still(&list, &ImageStore::new()), Err(SwfError::TooDeep(_))));
    }
}
