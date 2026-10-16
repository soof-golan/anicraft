//! Tool options: the Properties panel and toolbox state that tools read.

use anim_core::Rgba8;
use anim_shape::{FillStyle, StrokeStyle};
use serde::{Deserialize, Serialize};

/// How the pencil treats a freehand stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PencilMode {
    /// Straight segments between the corners of the stroke.
    Straighten,
    /// Curves fitted through the stroke.
    #[default]
    Smooth,
    /// Only light clean-up; the stroke stays close to the pointer path.
    Ink,
}

/// What the eraser removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EraserMode {
    /// Strokes and fills under the brush.
    #[default]
    Normal,
    /// Fills only.
    Fills,
    /// Strokes only.
    Lines,
    /// Only the selected shapes.
    SelectedFills,
    /// Only the fill that the press started in.
    Inside,
}

/// Gap closing for the paint bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapSize {
    /// Only completely closed regions.
    #[default]
    None,
    Small,
    Medium,
    Large,
}

impl GapSize {
    /// The largest gap that is closed, in pixels.
    #[must_use]
    pub const fn pixels(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Small => 4.0,
            Self::Medium => 8.0,
            Self::Large => 16.0,
        }
    }
}

/// Where the brush paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushMode {
    /// Over everything on the layer.
    #[default]
    PaintNormal,
    /// Over fills and the empty stage, leaving strokes untouched.
    PaintFills,
    /// Only where nothing is painted yet.
    PaintBehind,
    /// Only inside the selected fills.
    PaintSelection,
    /// Only inside the fill the press started in.
    PaintInside,
}

impl BrushMode {
    /// All modes in the order the options menu lists them.
    pub const ALL: [Self; 5] = [Self::PaintNormal, Self::PaintFills, Self::PaintBehind, Self::PaintSelection, Self::PaintInside];

    /// The menu label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PaintNormal => "Paint Normal",
            Self::PaintFills => "Paint Fills",
            Self::PaintBehind => "Paint Behind",
            Self::PaintSelection => "Paint Selection",
            Self::PaintInside => "Paint Inside",
        }
    }
}

/// The tip of the brush or the eraser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipShape {
    #[default]
    Round,
    Square,
    /// Wider than tall.
    HorizontalEllipse,
    /// Taller than wide.
    VerticalEllipse,
    /// A thin tip leaning like `/`.
    Slash,
    /// A thin tip leaning like `\`.
    Backslash,
}

impl TipShape {
    /// All shapes in the order the options menu lists them.
    pub const ALL: [Self; 6] = [Self::Round, Self::Square, Self::HorizontalEllipse, Self::VerticalEllipse, Self::Slash, Self::Backslash];

    /// The menu label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Round => "Round",
            Self::Square => "Square",
            Self::HorizontalEllipse => "Horizontal ellipse",
            Self::VerticalEllipse => "Vertical ellipse",
            Self::Slash => "Slash",
            Self::Backslash => "Backslash",
        }
    }
}

/// The oval tool's arc settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OvalSettings {
    /// Where the arc starts, in degrees clockwise from three o'clock.
    pub start_angle: f64,
    /// Where the arc ends, in degrees.
    pub end_angle: f64,
    /// Radius of the hole as a percentage of the outer radius (0 to 99).
    pub inner_radius: f64,
    /// Whether an open arc is closed with a straight edge.
    pub close_path: bool,
}

impl Default for OvalSettings {
    fn default() -> Self {
        Self { start_angle: 0.0, end_angle: 0.0, inner_radius: 0.0, close_path: true }
    }
}

impl OvalSettings {
    /// True for a plain full ellipse.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.inner_radius <= 0.0 && (self.start_angle - self.end_angle).rem_euclid(360.0) < 1.0e-9
    }
}

/// Snapping switches of the View > Snapping menu.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "independent snapping switches")]
pub struct SnapSettings {
    /// Snap to the anchor points of other shapes.
    pub objects: bool,
    /// Snap to whole stage pixels.
    pub pixels: bool,
    /// Snap to the grid.
    pub grid: bool,
    /// Snap to the guides.
    pub guides: bool,
    /// Grid spacing in pixels.
    pub grid_size: f64,
    /// Horizontal guides (y positions).
    pub guides_y: Vec<f64>,
    /// Vertical guides (x positions).
    pub guides_x: Vec<f64>,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self { objects: false, pixels: false, grid: false, guides: false, grid_size: 10.0, guides_y: Vec::new(), guides_x: Vec::new() }
    }
}

/// Defaults for new text fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSettings {
    pub face: String,
    pub size: f32,
    pub color: Rgba8,
    pub bold: bool,
    pub italic: bool,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self { face: "Arial".to_owned(), size: 24.0, color: Rgba8::BLACK, bold: false, italic: false }
    }
}

/// Everything the tools read from the toolbox and the Properties panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(clippy::struct_excessive_bools, reason = "independent toolbox switches that mirror the Properties panel")]
pub struct ToolSettings {
    pub fill: FillStyle,
    pub fill_enabled: bool,
    pub stroke: StrokeStyle,
    pub stroke_enabled: bool,
    /// Object drawing mode: new shapes are separate drawing objects instead of merging with what is on the layer.
    pub object_drawing: bool,
    /// Corner radius of the rectangle tool, in pixels.
    pub corner_radius: f64,
    pub pencil_mode: PencilMode,
    /// Brush diameter at full pressure, in pixels.
    pub brush_size: f64,
    /// Whether pen pressure changes the brush width.
    pub brush_pressure: bool,
    pub eraser_mode: EraserMode,
    /// Eraser diameter in pixels.
    pub eraser_size: f64,
    pub bucket_gap: GapSize,
    /// Where the brush paints.
    #[serde(default)]
    pub brush_mode: BrushMode,
    /// The tip of the brush.
    #[serde(default)]
    pub brush_shape: TipShape,
    /// How much the brush path is smoothed, 0 to 100.
    #[serde(default = "default_brush_smoothing")]
    pub brush_smoothing: f64,
    /// The tip of the eraser.
    #[serde(default)]
    pub eraser_shape: TipShape,
    /// The eraser faucet: a click removes the whole fill or stroke under the pointer.
    #[serde(default)]
    pub eraser_faucet: bool,
    /// The paint bucket keeps gradient and bitmap fills in the coordinates of the shape instead of each region.
    #[serde(default)]
    pub bucket_lock_fill: bool,
    /// Radii of the rectangle tool's corners (top left, top right, bottom right, bottom left), used when unlocked.
    #[serde(default)]
    pub corner_radii: [f64; 4],
    /// Whether all corners share `corner_radius`.
    #[serde(default = "default_true")]
    pub corner_lock: bool,
    /// Arc settings of the oval tool.
    #[serde(default)]
    pub oval: OvalSettings,
    /// Snapping switches.
    #[serde(default)]
    pub snap: SnapSettings,
    pub text: TextSettings,
    /// Hit tolerance in screen points (divided by the zoom to get stage pixels).
    pub hit_tolerance: f64,
    /// Drag distance in screen points before a press becomes a drag.
    pub drag_threshold: f64,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            fill: FillStyle::Solid(Rgba8::opaque(255, 0, 0)),
            fill_enabled: true,
            stroke: StrokeStyle::plain(1.0, Rgba8::BLACK),
            stroke_enabled: true,
            object_drawing: false,
            corner_radius: 0.0,
            pencil_mode: PencilMode::Smooth,
            brush_size: 12.0,
            brush_pressure: true,
            eraser_mode: EraserMode::Normal,
            eraser_size: 16.0,
            bucket_gap: GapSize::None,
            brush_mode: BrushMode::PaintNormal,
            brush_shape: TipShape::Round,
            brush_smoothing: default_brush_smoothing(),
            eraser_shape: TipShape::Round,
            eraser_faucet: false,
            bucket_lock_fill: false,
            corner_radii: [0.0; 4],
            corner_lock: true,
            oval: OvalSettings::default(),
            snap: SnapSettings::default(),
            text: TextSettings::default(),
            hit_tolerance: 4.0,
            drag_threshold: 3.0,
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn default_brush_smoothing() -> f64 {
    50.0
}

impl ToolSettings {
    /// The corner radii the rectangle tool uses (top left, top right, bottom right, bottom left).
    #[must_use]
    pub const fn radii(&self) -> [f64; 4] {
        if self.corner_lock { [self.corner_radius; 4] } else { self.corner_radii }
    }

    /// The fill to give new shapes, if fills are on.
    #[must_use]
    pub fn active_fill(&self) -> Option<FillStyle> {
        self.fill_enabled.then(|| self.fill.clone())
    }

    /// The stroke to give new shapes, if strokes are on.
    #[must_use]
    pub fn active_stroke(&self) -> Option<StrokeStyle> {
        self.stroke_enabled.then(|| self.stroke.clone())
    }
}
