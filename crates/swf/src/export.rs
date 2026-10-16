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
use crate::shape::{PathStyle, ShapeBuilder, ShapeDef, clean_path, filled_path_shape};
use crate::shape_cache::ShapeCache;
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
    /// A group under a perspective projection was emulated as an 8 x 8 mesh of affine triangles; SWF has no
    /// perspective, so straight lines inside it are only approximately straight.
    ProjectionTessellated,
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

/// Whether a path painted with `paint` can show anything at all. A solid colour with alpha 0 is only invisible when no
/// enclosing colour transform can add alpha back (`reveal`): a player applies the transform to the fill's colour
/// even when its alpha is 0.
fn is_visible(path: &BezPath, paint: &Paint, reveal: bool) -> bool {
    !path.elements().is_empty() && (reveal || !matches!(paint, Paint::Solid(c) if c.a == 0))
}

/// The depth (1-based) of the object at `index` in a display list.
fn depth_of(index: usize) -> Result<u16, SwfError> {
    index.checked_add(1).and_then(|d| u16::try_from(d).ok()).ok_or(SwfError::DepthOutOfRange(index))
}

/// The objects currently on one timeline's stage, by depth: what the next frame of that timeline is compared with,
/// so that only the depths that changed produce tags.
#[derive(Debug, Clone, Default)]
pub struct Stage {
    previous: Vec<Slot>,
}

impl Stage {
    /// An empty stage.
    #[must_use]
    pub const fn new() -> Self {
        Self { previous: Vec::new() }
    }
}

/// Converts display-list nodes into SWF characters and placements.
///
/// This is the engine behind [`DisplayListExporter`], public so that symbol exports can use it too: keep one
/// converter for a whole movie (it remembers which bitmaps are defined) and one [`Stage`] per timeline. Frames of a
/// movie clip come out of [`NodeConverter::define_sprite`]; the frames of the main timeline from
/// [`NodeConverter::next_frame`]. All of them share shapes, sprites and bitmaps through the [`Movie`].
#[derive(Debug, Default)]
pub struct NodeConverter {
    images: HashMap<ImageId, CharacterId>,
    /// Image ids that defined a bitmap, by a hash of their pixels.
    by_content: HashMap<u64, Vec<ImageId>>,
    bitmaps: u32,
    notes: Vec<ExportNote>,
    /// Shapes defined so far, by the nodes they came from.
    shapes: ShapeCache,
}

impl NodeConverter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What could not be carried over exactly so far.
    #[must_use]
    pub fn notes(&self) -> &[ExportNote] {
        &self.notes
    }

    /// Bitmap characters defined so far.
    #[must_use]
    pub const fn bitmap_count(&self) -> u32 {
        self.bitmaps
    }

    fn note(&mut self, note: ExportNote) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    /// Converts `list` and returns the frame that turns what `stage` holds into it.
    ///
    /// Definitions it needs are added to `movie` right away, so add the returned frame to the timeline before
    /// converting the next one.
    ///
    /// # Errors
    /// [`SwfError::MissingImage`] for a bitmap the store does not hold, [`SwfError::TooDeep`] for runaway nesting, and
    /// any limit of the format (65535 characters or depths).
    pub fn next_frame(&mut self, movie: &mut Movie, stage: &mut Stage, list: &DisplayList, images: &ImageStore) -> Result<Frame, SwfError> {
        let slots = self.convert_nodes(movie, &list.nodes, images, 0, false)?;
        let mut items = Vec::new();
        for index in 0..stage.previous.len().max(slots.len()) {
            let depth = depth_of(index)?;
            match (stage.previous.get(index), slots.get(index)) {
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
        stage.previous = slots;
        Ok(Frame::new(items))
    }

    /// Defines a movie clip whose timeline is `frames`, one display list per frame.
    ///
    /// # Errors
    /// See [`NodeConverter::next_frame`] and [`Movie::define_sprite`].
    pub fn define_sprite(&mut self, movie: &mut Movie, frames: &[DisplayList], images: &ImageStore) -> Result<CharacterId, SwfError> {
        let mut stage = Stage::new();
        let mut timeline = Vec::with_capacity(frames.len());
        for list in frames {
            timeline.push(self.next_frame(movie, &mut stage, list, images)?);
        }
        movie.define_sprite(&timeline)
    }

    /// `reveal`: some enclosing colour transform adds alpha, so fills with alpha 0 are kept.
    fn convert_nodes(&mut self, movie: &mut Movie, nodes: &[Node], images: &ImageStore, nesting: usize, reveal: bool) -> Result<Vec<Slot>, SwfError> {
        if nesting > MAX_NESTING {
            return Err(SwfError::TooDeep(MAX_NESTING));
        }
        let mut slots = Vec::with_capacity(nodes.len());
        let mut remaining = nodes.iter().peekable();
        while let Some(node) = remaining.next() {
            let slot = match node {
                // a fill immediately followed by a stroke of the same path is one SWF shape
                Node::Fill(fill) => match remaining.next_if(|next| matches!(next, Node::Stroke(s) if s.path == fill.path && s.transform == fill.transform)) {
                    Some(Node::Stroke(stroke)) => self.paint_slot(movie, Some(fill), Some(stroke), images, reveal)?,
                    _ => self.paint_slot(movie, Some(fill), None, images, reveal)?,
                },
                Node::Stroke(stroke) => self.paint_slot(movie, None, Some(stroke), images, reveal)?,
                Node::Bitmap(bitmap) => self.bitmap_slot(movie, bitmap, images)?,
                Node::Group(group) => self.group_slot(movie, group, images, nesting, reveal)?,
                Node::Mesh(mesh) => self.mesh_slot(movie, mesh, images, nesting, reveal)?,
            };
            slots.extend(slot);
        }
        Ok(slots)
    }

    /// The bitmap character for an image: one per image id, and images with identical pixels share a character.
    fn bitmap_character(&mut self, movie: &mut Movie, id: ImageId, images: &ImageStore) -> Result<CharacterId, SwfError> {
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
            let character = movie.define_bitmap(image)?;
            self.by_content.entry(fingerprint).or_default().push(id);
            character
        };
        self.images.insert(id, character);
        Ok(character)
    }

    fn fill_style(&mut self, movie: &mut Movie, paint: &Paint, images: &ImageStore) -> Result<FillStyle, SwfError> {
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
                let id = self.bitmap_character(movie, *image, images)?;
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

    fn define_slot(movie: &mut Movie, shape: &ShapeDef, transform: Affine) -> Result<Option<Slot>, SwfError> {
        let character = movie.define_shape(shape)?;
        Ok(Some(Slot::plain(character, Matrix::from_affine(transform))))
    }

    /// One shape holding a fill, a stroke, or a fill with the stroke drawn over it (SWF draws strokes over fills
    /// within a shape, which is exactly the order of a `Fill` node followed by a `Stroke` node).
    fn paint_slot(
        &mut self,
        movie: &mut Movie,
        fill: Option<&Fill>,
        stroke: Option<&Stroke>,
        images: &ImageStore,
        reveal: bool,
    ) -> Result<Option<Slot>, SwfError> {
        let fill = fill.filter(|f| is_visible(&f.path, &f.paint, reveal));
        let stroke = stroke.filter(|s| is_visible(&s.path, &s.paint, reveal));
        let transform = match (fill, stroke) {
            (Some(f), _) => f.transform,
            (None, Some(s)) => s.transform,
            (None, None) => return Ok(None),
        };
        if let Some(character) = self.shapes.get(fill, stroke) {
            return Ok(Some(Slot::plain(character, Matrix::from_affine(transform))));
        }
        let mut builder = ShapeBuilder::new();
        if let Some(fill) = fill {
            builder.set_fill_rule(fill.rule);
            let index = builder.add_fill(self.fill_style(movie, &fill.paint, images)?)?;
            let path = clean_path(&fill.path);
            builder.add_path(&path, PathStyle::filled_inside(&path, index));
        }
        if let Some(stroke) = stroke {
            let paint = match &stroke.paint {
                Paint::Solid(color) => LinePaint::Solid(*color),
                other => LinePaint::Fill(self.fill_style(movie, other, images)?),
            };
            let index = builder.add_line(line_style(&stroke.style, paint))?;
            let path = clean_path(&stroke.path);
            let style = PathStyle { line: index, ..PathStyle::default() };
            match self.dashed(&path, &stroke.style) {
                Some(dashes) => builder.add_elements(&dashes, style),
                None => builder.add_path(&path, style),
            }
        }
        let character = movie.define_shape(&builder.finish())?;
        self.shapes.insert(fill, stroke, character);
        Ok(Some(Slot::plain(character, Matrix::from_affine(transform))))
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

    fn bitmap_slot(&mut self, movie: &mut Movie, bitmap: &Bitmap, images: &ImageStore) -> Result<Option<Slot>, SwfError> {
        let character = self.bitmap_character(movie, bitmap.image, images)?;
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
        Self::define_slot(movie, &builder.finish(), bitmap.transform)
    }

    fn group_slot(&mut self, movie: &mut Movie, group: &Group, images: &ImageStore, nesting: usize, reveal: bool) -> Result<Option<Slot>, SwfError> {
        if let Some(projection) = &group.projection {
            return self.projected_slot(movie, group, projection, images, nesting, reveal);
        }
        let reveal = reveal || group.color.add.get(3).is_some_and(|a| *a > 0.0);
        let children = self.convert_nodes(movie, &group.children, images, nesting.saturating_add(1), reveal)?;
        if children.is_empty() {
            return Ok(None);
        }
        let mask = match &group.mask {
            Some(nodes) => {
                let slots = self.convert_nodes(movie, nodes, images, nesting.saturating_add(1), false)?;
                match slots.as_slice() {
                    // an empty mask hides everything
                    [] => return Ok(None),
                    [only] => Some(only.clone()),
                    many => Some(Slot::plain(sprite_of(movie, many, None)?, Matrix::IDENTITY)),
                }
            }
            None => None,
        };
        let character = sprite_of(movie, &children, mask)?;
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

    /// SWF has no perspective: a projected group becomes a mesh whose grid of points is carried to where the
    /// projection puts them, so the picture is correct at the grid points and affine inside each cell. Cells with a
    /// corner behind the viewer are left out.
    fn projected_slot(
        &mut self,
        movie: &mut Movie,
        group: &Group,
        projection: &anim_core::Homography,
        images: &ImageStore,
        nesting: usize,
        reveal: bool,
    ) -> Result<Option<Slot>, SwfError> {
        const CELLS: u32 = 8;
        let flat = Group { projection: None, ..group.clone() };
        let content = vec![Node::Group(flat)];
        let Some(bounds) = anim_render::nodes_bounds(&content, images, Affine::IDENTITY) else {
            return Ok(None);
        };
        let side = CELLS.saturating_add(1);
        let mut source = Vec::new();
        let mut target = Vec::new();
        let mut valid = Vec::new();
        for row in 0..side {
            for column in 0..side {
                let u = f64::from(column) / f64::from(CELLS);
                let v = f64::from(row) / f64::from(CELLS);
                let point = Point::new(bounds.x0, bounds.y0) + kurbo::Vec2::new(bounds.width() * u, bounds.height() * v);
                let projected = projection.apply(point);
                source.push(point);
                valid.push(projected.is_some());
                target.push(projected.unwrap_or(point));
            }
        }
        let mut triangles = Vec::new();
        for row in 0..CELLS {
            for column in 0..CELLS {
                let a = row.saturating_mul(side).saturating_add(column);
                let b = a.saturating_add(1);
                let c = a.saturating_add(side);
                let d = c.saturating_add(1);
                let ok = [a, b, c, d].iter().all(|i| usize::try_from(*i).ok().and_then(|i| valid.get(i)).copied().unwrap_or(false));
                if ok {
                    triangles.push([a, b, c]);
                    triangles.push([b, d, c]);
                }
            }
        }
        let mesh = Mesh { transform: Affine::IDENTITY, content, source, target, triangles };
        self.note(ExportNote::ProjectionTessellated);
        self.mesh_slot(movie, &mesh, images, nesting, reveal)
    }

    /// A mesh warps its content triangle by triangle: each triangle is a sprite with the triangle as a clip mask
    /// over a copy of the content moved by the affine map that carries the source triangle onto the target one.
    fn mesh_slot(&mut self, movie: &mut Movie, mesh: &Mesh, images: &ImageStore, nesting: usize, reveal: bool) -> Result<Option<Slot>, SwfError> {
        let content = self.convert_nodes(movie, &mesh.content, images, nesting.saturating_add(1), reveal)?;
        if content.is_empty() {
            return Ok(None);
        }
        let content_sprite = sprite_of(movie, &content, None)?;
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
                movie.define_shape(&filled_path_shape(&path, FillStyle::Solid(Rgba8::BLACK), anim_display::FillRule::EvenOdd)?)?
            };
            let mut mask_slot = Slot::plain(mask, Matrix::IDENTITY);
            mask_slot.clip_depth = Some(2);
            let items = vec![FrameItem::Place(mask_slot.place_new(1)), FrameItem::Place(Slot::plain(content_sprite, Matrix::from_affine(map)).place_new(2))];
            let sprite = movie.define_sprite(&[Frame::new(items)])?;
            triangles.push(Slot::plain(sprite, Matrix::IDENTITY));
        }
        if triangles.is_empty() {
            return Ok(None);
        }
        self.note(ExportNote::MeshEmulated);
        let character = sprite_of(movie, &triangles, None)?;
        Ok(Some(Slot::plain(character, Matrix::from_affine(mesh.transform))))
    }
}

/// A one-frame sprite showing `slots` bottom to top, with `mask` (if any) clipping all of them.
fn sprite_of(movie: &mut Movie, slots: &[Slot], mask: Option<Slot>) -> Result<CharacterId, SwfError> {
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
    movie.define_sprite(&[Frame::new(items)])
}

/// Turns display lists into a SWF, one frame at a time.
#[derive(Debug)]
pub struct DisplayListExporter {
    movie: Movie,
    converter: NodeConverter,
    stage: Stage,
    loop_playback: bool,
    pending: Option<Frame>,
}

impl DisplayListExporter {
    /// Starts an export.
    ///
    /// # Errors
    /// Invalid movie settings, see [`Movie::new`].
    pub fn new(options: ExportOptions) -> Result<Self, SwfError> {
        Ok(Self {
            movie: Movie::new(options.movie)?,
            converter: NodeConverter::new(),
            stage: Stage::new(),
            loop_playback: options.loop_playback,
            pending: None,
        })
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
        self.pending = Some(self.converter.next_frame(&mut self.movie, &mut self.stage, list, images)?);
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
        let report = ExportReport {
            frames: self.movie.frame_count(),
            characters: self.movie.character_count(),
            bitmaps: self.converter.bitmap_count(),
            notes: self.converter.notes.clone(),
        };
        Ok(ExportOutput { swf: self.movie.finish()?, report })
    }
}

/// The `LINESTYLE2` for a display-list stroke.
pub(crate) fn line_style(style: &StrokeStyle, paint: LinePaint) -> LineStyle {
    let width = u16::try_from(num::round_f64_to_i32(style.width.max(0.0) * crate::types::TWIPS_PER_PX).clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX);
    // (NoHScale, NoVScale): the thickness does not follow horizontal / vertical scaling. `Horizontal` is
    // `LineScaleMode.HORIZONTAL` ("do not scale the thickness if only horizontal scaling is applied"), which is the
    // `NoHScale` bit, as anim-render also reads it.
    let (fixed_x, fixed_y) = match style.scale {
        StrokeScale::Normal => (false, false),
        StrokeScale::Horizontal => (true, false),
        StrokeScale::Vertical => (false, true),
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
        assert!(line.no_h_scale && !line.no_v_scale);
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

    #[test]
    fn a_projected_group_is_tessellated_into_a_mesh() {
        let perspective = anim_core::Perspective::for_stage(200.0, 200.0);
        let projection = anim_core::Matrix3D::rotation_y(40.0).then(&anim_core::Matrix3D::translation(100.0, 100.0, 0.0)).homography(&perspective);
        let square = Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: kurbo::Rect::new(-40.0, -40.0, 40.0, 40.0).to_path(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid(Rgba8::BLACK),
        });
        let list = DisplayList {
            size: Size::new(200.0, 200.0),
            background: None,
            nodes: vec![Node::Group(Group { projection: Some(projection), children: vec![square], ..Group::default() })],
        };
        let out = export_still(&list, &ImageStore::new()).unwrap();
        assert!(out.report.notes.contains(&ExportNote::ProjectionTessellated));
        // 8 x 8 cells, two triangles each, a shape and a mask per triangle plus the shared content.
        assert!(out.report.characters > 128, "{}", out.report.characters);
    }
}
