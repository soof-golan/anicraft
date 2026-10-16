//! Locking single objects (Modify > Arrange > Lock, Unlock All).
//!
//! A locked object is drawn but cannot be picked, selected or painted. Animate stores the state as the `locked`
//! attribute of the object, which the model keeps in the element's extra attributes, so it round-trips in the file.

use anim_scene::{Element, Extra};

const ATTRIBUTE: &str = "locked";

/// The extra attributes of an element, `None` for elements the model does not know.
#[must_use]
pub const fn extra_of(element: &Element) -> Option<&Extra> {
    match element {
        Element::Shape(s) => Some(&s.extra),
        Element::SymbolInstance(s) => Some(&s.extra),
        Element::Bitmap(b) => Some(&b.extra),
        Element::Text(t) => Some(&t.extra),
        Element::Group(g) => Some(&g.extra),
        Element::Unknown(_) => None,
    }
}

/// Mutable access to [`extra_of`].
pub const fn extra_of_mut(element: &mut Element) -> Option<&mut Extra> {
    match element {
        Element::Shape(s) => Some(&mut s.extra),
        Element::SymbolInstance(s) => Some(&mut s.extra),
        Element::Bitmap(b) => Some(&mut b.extra),
        Element::Text(t) => Some(&mut t.extra),
        Element::Group(g) => Some(&mut g.extra),
        Element::Unknown(_) => None,
    }
}

/// True when the object is locked.
#[must_use]
pub fn is_locked(element: &Element) -> bool {
    extra_of(element).and_then(|e| e.attr(ATTRIBUTE)).is_some_and(|v| v == "true")
}

/// Locks or unlocks an object; returns whether the object's state changed.
pub fn set_locked(element: &mut Element, locked: bool) -> bool {
    let was = is_locked(element);
    let Some(extra) = extra_of_mut(element) else { return false };
    if locked {
        extra.set_attr(ATTRIBUTE, "true");
    } else {
        extra.remove_attr(ATTRIBUTE);
    }
    was != locked
}

#[cfg(test)]
mod tests {
    use anim_core::Affine;
    use anim_scene::BitmapInstance;

    use super::*;

    #[test]
    fn locking_round_trips_through_the_extra_attributes() {
        let mut e = Element::Bitmap(BitmapInstance::new("b", Affine::IDENTITY));
        assert!(!is_locked(&e));
        assert!(set_locked(&mut e, true));
        assert!(is_locked(&e));
        assert!(!set_locked(&mut e, true), "already locked");
        assert!(set_locked(&mut e, false));
        assert!(!is_locked(&e));
    }
}
