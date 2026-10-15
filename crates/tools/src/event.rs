//! Input events the tools consume. Everything is plain serde data so an event script can be written as JSON
//! (see [`crate::script`]).

use anim_core::Point;
use serde::{Deserialize, Serialize};

/// Keyboard modifiers. `ctrl` is Control on Windows and Linux and Command on macOS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Self = Self { shift: false, ctrl: false, alt: false };
    pub const SHIFT: Self = Self { shift: true, ctrl: false, alt: false };
    pub const CTRL: Self = Self { shift: false, ctrl: true, alt: false };
    pub const ALT: Self = Self { shift: false, ctrl: false, alt: true };
}

/// What the pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    /// The primary button went down.
    Down,
    /// The pointer moved with the button down.
    Move,
    /// The primary button went up.
    Up,
    /// The pointer moved with no button down.
    Hover,
}

const fn one_f32() -> f32 {
    1.0
}

const fn one_u8() -> u8 {
    1
}

/// A pointer event. Positions are stage pixels (y down).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerEvent {
    pub kind: PointerKind,
    /// Stage position.
    pub pos: Point,
    /// Position on screen in points, for tools whose drag must not depend on the view (hand). Falls back to `pos`.
    #[serde(default)]
    pub screen: Option<Point>,
    #[serde(default)]
    pub modifiers: Modifiers,
    /// Pen pressure 0..=1; 1.0 for a mouse.
    #[serde(default = "one_f32")]
    pub pressure: f32,
    /// 1 for a single click, 2 for the second press of a double click.
    #[serde(default = "one_u8")]
    pub click_count: u8,
}

impl PointerEvent {
    #[must_use]
    pub const fn new(kind: PointerKind, x: f64, y: f64) -> Self {
        Self { kind, pos: Point::new(x, y), screen: None, modifiers: Modifiers::NONE, pressure: 1.0, click_count: 1 }
    }

    #[must_use]
    pub const fn down(x: f64, y: f64) -> Self {
        Self::new(PointerKind::Down, x, y)
    }

    #[must_use]
    pub const fn drag(x: f64, y: f64) -> Self {
        Self::new(PointerKind::Move, x, y)
    }

    #[must_use]
    pub const fn up(x: f64, y: f64) -> Self {
        Self::new(PointerKind::Up, x, y)
    }

    #[must_use]
    pub const fn hover(x: f64, y: f64) -> Self {
        Self::new(PointerKind::Hover, x, y)
    }

    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    #[must_use]
    pub const fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = pressure;
        self
    }

    #[must_use]
    pub const fn with_click_count(mut self, click_count: u8) -> Self {
        self.click_count = click_count;
        self
    }

    #[must_use]
    pub const fn with_screen(mut self, x: f64, y: f64) -> Self {
        self.screen = Some(Point::new(x, y));
        self
    }

    /// The screen position, or the stage position when the host did not supply one.
    #[must_use]
    pub fn screen_pos(&self) -> Point {
        self.screen.unwrap_or(self.pos)
    }
}

/// A key the tools care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Escape,
    Enter,
    Backspace,
    Delete,
    Tab,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// A character key (lower case for letters), used for shortcuts such as `a` in Ctrl+A.
    Char(char),
}

const fn yes() -> bool {
    true
}

/// A key press or release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: Key,
    #[serde(default)]
    pub modifiers: Modifiers,
    /// False for a release (only Space release matters: it ends the temporary hand tool).
    #[serde(default = "yes")]
    pub pressed: bool,
}

impl KeyEvent {
    #[must_use]
    pub const fn press(key: Key) -> Self {
        Self { key, modifiers: Modifiers::NONE, pressed: true }
    }

    #[must_use]
    pub const fn release(key: Key) -> Self {
        Self { key, modifiers: Modifiers::NONE, pressed: false }
    }

    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }
}

/// Anything a tool can be fed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolEvent {
    Pointer(PointerEvent),
    Key(KeyEvent),
    /// Committed text input (typed characters, an IME commit, a paste). Logical order; right-to-left text needs no
    /// special handling.
    Text {
        text: String,
    },
    /// The host resolved a caret position with the text layout (a click, or visual arrow-key motion): byte offsets
    /// into the field's text. `anchor` differs from `offset` for a selection.
    SetCaret {
        offset: usize,
        anchor: usize,
    },
    /// Abort whatever gesture is in progress.
    Cancel,
}

impl From<PointerEvent> for ToolEvent {
    fn from(e: PointerEvent) -> Self {
        Self::Pointer(e)
    }
}

impl From<KeyEvent> for ToolEvent {
    fn from(e: KeyEvent) -> Self {
        Self::Key(e)
    }
}
