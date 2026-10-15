//! Addresses of elements on the stage and the selection.

use serde::{Deserialize, Serialize};

/// Points at one element of a keyframe, possibly inside groups.
///
/// `keyframe` is the frame number of the keyframe that holds the element (not the playhead). `path` is the index of
/// the element in the keyframe's element list followed by the indices of the group members down to the element;
/// it is never empty for a valid reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ElementRef {
    pub layer: usize,
    pub keyframe: u32,
    pub path: Vec<usize>,
}

impl ElementRef {
    /// An element directly in a keyframe.
    #[must_use]
    pub fn top_level(layer: usize, keyframe: u32, index: usize) -> Self {
        Self { layer, keyframe, path: vec![index] }
    }

    /// The reference to a member of the group this reference points at.
    #[must_use]
    pub fn child(&self, index: usize) -> Self {
        let mut path = self.path.clone();
        path.push(index);
        Self { layer: self.layer, keyframe: self.keyframe, path }
    }

    /// The group that contains this element, or `None` for a top-level element.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let mut path = self.path.clone();
        path.pop()?;
        if path.is_empty() { None } else { Some(Self { layer: self.layer, keyframe: self.keyframe, path }) }
    }

    /// The index of the element in its container (keyframe or group).
    #[must_use]
    pub fn index(&self) -> Option<usize> {
        self.path.last().copied()
    }

    /// True when `self` is `ancestor` or lies inside it.
    #[must_use]
    pub fn is_within(&self, ancestor: &Self) -> bool {
        self.layer == ancestor.layer && self.keyframe == ancestor.keyframe && self.path.starts_with(&ancestor.path)
    }
}

/// What is selected on the stage.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Selection {
    pub items: Vec<ElementRef>,
    /// The group being edited (entered with a double click). Picking is limited to its members and the selection
    /// tool treats them as separate objects. Symbols are entered by switching the editing timeline instead.
    pub scope: Option<ElementRef>,
}

impl Selection {
    /// An empty selection.
    #[must_use]
    pub const fn empty() -> Self {
        Self { items: Vec::new(), scope: None }
    }

    /// A selection of one element.
    #[must_use]
    pub fn one(element: ElementRef) -> Self {
        Self { items: vec![element], scope: None }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[must_use]
    pub fn contains(&self, element: &ElementRef) -> bool {
        self.items.contains(element)
    }

    /// The selection with `element` added (or removed, when it is already in).
    #[must_use]
    pub fn toggled(&self, element: &ElementRef) -> Self {
        let mut next = self.clone();
        if let Some(pos) = next.items.iter().position(|e| e == element) {
            next.items.remove(pos);
        } else {
            next.items.push(element.clone());
        }
        next
    }

    /// The same scope with a different set of items.
    #[must_use]
    pub fn with_items(&self, items: Vec<ElementRef>) -> Self {
        Self { items, scope: self.scope.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_and_parent_round_trip() {
        let top = ElementRef::top_level(1, 4, 2);
        let child = top.child(7);
        assert_eq!(child.path, vec![2, 7]);
        assert_eq!(child.parent(), Some(top.clone()));
        assert_eq!(top.parent(), None);
        assert!(child.is_within(&top));
        assert!(!top.is_within(&child));
    }

    #[test]
    fn toggle_adds_then_removes() {
        let a = ElementRef::top_level(0, 0, 0);
        let s = Selection::empty().toggled(&a);
        assert!(s.contains(&a));
        assert!(!s.toggled(&a).contains(&a));
    }
}
