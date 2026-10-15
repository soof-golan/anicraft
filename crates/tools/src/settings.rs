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
            text: TextSettings::default(),
            hit_tolerance: 4.0,
            drag_threshold: 3.0,
        }
    }
}

impl ToolSettings {
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
