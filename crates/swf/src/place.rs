//! Display list tags: `PlaceObject3` and `RemoveObject2`.

use anim_core::BlendMode;
use anim_display::Filter;

use crate::bits::PutBytes;
use crate::error::SwfError;
use crate::filter::write_filter_list;
use crate::types::{CharacterId, CxForm, Matrix};

/// The SWF blend mode code for `mode`, and whether SWF has an exact equivalent.
///
/// SWF has the Flash set (normal, layer, multiply, screen, lighten, darken, difference, add, subtract, invert,
/// alpha, erase, overlay, hard light). The Photoshop modes it lacks are mapped to the nearest look.
#[must_use]
pub const fn blend_code(mode: BlendMode) -> (u8, bool) {
    match mode {
        BlendMode::Normal | BlendMode::PassThrough => (1, true),
        BlendMode::Layer => (2, true),
        BlendMode::Multiply => (3, true),
        BlendMode::Screen => (4, true),
        BlendMode::Lighten => (5, true),
        BlendMode::Darken => (6, true),
        BlendMode::Difference => (7, true),
        BlendMode::Add => (8, true),
        BlendMode::Subtract => (9, true),
        BlendMode::Invert => (10, true),
        BlendMode::Alpha => (11, true),
        BlendMode::Erase => (12, true),
        BlendMode::Overlay => (13, true),
        BlendMode::HardLight => (14, true),
        BlendMode::ColorDodge => (4, false),
        BlendMode::ColorBurn => (3, false),
        BlendMode::SoftLight => (13, false),
        BlendMode::Exclusion => (7, false),
        BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => (1, false),
    }
}

/// One `PlaceObject3`. Fields left `None` are not written; on a move they keep their previous value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Placement {
    pub depth: u16,
    pub character: Option<CharacterId>,
    /// Modify the object already at `depth` instead of placing a new one.
    pub is_move: bool,
    pub matrix: Option<Matrix>,
    pub color: Option<CxForm>,
    /// Morph ratio (0 ..= 65535) for morph shapes.
    pub ratio: Option<u16>,
    /// Instance name.
    pub name: Option<String>,
    /// Makes this object a mask for the objects above it up to and including this depth.
    pub clip_depth: Option<u16>,
    pub filters: Option<Vec<Filter>>,
    pub blend: Option<BlendMode>,
    pub cache_as_bitmap: Option<bool>,
    pub visible: Option<bool>,
}

impl Placement {
    /// A fresh placement of `character` at `depth` with `matrix`.
    #[must_use]
    pub fn new(depth: u16, character: CharacterId, matrix: Matrix) -> Self {
        Self { depth, character: Some(character), matrix: Some(matrix), ..Self::default() }
    }

    /// The `PlaceObject3` tag body.
    ///
    /// # Errors
    /// [`SwfError::InteriorNul`] for a name with a NUL, [`SwfError::TooManyFilters`] past 255 filters.
    pub fn encode(&self) -> Result<Vec<u8>, SwfError> {
        let mut out = Vec::with_capacity(32);
        let flags1 = u8::from(self.clip_depth.is_some()).wrapping_shl(6)
            | u8::from(self.name.is_some()).wrapping_shl(5)
            | u8::from(self.ratio.is_some()).wrapping_shl(4)
            | u8::from(self.color.is_some()).wrapping_shl(3)
            | u8::from(self.matrix.is_some()).wrapping_shl(2)
            | u8::from(self.character.is_some()).wrapping_shl(1)
            | u8::from(self.is_move);
        let flags2 = u8::from(self.visible.is_some()).wrapping_shl(5)
            | u8::from(self.cache_as_bitmap.is_some()).wrapping_shl(2)
            | u8::from(self.blend.is_some()).wrapping_shl(1)
            | u8::from(self.filters.is_some());
        out.put_u8(flags1);
        out.put_u8(flags2);
        out.put_u16(self.depth);
        if let Some(character) = self.character {
            out.put_u16(character.0);
        }
        if let Some(matrix) = &self.matrix {
            matrix.write(&mut out);
        }
        if let Some(color) = &self.color {
            color.write(&mut out);
        }
        if let Some(ratio) = self.ratio {
            out.put_u16(ratio);
        }
        if let Some(name) = &self.name {
            out.put_cstr(name)?;
        }
        if let Some(clip_depth) = self.clip_depth {
            out.put_u16(clip_depth);
        }
        if let Some(filters) = &self.filters {
            write_filter_list(&mut out, filters)?;
        }
        if let Some(blend) = self.blend {
            out.put_u8(blend_code(blend).0);
        }
        if let Some(cache) = self.cache_as_bitmap {
            out.put_u8(u8::from(cache));
        }
        if let Some(visible) = self.visible {
            out.put_u8(u8::from(visible));
        }
        Ok(out)
    }
}

/// The `RemoveObject2` tag body.
#[must_use]
pub fn remove_object2_body(depth: u16) -> Vec<u8> {
    depth.to_le_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_placement() {
        let body = Placement::new(3, CharacterId(7), Matrix::IDENTITY).encode().unwrap();
        // flags: has matrix + has character
        assert_eq!(&body[..6], &[0b0000_0110, 0, 3, 0, 7, 0]);
    }

    #[test]
    fn move_with_everything_is_ordered_per_the_spec() {
        let placement = Placement {
            depth: 1,
            character: None,
            is_move: true,
            matrix: Some(Matrix::IDENTITY),
            color: Some(CxForm::IDENTITY),
            ratio: Some(0x1234),
            name: Some("a".into()),
            clip_depth: Some(9),
            filters: Some(vec![]),
            blend: Some(BlendMode::Multiply),
            cache_as_bitmap: Some(true),
            visible: Some(false),
        };
        let body = placement.encode().unwrap();
        assert_eq!(body[0], 0b0111_1101);
        assert_eq!(body[1], 0b0010_0111);
        // depth, matrix (2 bytes), cxform (1 byte), ratio, name, clip depth, filters (count), blend, cache, visible
        let tail = &body[2..];
        assert_eq!(&tail[..2], &[1, 0]);
        assert_eq!(&tail[2..4], &[0b0000_0010, 0]);
        assert_eq!(tail[4], 0b0000_0100);
        assert_eq!(&tail[5..7], &[0x34, 0x12]);
        assert_eq!(&tail[7..9], &[b'a', 0]);
        assert_eq!(&tail[9..11], &[9, 0]);
        assert_eq!(&tail[11..], &[0, 3, 1, 0]);
    }

    #[test]
    fn blend_codes() {
        assert_eq!(blend_code(BlendMode::Normal), (1, true));
        assert_eq!(blend_code(BlendMode::HardLight), (14, true));
        assert!(!blend_code(BlendMode::Hue).1);
        assert!(!blend_code(BlendMode::SoftLight).1);
    }

    #[test]
    fn names_with_nul_are_rejected() {
        let placement = Placement { name: Some("a\0b".into()), ..Placement::default() };
        assert!(matches!(placement.encode(), Err(SwfError::InteriorNul)));
    }
}
