//! What a tool may look at while handling an event.

use anim_core::{Affine, Rect};
use anim_scene::{Document, Element, Layer, Library, Timeline, TimelineId};

use crate::selection::{ElementRef, Selection};
use crate::settings::ToolSettings;

/// Supplies bounds for elements whose extent the document model cannot know by itself.
///
/// The default [`LibraryBounds`] uses the library (symbol contents, bitmap sizes) and the text field rectangle. An
/// application with a text layout engine can return the real ink bounds of text instead.
pub trait BoundsProvider {
    /// Bounds of a symbol instance, bitmap or text field in its own coordinates (before its matrix). Shapes and
    /// groups are measured by the tools themselves and never reach this method.
    fn local_bounds(&self, element: &Element) -> Option<Rect>;
}

/// [`BoundsProvider`] backed by the document library.
#[derive(Debug, Clone, Copy)]
pub struct LibraryBounds<'a> {
    pub library: &'a Library,
}

impl<'a> LibraryBounds<'a> {
    #[must_use]
    pub const fn new(library: &'a Library) -> Self {
        Self { library }
    }
}

impl BoundsProvider for LibraryBounds<'_> {
    fn local_bounds(&self, element: &Element) -> Option<Rect> {
        let mut plain = element.clone();
        plain.set_matrix(Affine::IDENTITY);
        anim_scene::element_bounds(&plain, self.library)
    }
}

/// An element together with the transform from its parent's space to the stage.
#[derive(Debug, Clone, Copy)]
pub struct Resolved<'a> {
    pub element: &'a Element,
    /// Maps the coordinates the element's own matrix produces (its parent's space) to the stage.
    pub parent_to_stage: Affine,
}

impl Resolved<'_> {
    /// The element's own matrix (identity for kinds without one).
    #[must_use]
    pub fn matrix(&self) -> Affine {
        self.element.matrix().unwrap_or(Affine::IDENTITY)
    }

    /// Maps the element's local coordinates to the stage.
    #[must_use]
    pub fn local_to_stage(&self) -> Affine {
        self.parent_to_stage * self.matrix()
    }
}

/// Resolves a reference in a timeline of `document`.
#[must_use]
pub fn resolve_ref<'a>(document: &'a Document, timeline: &TimelineId, r: &ElementRef) -> Option<Resolved<'a>> {
    let layer = document.timeline(timeline)?.layers.get(r.layer)?;
    let frame = layer.frames.iter().find(|f| f.index == r.keyframe)?;
    let mut elements: &'a [Element] = &frame.elements;
    let mut parent_to_stage = Affine::IDENTITY;
    let (last, ancestors) = r.path.split_last()?;
    for &i in ancestors {
        let Element::Group(g) = elements.get(i)? else { return None };
        parent_to_stage *= g.matrix;
        elements = &g.members;
    }
    Some(Resolved { element: elements.get(*last)?, parent_to_stage })
}

/// Read-only view of the editor for one event.
#[derive(Clone, Copy)]
pub struct ToolContext<'a> {
    pub document: &'a Document,
    /// The timeline being edited (a scene or a symbol).
    pub timeline: &'a TimelineId,
    /// The current layer index.
    pub layer: usize,
    /// The playhead.
    pub frame: u32,
    pub selection: &'a Selection,
    pub settings: &'a ToolSettings,
    /// Screen points per stage pixel.
    pub zoom: f64,
    pub bounds: &'a dyn BoundsProvider,
}

impl<'a> ToolContext<'a> {
    #[must_use]
    pub fn timeline(&self) -> Option<&'a Timeline> {
        self.document.timeline(self.timeline)
    }

    #[must_use]
    pub fn layer_ref(&self, index: usize) -> Option<&'a Layer> {
        self.timeline()?.layers.get(index)
    }

    /// Hit tolerance in stage pixels.
    #[must_use]
    pub fn tolerance(&self) -> f64 {
        self.settings.hit_tolerance / self.zoom.max(1.0e-6)
    }

    /// Drag threshold in stage pixels.
    #[must_use]
    pub fn drag_threshold(&self) -> f64 {
        self.settings.drag_threshold / self.zoom.max(1.0e-6)
    }

    /// The keyframe of the current layer that covers the playhead.
    #[must_use]
    pub fn current_keyframe(&self) -> Option<u32> {
        self.layer_ref(self.layer)?.keyframe_at(self.frame).map(|f| f.index)
    }

    /// True when a layer exists, is visible, unlocked and holds drawable content (not a folder, guide or camera).
    #[must_use]
    pub fn layer_editable(&self, index: usize) -> bool {
        use anim_scene::LayerKind;
        let Some(timeline) = self.timeline() else { return false };
        let kind = timeline.resolved_kind(index);
        matches!(kind, Some(LayerKind::Normal | LayerKind::Guided | LayerKind::Masked | LayerKind::Mask))
            && timeline.is_effectively_visible(index)
            && !timeline.is_effectively_locked(index)
    }

    /// Resolves a reference to its element and parent-to-stage transform.
    #[must_use]
    pub fn resolve(&self, r: &ElementRef) -> Option<Resolved<'a>> {
        resolve_ref(self.document, self.timeline, r)
    }

    /// Bounds of an element in the coordinates of its parent (its own matrix applied).
    #[must_use]
    pub fn bounds_in_parent(&self, element: &Element) -> Option<Rect> {
        crate::hit::bounds_in_parent(self.bounds, element, 0)
    }

    /// Bounds of the referenced element on the stage.
    #[must_use]
    pub fn stage_bounds(&self, r: &ElementRef) -> Option<Rect> {
        let resolved = self.resolve(r)?;
        crate::hit::transform_rect(resolved.parent_to_stage, self.bounds_in_parent(resolved.element)?)
    }

    /// Union of the stage bounds of the selected elements.
    #[must_use]
    pub fn selection_bounds(&self) -> Option<Rect> {
        self.selection.items.iter().filter_map(|r| self.stage_bounds(r)).reduce(|a, b| a.union(b))
    }
}
