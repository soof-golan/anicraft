//! Guides and the grid: the per-document layout aids of the View menu.
//!
//! They live in the document as attributes of `DOMDocument` that the XFL reader keeps in [`Document::extra`], so they
//! survive a save and open round trip without a new field in the engine model, and an undo step restores them with
//! the rest of the document.

use anim_core::Rgba8;
use anim_scene::Document;

/// Most guides of one direction a document keeps.
pub const MAX_GUIDES: usize = 256;
/// Smallest and largest grid spacing in pixels.
pub const MIN_GRID: f64 = 1.0;
pub const MAX_GRID: f64 = 1000.0;

const ATTRS: [&str; 9] =
    ["guidesX", "guidesY", "guidesLocked", "guidesVisible", "guideColor", "gridVisible", "gridSpacingX", "gridSpacingY", "gridColor"];

/// The guides and the grid of a document.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// X positions of the vertical guides, in stage pixels.
    pub guides_x: Vec<f64>,
    /// Y positions of the horizontal guides.
    pub guides_y: Vec<f64>,
    /// Guides cannot be dragged.
    pub guides_locked: bool,
    /// View > Guides > Show Guides.
    pub guides_visible: bool,
    pub guide_color: Rgba8,
    /// View > Grid > Show Grid.
    pub grid_visible: bool,
    /// Horizontal and vertical grid spacing in pixels.
    pub grid: [f64; 2],
    pub grid_color: Rgba8,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            guides_x: Vec::new(),
            guides_y: Vec::new(),
            guides_locked: false,
            guides_visible: true,
            guide_color: Rgba8::opaque(0, 255, 255),
            grid_visible: false,
            grid: [18.0, 18.0],
            grid_color: Rgba8::opaque(192, 192, 192),
        }
    }
}

fn list(values: &[f64]) -> String {
    values.iter().take(MAX_GUIDES).map(f64::to_string).collect::<Vec<_>>().join(",")
}

fn parse_list(text: &str) -> Vec<f64> {
    text.split(',').filter_map(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).take(MAX_GUIDES).collect()
}

fn flag(text: Option<&str>, default: bool) -> bool {
    text.map_or(default, |t| t == "true")
}

fn colour(text: Option<&str>, default: Rgba8) -> Rgba8 {
    text.and_then(|t| Rgba8::from_hex(t).ok()).unwrap_or(default)
}

fn spacing(text: Option<&str>, default: f64) -> f64 {
    text.and_then(|t| t.parse::<f64>().ok()).filter(|v| v.is_finite()).map_or(default, |v| v.clamp(MIN_GRID, MAX_GRID))
}

/// Reads the layout aids of a document (the defaults for a document that has none).
#[must_use]
pub fn read(doc: &Document) -> Layout {
    let d = Layout::default();
    let e = &doc.extra;
    Layout {
        guides_x: e.attr("guidesX").map(parse_list).unwrap_or_default(),
        guides_y: e.attr("guidesY").map(parse_list).unwrap_or_default(),
        guides_locked: flag(e.attr("guidesLocked"), d.guides_locked),
        guides_visible: flag(e.attr("guidesVisible"), d.guides_visible),
        guide_color: colour(e.attr("guideColor"), d.guide_color),
        grid_visible: flag(e.attr("gridVisible"), d.grid_visible),
        grid: [spacing(e.attr("gridSpacingX"), d.grid[0]), spacing(e.attr("gridSpacingY"), d.grid[1])],
        grid_color: colour(e.attr("gridColor"), d.grid_color),
    }
}

/// Stores the layout aids; attributes that equal the defaults are left out of the file.
pub fn write(doc: &mut Document, layout: &Layout) {
    let d = Layout::default();
    for name in ATTRS {
        doc.extra.remove_attr(name);
    }
    let mut set = |name: &str, value: String| doc.extra.set_attr(name, value);
    if !layout.guides_x.is_empty() {
        set("guidesX", list(&layout.guides_x));
    }
    if !layout.guides_y.is_empty() {
        set("guidesY", list(&layout.guides_y));
    }
    if layout.guides_locked != d.guides_locked {
        set("guidesLocked", layout.guides_locked.to_string());
    }
    if layout.guides_visible != d.guides_visible {
        set("guidesVisible", layout.guides_visible.to_string());
    }
    if layout.guide_color != d.guide_color {
        set("guideColor", layout.guide_color.to_hex_rgb());
    }
    if layout.grid_visible != d.grid_visible {
        set("gridVisible", layout.grid_visible.to_string());
    }
    if (layout.grid[0] - d.grid[0]).abs() > f64::EPSILON {
        set("gridSpacingX", layout.grid[0].clamp(MIN_GRID, MAX_GRID).to_string());
    }
    if (layout.grid[1] - d.grid[1]).abs() > f64::EPSILON {
        set("gridSpacingY", layout.grid[1].clamp(MIN_GRID, MAX_GRID).to_string());
    }
    if layout.grid_color != d.grid_color {
        set("gridColor", layout.grid_color.to_hex_rgb());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trips_through_the_document_attributes() {
        let mut doc = Document::default();
        assert_eq!(read(&doc), Layout::default());
        let layout = Layout {
            guides_x: vec![10.0, 250.5],
            guides_y: vec![33.0],
            guides_locked: true,
            guides_visible: false,
            guide_color: Rgba8::opaque(255, 0, 0),
            grid_visible: true,
            grid: [20.0, 12.5],
            grid_color: Rgba8::opaque(1, 2, 3),
        };
        write(&mut doc, &layout);
        assert_eq!(read(&doc), layout);
        write(&mut doc, &Layout::default());
        assert!(doc.extra.attrs.is_empty(), "defaults leave no attributes behind");
    }

    #[test]
    fn hostile_attributes_are_clamped() {
        let mut doc = Document::default();
        doc.extra.set_attr("gridSpacingX", "-5");
        doc.extra.set_attr("gridSpacingY", "NaN");
        doc.extra.set_attr("guidesX", "1,x,inf,4");
        let layout = read(&doc);
        assert_eq!(layout.grid, [MIN_GRID, 18.0]);
        assert_eq!(layout.guides_x, vec![1.0, 4.0]);
    }
}
