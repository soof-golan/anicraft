//! Buttons: `DefineButton2`.
//!
//! A button is a list of records, each placing a character for some of the four states (up, over, down, hit test),
//! plus optional scripts that run on state transitions or key presses. Animate's button symbols have a four frame
//! timeline (up, over, down, hit); the usual export defines each frame's content as a sprite and gives the button one
//! record per state.

use anim_core::BlendMode;
use anim_display::Filter;

use crate::bits::PutBytes;
use crate::error::SwfError;
use crate::filter::write_filter_list;
use crate::place::blend_code;
use crate::types::{CharacterId, CxForm, Matrix};

/// Transition bits of a [`ButtonAction`]'s condition field.
pub mod condition {
    pub const IDLE_TO_OVER_UP: u16 = 1 << 0;
    pub const OVER_UP_TO_IDLE: u16 = 1 << 1;
    pub const OVER_UP_TO_OVER_DOWN: u16 = 1 << 2;
    /// The mouse button was released over the button: the usual "click".
    pub const OVER_DOWN_TO_OVER_UP: u16 = 1 << 3;
    pub const OVER_DOWN_TO_OUT_DOWN: u16 = 1 << 4;
    pub const OUT_DOWN_TO_OVER_DOWN: u16 = 1 << 5;
    pub const OUT_DOWN_TO_IDLE: u16 = 1 << 6;
    pub const IDLE_TO_OVER_DOWN: u16 = 1 << 7;
    pub const OVER_DOWN_TO_IDLE: u16 = 1 << 8;

    /// The condition for a key press (key codes 1 ..= 127).
    #[must_use]
    pub fn key_press(code: u8) -> u16 {
        u16::from(code & 0x7f).wrapping_shl(9)
    }
}

/// Which button states a record is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(clippy::struct_excessive_bools, reason = "one flag per button state, as in the file")]
pub struct ButtonStates {
    pub up: bool,
    pub over: bool,
    pub down: bool,
    /// The area that reacts to the mouse.
    pub hit: bool,
}

impl ButtonStates {
    /// All four states.
    pub const ALL: Self = Self { up: true, over: true, down: true, hit: true };
    pub const UP: Self = Self { up: true, over: false, down: false, hit: false };
    pub const OVER: Self = Self { up: false, over: true, down: false, hit: false };
    pub const DOWN: Self = Self { up: false, over: false, down: true, hit: false };
    pub const HIT: Self = Self { up: false, over: false, down: false, hit: true };
}

/// One character placed in one or more button states.
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonRecord {
    pub states: ButtonStates,
    pub character: CharacterId,
    pub depth: u16,
    pub matrix: Matrix,
    pub color: CxForm,
    pub filters: Vec<Filter>,
    pub blend: Option<BlendMode>,
}

impl ButtonRecord {
    /// A plain record: `character` at the origin, no colour transform, filters or blend mode.
    #[must_use]
    pub const fn new(states: ButtonStates, character: CharacterId, depth: u16) -> Self {
        Self { states, character, depth, matrix: Matrix::IDENTITY, color: CxForm::IDENTITY, filters: Vec::new(), blend: None }
    }
}

/// A script that runs when one of the transitions in `conditions` happens (see [`condition`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonAction {
    pub conditions: u16,
    /// Script bytecode for the first virtual machine, ending with a zero byte.
    pub bytecode: Vec<u8>,
}

/// A button.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ButtonDef {
    /// Behave like a menu item: a press that starts outside and is dragged in highlights it.
    pub track_as_menu: bool,
    pub records: Vec<ButtonRecord>,
    pub actions: Vec<ButtonAction>,
}

impl ButtonDef {
    /// The `DefineButton2` body without the leading character id.
    ///
    /// # Errors
    /// [`SwfError::TooLarge`] for records or scripts too long for the 16-bit offsets, and the errors of the filter
    /// list.
    pub fn encode(&self) -> Result<Vec<u8>, SwfError> {
        let mut records = Vec::new();
        for record in &self.records {
            let flags = u8::from(record.blend.is_some()).wrapping_shl(5)
                | u8::from(!record.filters.is_empty()).wrapping_shl(4)
                | u8::from(record.states.hit).wrapping_shl(3)
                | u8::from(record.states.down).wrapping_shl(2)
                | u8::from(record.states.over).wrapping_shl(1)
                | u8::from(record.states.up);
            records.put_u8(flags);
            records.put_u16(record.character.0);
            records.put_u16(record.depth);
            record.matrix.write(&mut records);
            record.color.write(&mut records);
            if !record.filters.is_empty() {
                write_filter_list(&mut records, &record.filters)?;
            }
            if let Some(blend) = record.blend {
                records.put_u8(blend_code(blend).0);
            }
        }
        records.put_u8(0);

        let mut actions = Vec::new();
        for (index, action) in self.actions.iter().enumerate() {
            let size = action.bytecode.len().saturating_add(4);
            let last = index.saturating_add(1) == self.actions.len();
            actions.put_u16(if last { 0 } else { u16::try_from(size).map_err(|_| SwfError::TooLarge(size))? });
            actions.put_u16(action.conditions);
            actions.extend_from_slice(&action.bytecode);
        }

        let mut out = Vec::with_capacity(records.len().saturating_add(actions.len()).saturating_add(3));
        out.put_u8(u8::from(self.track_as_menu));
        // the offset counts from the start of this field to the first action
        let offset = if actions.is_empty() { 0 } else { u16::try_from(records.len().saturating_add(2)).map_err(|_| SwfError::TooLarge(records.len()))? };
        out.put_u16(offset);
        out.extend_from_slice(&records);
        out.extend_from_slice(&actions);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movie::ACTION_STOP;

    #[test]
    fn layout_without_actions() {
        let def = ButtonDef { track_as_menu: true, records: vec![ButtonRecord::new(ButtonStates::UP, CharacterId(5), 1)], actions: vec![] };
        let body = def.encode().unwrap();
        // flags, offset 0, record (state flags, id, depth, matrix 2 bytes, cxform 1 byte), end
        assert_eq!(body, vec![1, 0, 0, 0b0000_0001, 5, 0, 1, 0, 0b0000_0010, 0, 0b0000_0100, 0]);
    }

    #[test]
    fn actions_get_sizes_and_the_offset() {
        let def = ButtonDef {
            track_as_menu: false,
            records: vec![ButtonRecord::new(ButtonStates::ALL, CharacterId(1), 1)],
            actions: vec![
                ButtonAction { conditions: condition::OVER_DOWN_TO_OVER_UP, bytecode: ACTION_STOP.to_vec() },
                ButtonAction { conditions: condition::key_press(13), bytecode: vec![0] },
            ],
        };
        let body = def.encode().unwrap();
        let offset = u16::from_le_bytes([body[1], body[2]]);
        // record 8 bytes + end byte + the offset field itself
        assert_eq!(offset, 11);
        let first = &body[usize::from(offset) + 1..];
        // the size covers the whole action: 2 bytes of size, 2 of conditions and 2 of script
        assert_eq!(&first[..4], &[6, 0, 8, 0]);
        assert_eq!(&first[4..6], &ACTION_STOP);
        // the last action has size 0 and the key press condition
        assert_eq!(&first[6..10], &[0, 0, 0, 26]);
    }

    #[test]
    fn filters_and_blend_set_flag_bits() {
        let mut record = ButtonRecord::new(ButtonStates::OVER, CharacterId(2), 3);
        record.filters = vec![Filter::Blur { x: 1.0, y: 1.0, quality: 1 }];
        record.blend = Some(BlendMode::Multiply);
        let body = ButtonDef { records: vec![record], ..ButtonDef::default() }.encode().unwrap();
        assert_eq!(body[3], 0b0011_0010);
        assert_eq!(*body.last().unwrap(), 0);
    }
}
