//! View models: plain functions from the document and the editor state to what the `anim-ui` widgets draw.

use std::collections::{BTreeSet, HashMap};

use anicraft_tools::{Cursor, ElementRef, Overlay as ToolOverlay, Role};
use anim_core::{Rgba8, num};
use anim_scene::timeline::{LayerKind as SceneLayerKind, TweenType};
use anim_scene::{Document, Element, LibraryItem, SymbolKind, Timeline, TimelineId};
use anim_ui::library::{LibraryItemView, LibraryKind, LibraryModel};
use anim_ui::stage::{MarkerShape, Overlay, OverlayStyle, PathNode};
use anim_ui::timeline::{LayerKind, LayerRow, OnionSkin, Span, TimelineModel, TweenKind};
use egui::{Color32, CursorIcon, Pos2, Rect, pos2, vec2};

/// A position as the `u64` id the widgets use.
#[must_use]
pub fn id_of(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

/// The scene or symbol timeline the editor is working in.
#[must_use]
pub fn editing_timeline<'a>(doc: &'a Document, id: &TimelineId) -> Option<&'a Timeline> {
    doc.timeline(id)
}

/// The timeline panel's model for the timeline being edited.
#[must_use]
pub fn timeline_model(doc: &Document, id: &TimelineId, frame: u32, onion: OnionSkin, playing: bool, collapsed: &BTreeSet<u64>) -> TimelineModel {
    let Some(timeline) = doc.timeline(id) else { return TimelineModel { fps: 24.0, frame_count: 1, ..TimelineModel::default() } };
    let mut layers = Vec::with_capacity(timeline.layers.len());
    for (index, layer) in timeline.layers.iter().enumerate() {
        let kind = match timeline.resolved_kind(index).unwrap_or(layer.kind) {
            SceneLayerKind::Normal => LayerKind::Normal,
            SceneLayerKind::Guide => LayerKind::Guide,
            SceneLayerKind::Guided => LayerKind::Guided,
            SceneLayerKind::Mask => LayerKind::Mask,
            SceneLayerKind::Masked => LayerKind::Masked,
            SceneLayerKind::Folder => LayerKind::Folder,
            SceneLayerKind::Camera => LayerKind::Camera,
        };
        let mut depth = 0_u32;
        let mut parent = layer.parent_layer_index;
        let mut guard = 0;
        while let Some(p) = parent {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(parent_layer) = timeline.layers.get(p) else { break };
            if matches!(parent_layer.kind, SceneLayerKind::Folder) || parent_layer.kind == SceneLayerKind::Mask || parent_layer.kind == SceneLayerKind::Guide {
                depth = depth.saturating_add(1);
            }
            parent = parent_layer.parent_layer_index;
        }
        let spans = layer
            .frames
            .iter()
            .map(|f| {
                let tween = match f.tween_type {
                    TweenType::None => TweenKind::None,
                    TweenType::Classic => TweenKind::Classic,
                    TweenType::Shape => TweenKind::Shape,
                    TweenType::MotionObject => TweenKind::Motion,
                };
                let broken = f.is_tweened() && layer.next_keyframe_after(f.index).is_none();
                Span {
                    start: f.index,
                    len: f.duration.max(1),
                    blank: f.is_blank() && !f.is_tweened(),
                    tween,
                    tween_broken: broken,
                    label: f.label.clone(),
                    has_script: f.actionscript.as_deref().is_some_and(|s| !s.trim().is_empty()),
                    sound: None,
                }
            })
            .collect();
        let id = id_of(index);
        layers.push(LayerRow {
            id,
            name: layer.name.clone(),
            kind,
            depth,
            expanded: !collapsed.contains(&id),
            color: layer.color,
            visible: layer.visible,
            locked: layer.locked,
            outline: layer.outline,
            spans,
        });
    }
    let used = layers.iter().filter_map(|l| l.spans.last()).map(Span::end).max().unwrap_or(1);
    TimelineModel { layers, frame_count: used.max(timeline.frame_count()).max(1), current_frame: frame, fps: num::f64_to_f32(doc.frame_rate), onion, playing }
}

/// Library rows in tree order with folder collapsing, and the library index each row stands for.
#[derive(Debug, Clone, Default)]
pub struct LibraryRows {
    pub model: LibraryModel,
    /// `ids[row.id]` is the item's name; the widget's ids are positions in `names`.
    pub names: Vec<String>,
}

fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn depth_of(name: &str) -> u32 {
    num::usize_to_u32_sat(name.matches('/').count())
}

fn parent_of(name: &str) -> Option<&str> {
    name.rsplit_once('/').map(|(p, _)| p)
}

/// The library panel's model. Items are sorted so folders come before their contents.
#[must_use]
pub fn library_rows(doc: &Document, collapsed: &BTreeSet<String>, filter: &str, selected: Option<&str>, thumbs: &HashMap<String, egui::TextureId>) -> LibraryRows {
    let mut order: Vec<&LibraryItem> = doc.library.items.iter().collect();
    order.sort_by(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase()));
    let needle = filter.to_lowercase();
    let mut items = Vec::new();
    let mut names = Vec::new();
    let mut selected_id = None;
    for item in order {
        let name = item.name();
        if !needle.is_empty() {
            if matches!(item, LibraryItem::Folder(_)) || !name.to_lowercase().contains(&needle) {
                continue;
            }
        } else if hidden_by_collapsed(name, collapsed) {
            continue;
        }
        let kind = match item {
            LibraryItem::Folder(_) => LibraryKind::Folder,
            LibraryItem::Symbol(s) => match s.kind {
                SymbolKind::Graphic => LibraryKind::Graphic,
                SymbolKind::MovieClip => LibraryKind::MovieClip,
                SymbolKind::Button => LibraryKind::Button,
            },
            LibraryItem::Bitmap(_) => LibraryKind::Bitmap,
            LibraryItem::Sound(_) => LibraryKind::Sound,
            LibraryItem::Video(_) => LibraryKind::Video,
            LibraryItem::Font(_) => LibraryKind::Font,
        };
        let id = id_of(names.len());
        if selected == Some(name) {
            selected_id = Some(id);
        }
        items.push(LibraryItemView {
            id,
            name: if needle.is_empty() { leaf(name).to_owned() } else { name.to_owned() },
            kind,
            depth: if needle.is_empty() { depth_of(name) } else { 0 },
            expanded: !collapsed.contains(name),
            thumbnail: thumbs.get(name).copied(),
        });
        names.push(name.to_owned());
    }
    LibraryRows { model: LibraryModel { items, selected: selected_id, filter: filter.to_owned() }, names }
}

fn hidden_by_collapsed(name: &str, collapsed: &BTreeSet<String>) -> bool {
    let mut current = parent_of(name);
    while let Some(parent) = current {
        if collapsed.contains(parent) {
            return true;
        }
        current = parent_of(parent);
    }
    false
}

/// Colours for the roles tools draw in.
#[must_use]
pub fn role_colour(role: Role, accent: Color32) -> Color32 {
    match role {
        Role::Selection | Role::Handle => accent,
        Role::Preview => Color32::from_rgb(255, 170, 40),
        Role::Hover => Color32::from_rgb(120, 200, 255),
        Role::Pivot => Color32::from_rgb(230, 70, 70),
        Role::Guide => Color32::from_rgb(160, 160, 170),
    }
}

fn p(point: anim_core::Point) -> Pos2 {
    pos2(num::f64_to_f32(point.x), num::f64_to_f32(point.y))
}

fn r(rect: anim_core::Rect) -> Rect {
    Rect::from_min_max(pos2(num::f64_to_f32(rect.x0), num::f64_to_f32(rect.y0)), pos2(num::f64_to_f32(rect.x1), num::f64_to_f32(rect.y1)))
}

/// Flattens a bezier path to polylines (stage pixels) for drawing.
fn flatten(path: &anim_core::BezPath) -> Vec<(Vec<Pos2>, bool)> {
    let mut out: Vec<(Vec<Pos2>, bool)> = Vec::new();
    anim_core::kurbo::flatten(path.iter(), 0.25, |el| match el {
        anim_core::kurbo::PathEl::MoveTo(pt) => out.push((vec![p(pt)], false)),
        anim_core::kurbo::PathEl::LineTo(pt) => {
            if let Some((points, _)) = out.last_mut() {
                points.push(p(pt));
            }
        }
        anim_core::kurbo::PathEl::ClosePath => {
            if let Some((_, closed)) = out.last_mut() {
                *closed = true;
            }
        }
        _ => {}
    });
    out
}

/// Converts what the active tool wants drawn into stage overlays.
#[must_use]
pub fn overlays(tool: &[ToolOverlay], accent: Color32) -> Vec<Overlay> {
    let mut out = Vec::new();
    for item in tool {
        match item {
            ToolOverlay::Rect { rect, role } => {
                let colour = role_colour(*role, accent);
                let style = if matches!(role, Role::Preview) { OverlayStyle::dashed(colour) } else { OverlayStyle::line(colour) };
                out.push(Overlay::Rect { rect: r(*rect), style });
            }
            ToolOverlay::Line { a, b, role } => {
                out.push(Overlay::Line { a: p(*a), b: p(*b), style: OverlayStyle::line(role_colour(*role, accent)) });
            }
            ToolOverlay::Polyline { points, closed, role } => {
                out.push(Overlay::Polygon { points: points.iter().map(|pt| p(*pt)).collect(), closed: *closed, style: OverlayStyle::line(role_colour(*role, accent)) });
            }
            ToolOverlay::Path { path, role, width: _, filled } => {
                let colour = role_colour(*role, accent);
                for (points, closed) in flatten(path) {
                    let mut style = OverlayStyle::line(colour);
                    if *filled {
                        style = style.filled(colour.gamma_multiply(0.25));
                    }
                    out.push(Overlay::Polygon { points, closed: closed || *filled, style });
                }
            }
            ToolOverlay::Nodes { nodes, closed, role } => {
                out.push(Overlay::Path {
                    nodes: nodes
                        .iter()
                        .map(|n| PathNode { pos: p(n.pos), control_in: n.control_in.map(p), control_out: n.control_out.map(p), selected: n.selected })
                        .collect(),
                    closed: *closed,
                    style: OverlayStyle::line(role_colour(*role, accent)),
                    show_controls: true,
                });
            }
            ToolOverlay::Marker { pos, marker, role, filled } => {
                let shape = match marker {
                    anicraft_tools::Marker::Square => MarkerShape::Square,
                    anicraft_tools::Marker::Circle => MarkerShape::Circle,
                    anicraft_tools::Marker::Diamond => MarkerShape::Diamond,
                };
                out.push(Overlay::Marker { pos: p(*pos), shape, color: role_colour(*role, accent), filled: *filled });
            }
            ToolOverlay::Marquee { rect } => out.push(Overlay::Marquee { rect: r(*rect) }),
            ToolOverlay::Crosshair { pos, role } => out.push(Overlay::Crosshair { pos: p(*pos), color: role_colour(*role, accent) }),
            ToolOverlay::TextCursor { .. } => {}
        }
    }
    out
}

/// A selection box around a stage rectangle.
#[must_use]
pub fn selection_box(rect: anim_core::Rect, accent: Color32) -> Overlay {
    Overlay::Rect { rect: r(rect), style: OverlayStyle::line(accent) }
}

/// Maps a tool cursor to an egui cursor icon.
#[must_use]
pub const fn cursor_icon(cursor: Cursor) -> CursorIcon {
    match cursor {
        Cursor::Default => CursorIcon::Default,
        Cursor::Move => CursorIcon::Move,
        Cursor::Bend | Cursor::Corner | Cursor::Point | Cursor::Pivot => CursorIcon::PointingHand,
        Cursor::Crosshair | Cursor::Pen | Cursor::PenClose | Cursor::Pencil | Cursor::Brush | Cursor::Bucket | Cursor::InkBottle | Cursor::Eraser | Cursor::Eyedropper => {
            CursorIcon::Crosshair
        }
        Cursor::Text => CursorIcon::Text,
        Cursor::Hand => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
        Cursor::Resize(_) | Cursor::Rotate | Cursor::Skew => CursorIcon::AllScroll,
    }
}

/// Where an element ref points, as the stage rectangle it covers, for the selection outline.
#[must_use]
pub fn selection_rects(doc: &Document, id: &TimelineId, refs: &[ElementRef]) -> Vec<anim_core::Rect> {
    let bounds = anicraft_tools::LibraryBounds::new(&doc.library);
    refs.iter()
        .filter_map(|r| {
            let resolved = anicraft_tools::resolve_ref(doc, id, r)?;
            let local = anicraft_tools::hit::bounds_in_parent(&bounds, resolved.element, 0)?;
            anicraft_tools::hit::transform_rect(resolved.parent_to_stage, local)
        })
        .collect()
}

/// A short label for an element, for the properties panel header.
#[must_use]
pub fn element_title(element: &Element) -> String {
    match element {
        Element::Shape(s) => if s.is_drawing_object { "Drawing Object" } else { "Shape" }.to_owned(),
        Element::SymbolInstance(i) => format!("Instance of {}", i.library_item_name.rsplit('/').next().unwrap_or(&i.library_item_name)),
        Element::Bitmap(b) => format!("Bitmap {}", b.library_item_name),
        Element::Text(_) => "Text".to_owned(),
        Element::Group(g) => format!("Group of {}", g.members.len()),
        Element::Unknown(_) => "Unsupported element".to_owned(),
    }
}

/// Opaque colour used for layer outlines and tints in the UI.
#[must_use]
pub const fn to_color32(c: Rgba8) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// A small helper for panel sizes.
#[must_use]
pub fn panel_hint(width: f32, height: f32) -> egui::Vec2 {
    vec2(width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_scene::Document;

    #[test]
    fn the_default_document_has_one_layer_one_span() {
        let doc = Document::default();
        let model = timeline_model(&doc, &TimelineId::Scene(0), 0, OnionSkin::default(), false, &BTreeSet::new());
        assert_eq!(model.layers.len(), 1);
        assert_eq!(model.layers.first().map(|l| l.spans.len()), Some(1));
        assert!(model.frame_count >= 1);
    }

    #[test]
    fn library_rows_nest_and_collapse() {
        let mut doc = Document::default();
        doc.add_symbol("folder/wing", SymbolKind::Graphic).unwrap();
        doc.add_symbol("body", SymbolKind::MovieClip).unwrap();
        let open = library_rows(&doc, &BTreeSet::new(), "", None, &HashMap::new());
        assert_eq!(open.names.len(), 3, "{:?}", open.names);
        let mut closed = BTreeSet::new();
        closed.insert("folder".to_owned());
        let rows = library_rows(&doc, &closed, "", None, &HashMap::new());
        assert_eq!(rows.names.len(), 2);
        let filtered = library_rows(&doc, &BTreeSet::new(), "WIN", None, &HashMap::new());
        assert_eq!(filtered.names, vec!["folder/wing".to_owned()]);
    }

    #[test]
    fn cursors_map() {
        assert_eq!(cursor_icon(Cursor::Text), CursorIcon::Text);
    }
}
