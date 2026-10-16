//! Document-level commands of the File and Control menus.
//!
//! Covers the document type, the presets of the New Document dialog, built-in templates, Document Settings (units,
//! scale content, anchor, match contents) and the Control play options.
//!
//! Everything here is a pure function of the model, so the window, the headless server and the tests share it.
//! The document type, ruler units and play options are stored as attributes of `DOMDocument` (the model's `extra`),
//! so they survive a save and open and are kept by other readers that ignore them.

use anim_core::{Affine, Rect, Rgba8};
use anim_scene::edit;
use anim_scene::{Document, Element, Frame, Layer, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

/// Attribute of `DOMDocument` that names the document type.
const TYPE_ATTR: &str = "anicraftDocType";
/// Attribute of `DOMDocument` that names the ruler units.
const UNITS_ATTR: &str = "anicraftRulerUnits";
/// Animate's own attribute for "Enable Live Preview"-like playback of frame actions in the editor.
const FRAME_ACTIONS_ATTR: &str = "playOptionsPlayFrameActions";
/// Attribute that remembers "Mute Sounds".
const MUTE_ATTR: &str = "anicraftMuteSounds";
/// Attribute that remembers "Enable Simple Buttons".
const BUTTONS_ATTR: &str = "anicraftSimpleButtons";

/// Largest stage side Animate accepts, in pixels.
pub const MAX_STAGE: f64 = 16384.0;

/// The platform a document targets; it decides what Publish offers and which features the document may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocType {
    /// `ActionScript 3.0` for Flash Player.
    #[default]
    ActionScript3,
    /// `HTML5 Canvas` (`CreateJS`).
    Html5Canvas,
    /// `WebGL glTF (Standard)`.
    WebGlStandard,
    /// `WebGL glTF (Extended)`.
    WebGlExtended,
    /// `VR 360`.
    Vr360,
    /// `VR Panorama`.
    VrPanorama,
    /// `AIR for Desktop`.
    AirDesktop,
    /// `AIR for Android`.
    AirAndroid,
    /// `AIR for iOS`.
    AirIos,
}

impl DocType {
    /// Every type, in the order the New Document dialog lists them.
    pub const ALL: [Self; 9] = [
        Self::ActionScript3,
        Self::Html5Canvas,
        Self::WebGlStandard,
        Self::WebGlExtended,
        Self::Vr360,
        Self::VrPanorama,
        Self::AirDesktop,
        Self::AirAndroid,
        Self::AirIos,
    ];

    /// The id used in commands and files.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::ActionScript3 => "as3",
            Self::Html5Canvas => "html5_canvas",
            Self::WebGlStandard => "webgl_standard",
            Self::WebGlExtended => "webgl_extended",
            Self::Vr360 => "vr_360",
            Self::VrPanorama => "vr_panorama",
            Self::AirDesktop => "air_desktop",
            Self::AirAndroid => "air_android",
            Self::AirIos => "air_ios",
        }
    }

    /// The name the dialog shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ActionScript3 => "ActionScript 3.0",
            Self::Html5Canvas => "HTML5 Canvas",
            Self::WebGlStandard => "WebGL glTF (Standard)",
            Self::WebGlExtended => "WebGL glTF (Extended)",
            Self::Vr360 => "VR 360",
            Self::VrPanorama => "VR Panorama",
            Self::AirDesktop => "AIR for Desktop",
            Self::AirAndroid => "AIR for Android",
            Self::AirIos => "AIR for iOS",
        }
    }

    /// One line on what the type is for.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ActionScript3 => "Animation and interactive content for Flash Player and AIR runtimes, scripted with ActionScript 3.0.",
            Self::Html5Canvas => "Animation and interactive content for the web, published as HTML5 canvas with JavaScript.",
            Self::WebGlStandard => "Animation published as glTF 2.0 for WebGL viewers, with the standard feature set.",
            Self::WebGlExtended => "Animation published as glTF 2.0 with the extended feature set for WebGL viewers.",
            Self::Vr360 => "A 360-degree equirectangular scene, 2:1, for VR headsets and 360 players.",
            Self::VrPanorama => "A wide panorama a viewer looks around in, for VR headsets and panorama players.",
            Self::AirDesktop => "An application for Windows and macOS desktops, scripted with ActionScript 3.0.",
            Self::AirAndroid => "An application for Android devices, scripted with ActionScript 3.0.",
            Self::AirIos => "An application for iPhone and iPad, scripted with ActionScript 3.0.",
        }
    }

    /// The type with this id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// True for the types whose scripts are `ActionScript`.
    #[must_use]
    pub const fn uses_actionscript(self) -> bool {
        matches!(self, Self::ActionScript3 | Self::AirDesktop | Self::AirAndroid | Self::AirIos)
    }

    /// The publish format the type produces by default.
    #[must_use]
    pub const fn publish_target(self) -> &'static str {
        match self {
            Self::ActionScript3 => "swf",
            Self::Html5Canvas => "html5",
            Self::WebGlStandard | Self::WebGlExtended => "gltf",
            Self::Vr360 | Self::VrPanorama => "vr",
            Self::AirDesktop | Self::AirAndroid | Self::AirIos => "air",
        }
    }

    /// The stage size and frame rate a fresh document of this type starts with when no preset says otherwise.
    #[must_use]
    pub const fn default_stage(self) -> (f64, f64, f64) {
        match self {
            Self::Vr360 => (4096.0, 2048.0, 30.0),
            Self::VrPanorama => (3840.0, 1080.0, 30.0),
            Self::AirAndroid | Self::AirIos => (1080.0, 1920.0, 30.0),
            Self::AirDesktop => (1280.0, 720.0, 24.0),
            _ => (550.0, 400.0, 24.0),
        }
    }
}

/// The type a document was made as (`ActionScript` 3.0 when it says nothing, as for every file Animate wrote before
/// document types existed).
#[must_use]
pub fn doc_type(document: &Document) -> DocType {
    document.extra.attr(TYPE_ATTR).and_then(DocType::from_id).unwrap_or_default()
}

/// Records the document type.
pub fn set_doc_type(document: &mut Document, kind: DocType) {
    if kind == DocType::ActionScript3 {
        document.extra.remove_attr(TYPE_ATTR);
    } else {
        document.extra.set_attr(TYPE_ATTR, kind.id());
    }
}

/// Ruler units of the stage and of the size fields of Document Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Pixels,
    Inches,
    Points,
    Centimeters,
    Millimeters,
}

impl Units {
    /// Every unit, in the order the dialog lists them.
    pub const ALL: [Self; 5] = [Self::Pixels, Self::Inches, Self::Points, Self::Centimeters, Self::Millimeters];

    /// The id used in commands and files.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pixels => "pixels",
            Self::Inches => "inches",
            Self::Points => "points",
            Self::Centimeters => "centimeters",
            Self::Millimeters => "millimeters",
        }
    }

    /// The name the dialog shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pixels => "Pixels",
            Self::Inches => "Inches",
            Self::Points => "Points",
            Self::Centimeters => "Centimeters",
            Self::Millimeters => "Millimeters",
        }
    }

    /// The short suffix shown after a value.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Pixels => "px",
            Self::Inches => "in",
            Self::Points => "pt",
            Self::Centimeters => "cm",
            Self::Millimeters => "mm",
        }
    }

    /// The unit with this id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|u| u.id() == id)
    }

    /// Stage pixels in one unit (the stage is 72 pixels to the inch, as in Animate).
    #[must_use]
    pub const fn pixels_per_unit(self) -> f64 {
        match self {
            Self::Pixels | Self::Points => 1.0,
            Self::Inches => 72.0,
            Self::Centimeters => 72.0 / 2.54,
            Self::Millimeters => 72.0 / 25.4,
        }
    }

    /// A length in stage pixels expressed in this unit.
    #[must_use]
    pub fn from_pixels(self, pixels: f64) -> f64 {
        pixels / self.pixels_per_unit()
    }

    /// A length in this unit expressed in stage pixels.
    #[must_use]
    pub fn to_pixels(self, value: f64) -> f64 {
        value * self.pixels_per_unit()
    }
}

/// The ruler units of the document.
#[must_use]
pub fn units(document: &Document) -> Units {
    document.extra.attr(UNITS_ATTR).and_then(Units::from_id).unwrap_or_default()
}

/// Records the ruler units.
pub fn set_units(document: &mut Document, units: Units) {
    if units == Units::Pixels {
        document.extra.remove_attr(UNITS_ATTR);
    } else {
        document.extra.set_attr(UNITS_ATTR, units.id());
    }
}

/// The Control menu toggles that belong to the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayOptions {
    /// Mute Sounds: the editor plays no sound.
    pub mute_sounds: bool,
    /// Enable Simple Buttons: buttons on the stage react to the pointer.
    pub simple_buttons: bool,
    /// Enable Live Frame Actions: frame scripts of the main timeline run while playing in the editor.
    pub frame_actions: bool,
}

/// The play options of the document.
#[must_use]
pub fn play_options(document: &Document) -> PlayOptions {
    let on = |name: &str| document.extra.attr(name) == Some("true");
    PlayOptions { mute_sounds: on(MUTE_ATTR), simple_buttons: on(BUTTONS_ATTR), frame_actions: on(FRAME_ACTIONS_ATTR) }
}

/// Records the play options.
pub fn set_play_options(document: &mut Document, options: PlayOptions) {
    for (name, on) in [(MUTE_ATTR, options.mute_sounds), (BUTTONS_ATTR, options.simple_buttons), (FRAME_ACTIONS_ATTR, options.frame_actions)] {
        if on {
            document.extra.set_attr(name, "true");
        } else {
            document.extra.remove_attr(name);
        }
    }
}

/// One stage size and frame rate of the New Document dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    /// Id used in commands.
    pub id: &'static str,
    /// Name the dialog shows.
    pub name: &'static str,
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    /// The document type the preset suggests.
    pub kind: DocType,
}

/// A tab of the New Document dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Category {
    /// Id used in commands.
    pub id: &'static str,
    /// Name of the tab.
    pub name: &'static str,
    pub presets: &'static [Preset],
}

const fn preset(id: &'static str, name: &'static str, width: f64, height: f64, fps: f64, kind: DocType) -> Preset {
    Preset { id, name, width, height, fps, kind }
}

/// The categories and presets of the New Document dialog.
pub const CATEGORIES: [Category; 7] = [
    Category {
        id: "character_animation",
        name: "Character Animation",
        presets: &[
            preset("hdtv_1080", "HDTV 1080p", 1920.0, 1080.0, 24.0, DocType::ActionScript3),
            preset("hdtv_720", "HDTV 720p", 1280.0, 720.0, 24.0, DocType::ActionScript3),
            preset("film_2k", "Film 2K", 2048.0, 1080.0, 24.0, DocType::ActionScript3),
            preset("uhd_4k", "UHD 4K", 3840.0, 2160.0, 24.0, DocType::ActionScript3),
            preset("ntsc_dv", "NTSC DV", 720.0, 480.0, 29.97, DocType::ActionScript3),
            preset("pal_dv", "PAL DV", 720.0, 576.0, 25.0, DocType::ActionScript3),
        ],
    },
    Category {
        id: "social",
        name: "Social",
        presets: &[
            preset("instagram_square", "Instagram Post (square)", 1080.0, 1080.0, 30.0, DocType::Html5Canvas),
            preset("instagram_portrait", "Instagram Post (portrait)", 1080.0, 1350.0, 30.0, DocType::Html5Canvas),
            preset("story", "Story / Reel / TikTok", 1080.0, 1920.0, 30.0, DocType::Html5Canvas),
            preset("facebook_cover", "Facebook Cover", 820.0, 312.0, 24.0, DocType::Html5Canvas),
            preset("twitter_post", "X / Twitter Post", 1200.0, 675.0, 30.0, DocType::Html5Canvas),
            preset("youtube", "YouTube", 1920.0, 1080.0, 30.0, DocType::Html5Canvas),
        ],
    },
    Category {
        id: "game",
        name: "Game",
        presets: &[
            preset("game_landscape", "Landscape 1920 x 1080", 1920.0, 1080.0, 60.0, DocType::Html5Canvas),
            preset("game_portrait", "Portrait 1080 x 1920", 1080.0, 1920.0, 60.0, DocType::Html5Canvas),
            preset("game_classic", "Classic 4:3 1024 x 768", 1024.0, 768.0, 30.0, DocType::Html5Canvas),
            preset("sprite_sheet", "Sprite sheet 512 x 512", 512.0, 512.0, 24.0, DocType::Html5Canvas),
            preset("game_webgl", "WebGL scene", 1280.0, 720.0, 60.0, DocType::WebGlStandard),
        ],
    },
    Category {
        id: "education",
        name: "Education",
        presets: &[
            preset("slides_4_3", "Slides 4:3", 1024.0, 768.0, 12.0, DocType::Html5Canvas),
            preset("slides_16_9", "Slides 16:9", 1280.0, 720.0, 24.0, DocType::Html5Canvas),
            preset("classroom", "Classroom display", 1920.0, 1080.0, 24.0, DocType::Html5Canvas),
            preset("worksheet", "Worksheet (A4 at 72 dpi)", 595.0, 842.0, 12.0, DocType::Html5Canvas),
        ],
    },
    Category {
        id: "ads",
        name: "Ads",
        presets: &[
            preset("medium_rectangle", "Medium Rectangle 300 x 250", 300.0, 250.0, 24.0, DocType::Html5Canvas),
            preset("leaderboard", "Leaderboard 728 x 90", 728.0, 90.0, 24.0, DocType::Html5Canvas),
            preset("skyscraper", "Wide Skyscraper 160 x 600", 160.0, 600.0, 24.0, DocType::Html5Canvas),
            preset("half_page", "Half Page 300 x 600", 300.0, 600.0, 24.0, DocType::Html5Canvas),
            preset("billboard", "Billboard 970 x 250", 970.0, 250.0, 24.0, DocType::Html5Canvas),
            preset("mobile_banner", "Mobile Banner 320 x 50", 320.0, 50.0, 24.0, DocType::Html5Canvas),
        ],
    },
    Category {
        id: "web",
        name: "Web",
        presets: &[
            preset("web_hd", "Web 1280 x 720", 1280.0, 720.0, 24.0, DocType::Html5Canvas),
            preset("web_desktop", "Desktop 1920 x 1080", 1920.0, 1080.0, 24.0, DocType::Html5Canvas),
            preset("web_banner", "Hero banner 1200 x 628", 1200.0, 628.0, 24.0, DocType::Html5Canvas),
            preset("web_mobile", "Mobile 375 x 667", 375.0, 667.0, 24.0, DocType::Html5Canvas),
            preset("web_tablet", "Tablet 768 x 1024", 768.0, 1024.0, 24.0, DocType::Html5Canvas),
        ],
    },
    Category {
        id: "advanced",
        name: "Advanced",
        presets: &[
            preset("default", "Default 550 x 400", 550.0, 400.0, 24.0, DocType::ActionScript3),
            preset("air_desktop", "AIR for Desktop 1280 x 720", 1280.0, 720.0, 24.0, DocType::AirDesktop),
            preset("air_android", "AIR for Android 1080 x 1920", 1080.0, 1920.0, 30.0, DocType::AirAndroid),
            preset("air_ios", "AIR for iOS 1080 x 1920", 1080.0, 1920.0, 30.0, DocType::AirIos),
            preset("vr_360", "VR 360 4096 x 2048", 4096.0, 2048.0, 30.0, DocType::Vr360),
            preset("vr_panorama", "VR Panorama 3840 x 1080", 3840.0, 1080.0, 30.0, DocType::VrPanorama),
            preset("webgl_extended", "WebGL glTF (Extended) 1920 x 1080", 1920.0, 1080.0, 30.0, DocType::WebGlExtended),
        ],
    },
];

/// The preset with this id, in any category.
#[must_use]
pub fn find_preset(id: &str) -> Option<&'static Preset> {
    CATEGORIES.iter().flat_map(|c| c.presets.iter()).find(|p| p.id == id)
}

/// A built-in template: a ready-made document structure to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTemplate {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
}

/// The templates that ship with the application.
pub const BUILTIN_TEMPLATES: [BuiltinTemplate; 5] = [
    BuiltinTemplate { id: "blank", name: "Blank", category: "Basic", description: "One empty layer." },
    BuiltinTemplate {
        id: "character_scene",
        name: "Character scene",
        category: "Animation",
        description: "Background, Characters, Effects and Sound layers on a 1920 x 1080 stage at 24 fps, 48 frames long.",
    },
    BuiltinTemplate {
        id: "banner_ad",
        name: "Banner ad",
        category: "Advertising",
        description: "A 300 x 250 ad with Background, Artwork, Headline and Call to action layers and start/loop labels.",
    },
    BuiltinTemplate {
        id: "slideshow",
        name: "Slideshow",
        category: "Presentation",
        description: "Five scenes named Slide 1 to Slide 5 on a 1280 x 720 stage.",
    },
    BuiltinTemplate {
        id: "lip_sync",
        name: "Lip-sync talking head",
        category: "Animation",
        description: "Head, Mouth, Eyes and Dialogue layers, 72 frames at 24 fps, for mouth symbols and a voice track.",
    },
];

/// What a New Document asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSpec {
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    pub background: Rgba8,
    pub kind: DocType,
    pub units: Units,
}

impl Default for NewSpec {
    fn default() -> Self {
        Self { width: 550.0, height: 400.0, fps: 24.0, background: Rgba8::WHITE, kind: DocType::ActionScript3, units: Units::Pixels }
    }
}

impl NewSpec {
    /// The spec of a preset.
    #[must_use]
    pub fn from_preset(preset: &Preset) -> Self {
        Self { width: preset.width, height: preset.height, fps: preset.fps, kind: preset.kind, ..Self::default() }
    }

    /// Checks the numbers; the message says what is wrong.
    ///
    /// # Errors
    /// When a side is not in `1 ..= 16384` pixels or the frame rate is not in `0.01 ..= 1000`.
    pub fn validate(&self) -> Result<(), String> {
        let side = |v: f64, what: &str| {
            if v.is_finite() && (1.0..=MAX_STAGE).contains(&v) { Ok(()) } else { Err(format!("{what} must be between 1 and {MAX_STAGE} pixels")) }
        };
        side(self.width, "width")?;
        side(self.height, "height")?;
        if self.fps.is_finite() && (0.01..=1000.0).contains(&self.fps) { Ok(()) } else { Err("frame rate must be between 0.01 and 1000".to_owned()) }
    }
}

/// A new, empty document.
///
/// # Errors
/// When the spec is out of range.
pub fn new_document(spec: &NewSpec) -> Result<AniDoc, String> {
    spec.validate()?;
    let mut document = Document::new(spec.width.round(), spec.height.round(), spec.fps);
    document.background_color = spec.background;
    set_doc_type(&mut document, spec.kind);
    set_units(&mut document, spec.units);
    Ok(AniDoc::new(document, anim_scene::MediaStore::new(), Vec::new()))
}

fn layer(name: &str, colour: Rgba8, frames: u32) -> Layer {
    let mut layer = Layer::new(name, colour);
    layer.frames.push(Frame::new(0, frames.max(1), Vec::new()));
    layer
}

const PALETTE: [Rgba8; 6] = [
    Rgba8::new(0x4f, 0x80, 0xff, 255),
    Rgba8::new(0xff, 0x4f, 0x4f, 255),
    Rgba8::new(0x4f, 0xff, 0x4f, 255),
    Rgba8::new(0x98, 0x66, 0xff, 255),
    Rgba8::new(0xff, 0x80, 0x00, 255),
    Rgba8::new(0x00, 0xcc, 0xcc, 255),
];

fn layers(names: &[&str], frames: u32) -> Vec<Layer> {
    names.iter().enumerate().map(|(i, name)| layer(name, PALETTE.get(i.checked_rem(PALETTE.len()).unwrap_or(0)).copied().unwrap_or(Rgba8::BLACK), frames)).collect()
}

/// The document of a built-in template.
///
/// # Errors
/// When the id names no built-in template.
pub fn builtin_template(id: &str) -> Result<AniDoc, String> {
    let mut doc = match id {
        "blank" => new_document(&NewSpec::default())?,
        "character_scene" => {
            let mut doc = new_document(&NewSpec { width: 1920.0, height: 1080.0, fps: 24.0, ..NewSpec::default() })?;
            let scene = doc.editor.document.scenes.first_mut().ok_or("the new document has no scene")?;
            scene.layers = layers(&["Effects", "Characters", "Background", "Sound"], 48);
            doc
        }
        "banner_ad" => {
            let mut doc = new_document(&NewSpec::from_preset(find_preset("medium_rectangle").ok_or("preset missing")?))?;
            let scene = doc.editor.document.scenes.first_mut().ok_or("the new document has no scene")?;
            scene.layers = layers(&["Call to action", "Headline", "Artwork", "Background"], 72);
            if let Some(top) = scene.layers.first_mut()
                && let Some(frame) = top.frames.first_mut()
            {
                frame.label = Some("start".to_owned());
                frame.label_type = Some(anim_scene::timeline::LabelType::Name);
            }
            doc
        }
        "slideshow" => {
            let mut doc = new_document(&NewSpec::from_preset(find_preset("slides_16_9").ok_or("preset missing")?))?;
            let first = doc.editor.document.scenes.first_mut().ok_or("the new document has no scene")?;
            "Slide 1".clone_into(&mut first.name);
            for n in 2..=5_u32 {
                edit::add_scene(&mut doc.editor.document, &format!("Slide {n}"), usize::try_from(n.saturating_sub(1)).unwrap_or(0)).map_err(|e| e.to_string())?;
            }
            doc
        }
        "lip_sync" => {
            let mut doc = new_document(&NewSpec { width: 1280.0, height: 720.0, fps: 24.0, ..NewSpec::default() })?;
            let scene = doc.editor.document.scenes.first_mut().ok_or("the new document has no scene")?;
            scene.layers = layers(&["Dialogue", "Eyes", "Mouth", "Head"], 72);
            doc
        }
        other => return Err(format!("unknown template {other:?}")),
    };
    doc.editor.state.timeline = TimelineId::Scene(0);
    Ok(doc)
}

/// Where Document Settings anchors the stage when its size changes and content is scaled or shifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// The nine anchors in grid order (row by row).
    pub const GRID: [Self; 9] =
        [Self::TopLeft, Self::Top, Self::TopRight, Self::Left, Self::Center, Self::Right, Self::BottomLeft, Self::Bottom, Self::BottomRight];

    /// Fractions `(x, y)` of the stage width and height at the anchor point.
    #[must_use]
    pub const fn fractions(self) -> (f64, f64) {
        match self {
            Self::TopLeft => (0.0, 0.0),
            Self::Top => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::Left => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::Right => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::Bottom => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }

    /// The id used in commands.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::TopLeft => "top_left",
            Self::Top => "top",
            Self::TopRight => "top_right",
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::BottomLeft => "bottom_left",
            Self::Bottom => "bottom",
            Self::BottomRight => "bottom_right",
        }
    }
}

/// Applies `f` to every top-level element of every keyframe of the scenes (not of symbols). Group members move with
/// their group's matrix, so callers that transform content only need the top level.
pub fn for_each_scene_element_mut(document: &mut Document, mut f: impl FnMut(&mut Element)) {
    for scene in &mut document.scenes {
        for layer in &mut scene.layers {
            for frame in &mut layer.frames {
                frame.elements.iter_mut().for_each(&mut f);
            }
        }
    }
}

/// Transforms every top-level element of every scene keyframe by `by` (left-multiplied onto its matrix).
pub fn transform_scene_content(document: &mut Document, by: Affine) {
    for_each_scene_element_mut(document, |element| {
        if let Some(m) = element.matrix() {
            element.set_matrix(by * m);
        }
    });
}

/// The union of the bounds of everything the scenes show at any frame.
#[must_use]
pub fn content_bounds(document: &Document) -> Option<Rect> {
    document
        .scenes
        .iter()
        .flat_map(|scene| scene.layers.iter().filter(|l| !l.kind.is_structural()).flat_map(|l| l.frames.iter()))
        .filter_map(|frame| anim_scene::bounds::elements_bounds(frame.elements.iter(), &document.library))
        .reduce(|a, b| a.union(b))
}

/// How Document Settings changes the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resize {
    pub width: f64,
    pub height: f64,
    /// Scale the artwork by the same factor as the stage (the smaller of the width and height ratios).
    pub scale_content: bool,
    pub anchor: Anchor,
}

/// Resizes the stage.
///
/// Like Animate, the anchor decides which point of the stage stays put: the content moves so that
/// the point at the anchor of the old stage is at the anchor of the new one, and with `scale_content` the artwork also
/// grows or shrinks to fit.
pub fn resize_stage(document: &mut Document, resize: Resize) {
    let (old_w, old_h) = (document.width, document.height);
    let (fx, fy) = resize.anchor.fractions();
    let factor = if resize.scale_content && old_w > 0.0 && old_h > 0.0 { (resize.width / old_w).min(resize.height / old_h) } else { 1.0 };
    // The anchor point of the old stage, after scaling about the origin, must land on the new stage's anchor point.
    let dx = resize.width.mul_add(fx, -(old_w * fx * factor));
    let dy = resize.height.mul_add(fy, -(old_h * fy * factor));
    if (factor - 1.0).abs() > f64::EPSILON || dx.abs() > f64::EPSILON || dy.abs() > f64::EPSILON {
        transform_scene_content(document, Affine::translate((dx, dy)) * Affine::scale(factor));
    }
    document.width = resize.width;
    document.height = resize.height;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsParams {
    width: Option<f64>,
    height: Option<f64>,
    frame_rate: Option<f64>,
    background: Option<String>,
    doc_type: Option<String>,
    units: Option<String>,
    #[serde(default)]
    scale_content: bool,
    anchor: Option<String>,
}

fn settings(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SettingsParams = parse_params(params)?;
    let document = &mut doc.editor.document;
    let anchor = match p.anchor.as_deref() {
        None => Anchor::default(),
        Some(id) => Anchor::GRID.into_iter().find(|a| a.id() == id).ok_or_else(|| CommandError::invalid_params(format!("unknown anchor {id:?}")))?,
    };
    let spec = NewSpec {
        width: p.width.unwrap_or(document.width),
        height: p.height.unwrap_or(document.height),
        fps: p.frame_rate.unwrap_or(document.frame_rate),
        ..NewSpec::default()
    };
    spec.validate().map_err(CommandError::invalid_params)?;
    if let Some(id) = &p.doc_type {
        set_doc_type(document, DocType::from_id(id).ok_or_else(|| CommandError::invalid_params(format!("unknown document type {id:?}")))?);
    }
    if let Some(id) = &p.units {
        set_units(document, Units::from_id(id).ok_or_else(|| CommandError::invalid_params(format!("unknown units {id:?}")))?);
    }
    if let Some(text) = &p.background {
        document.background_color = super::commands::parse_color(text)?;
    }
    document.frame_rate = spec.fps;
    resize_stage(document, Resize { width: spec.width, height: spec.height, scale_content: p.scale_content, anchor });
    Ok(json!({ "width": document.width, "height": document.height, "frame_rate": document.frame_rate, "doc_type": doc_type(document).id(), "units": units(document).id() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetParams {
    preset: String,
    #[serde(default)]
    scale_content: bool,
    anchor: Option<String>,
}

fn apply_preset(doc: &mut AniDoc, params: &Value) -> Out {
    let p: PresetParams = parse_params(params)?;
    let preset = find_preset(&p.preset).ok_or_else(|| CommandError::invalid_params(format!("unknown preset {:?}; see document_presets", p.preset)))?;
    let mut forward = json!({ "width": preset.width, "height": preset.height, "frame_rate": preset.fps, "doc_type": preset.kind.id(), "scale_content": p.scale_content });
    if let (Some(anchor), Some(map)) = (&p.anchor, forward.as_object_mut()) {
        map.insert("anchor".to_owned(), json!(anchor));
    }
    settings(doc, &forward)
}

fn match_contents(doc: &mut AniDoc, _params: &Value) -> Out {
    let document = &mut doc.editor.document;
    let bounds = content_bounds(document).ok_or_else(|| CommandError::failed("the document has no content to match"))?;
    let (w, h) = (bounds.width().ceil().max(1.0), bounds.height().ceil().max(1.0));
    if w > MAX_STAGE || h > MAX_STAGE {
        return Err(CommandError::failed("the content is larger than the largest stage"));
    }
    transform_scene_content(document, Affine::translate((-bounds.x0, -bounds.y0)));
    document.width = w;
    document.height = h;
    Ok(json!({ "width": w, "height": h }))
}

/// What Save and Compact reclaims: media no library item uses any more (kept alive only by deleted items and undo
/// snapshots) and the archive caches Animate rebuilds. Returns the bytes freed.
pub fn compact(doc: &mut AniDoc) -> u64 {
    let keep: Vec<anim_scene::MediaId> = doc
        .editor
        .document
        .library
        .items
        .iter()
        .filter_map(|item| match item {
            anim_scene::LibraryItem::Bitmap(b) => b.media,
            anim_scene::LibraryItem::Sound(s) => s.media,
            anim_scene::LibraryItem::Video(v) => v.media,
            _ => None,
        })
        .collect();
    let mut freed = doc.media.retain_only(&keep);
    doc.sidecars.retain(|s| {
        let cache = std::path::Path::new(&s.name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cache"));
        if cache {
            freed = freed.saturating_add(u64::try_from(s.data.len()).unwrap_or(u64::MAX));
        }
        !cache
    });
    freed
}

#[expect(clippy::unnecessary_wraps, reason = "the command table takes one signature")]
fn compact_command(doc: &mut AniDoc, _params: &Value) -> Out {
    let freed = compact(doc);
    Ok(json!({ "freed_bytes": freed }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayParams {
    mute_sounds: Option<bool>,
    simple_buttons: Option<bool>,
    frame_actions: Option<bool>,
}

fn play_options_command(doc: &mut AniDoc, params: &Value) -> Out {
    let p: PlayParams = parse_params(params)?;
    let mut options = play_options(&doc.editor.document);
    options.mute_sounds = p.mute_sounds.unwrap_or(options.mute_sounds);
    options.simple_buttons = p.simple_buttons.unwrap_or(options.simple_buttons);
    options.frame_actions = p.frame_actions.unwrap_or(options.frame_actions);
    set_play_options(&mut doc.editor.document, options);
    Ok(serde_json::to_value(options).unwrap_or(Value::Null))
}

#[expect(clippy::unnecessary_wraps, reason = "the command table takes one signature")]
fn describe(_doc: &mut AniDoc, _params: &Value) -> Out {
    let categories: Vec<Value> = CATEGORIES
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "presets": c.presets.iter().map(|p| json!({ "id": p.id, "name": p.name, "width": p.width, "height": p.height, "frame_rate": p.fps, "doc_type": p.kind.id() })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let types: Vec<Value> = DocType::ALL.iter().map(|t| json!({ "id": t.id(), "name": t.label(), "description": t.description(), "publishes": t.publish_target() })).collect();
    let templates: Vec<Value> = BUILTIN_TEMPLATES.iter().map(|t| json!({ "id": t.id, "name": t.name, "category": t.category, "description": t.description })).collect();
    Ok(json!({ "categories": categories, "document_types": types, "units": Units::ALL.map(Units::id), "builtin_templates": templates }))
}

/// Registers the commands of this module.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry
        .register_all(vec![
            CommandSpec::new("document.settings", "Document Settings", settings)
                .in_menu(&["Modify"])
                .describe("Document Settings: stage size, frame rate, background, document type (as3, html5_canvas, webgl_standard, webgl_extended, vr_360, vr_panorama, air_desktop, air_android, air_ios), ruler units (pixels, inches, points, centimeters, millimeters), and whether the artwork is scaled with the stage (`scale_content`) about an `anchor` (top_left ... bottom_right). Omitted fields stay as they are.")
                .with_schema(r##"{"type":"object","additionalProperties":false,"properties":{"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0},"frame_rate":{"type":"number","exclusiveMinimum":0,"maximum":1000},"background":{"type":"string","description":"#rrggbb"},"doc_type":{"type":"string"},"units":{"type":"string"},"scale_content":{"type":"boolean"},"anchor":{"type":"string"}}}"##),
            CommandSpec::new("document.apply_preset", "Apply Document Preset", apply_preset)
                .describe("Gives the document the size, frame rate and type of a New Document preset (see document_describe for the ids).")
                .with_schema(r#"{"type":"object","additionalProperties":false,"required":["preset"],"properties":{"preset":{"type":"string"},"scale_content":{"type":"boolean"},"anchor":{"type":"string"}}}"#),
            CommandSpec::new("document.match_contents", "Match Contents", match_contents)
                .in_menu(&["Modify"])
                .describe("Sets the stage to the bounds of all the artwork in the scenes and moves the artwork to the top left corner."),
            CommandSpec::new("document.compact", "Compact Document", compact_command)
                .describe("Save and Compact's first half: drops media that no library item uses (left behind by deleted items) and the archive caches Animate rebuilds. Save afterwards."),
            CommandSpec::new("document.describe", "Describe New Document Options", describe)
                .transient()
                .describe("Lists the New Document presets by category, the document types, the ruler units and the built-in templates."),
            CommandSpec::new("control.play_options", "Play Options", play_options_command)
                .in_menu(&["Control"])
                .describe("The Control menu toggles: mute_sounds (the editor plays no sound), simple_buttons (buttons react to the pointer on the stage) and frame_actions (frame scripts run while playing in the editor). Omitted fields stay as they are.")
                .with_schema(r#"{"type":"object","additionalProperties":false,"properties":{"mute_sounds":{"type":"boolean"},"simple_buttons":{"type":"boolean"},"frame_actions":{"type":"boolean"}}}"#),
        ])
        .expect("document file command ids are unique");
}

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), super::super::registry())
    }

    fn rect(doc: &mut AniDoc, (left, top): (f64, f64), (width, height): (f64, f64)) {
        let mut s = Session::new(std::mem::take(doc), super::super::registry());
        s.execute("draw.rectangle", &json!({ "x": left, "y": top, "width": width, "height": height, "fill": "#ff0000", "object_drawing": true })).unwrap();
        *doc = s.doc().clone();
    }

    #[test]
    fn document_type_and_units_round_trip_through_the_xfl_writer() {
        let mut doc = new_document(&NewSpec { kind: DocType::Html5Canvas, units: Units::Centimeters, ..NewSpec::default() }).unwrap();
        assert_eq!(doc_type(doc.document()), DocType::Html5Canvas);
        let (bytes, _) = anicraft_xfl::write_fla_bytes(doc.document(), &doc.media, &doc.sidecars, &anicraft_xfl::WriteOptions::default()).unwrap();
        let loaded = anicraft_xfl::read_bytes(bytes, &anicraft_xfl::ReadOptions::default()).unwrap();
        assert_eq!(doc_type(&loaded.document), DocType::Html5Canvas);
        assert_eq!(units(&loaded.document), Units::Centimeters);
        set_doc_type(doc.document_mut(), DocType::ActionScript3);
        assert!(doc.document().extra.attr(TYPE_ATTR).is_none(), "the default type is not written");
    }

    #[test]
    fn every_preset_is_valid_and_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for preset in CATEGORIES.iter().flat_map(|c| c.presets.iter()) {
            assert!(seen.insert(preset.id), "duplicate preset id {}", preset.id);
            NewSpec::from_preset(preset).validate().unwrap();
            assert!(new_document(&NewSpec::from_preset(preset)).is_ok());
        }
        assert_eq!(CATEGORIES.map(|c| c.name), ["Character Animation", "Social", "Game", "Education", "Ads", "Web", "Advanced"]);
        for kind in DocType::ALL {
            assert_eq!(DocType::from_id(kind.id()), Some(kind));
        }
    }

    #[test]
    fn builtin_templates_build() {
        for t in BUILTIN_TEMPLATES {
            let doc = builtin_template(t.id).unwrap();
            assert!(!doc.document().scenes.is_empty(), "{}", t.id);
        }
        assert_eq!(builtin_template("slideshow").unwrap().document().scenes.len(), 5);
        assert_eq!(builtin_template("banner_ad").unwrap().document().scenes[0].layers.len(), 4);
        assert!(builtin_template("nope").is_err());
        assert!(new_document(&NewSpec { width: 0.0, ..NewSpec::default() }).is_err());
    }

    #[test]
    fn units_convert() {
        assert!((Units::Inches.from_pixels(144.0) - 2.0).abs() < 1e-9);
        assert!((Units::Centimeters.to_pixels(2.54) - 72.0).abs() < 1e-9);
        assert!((Units::Points.to_pixels(10.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn settings_scale_content_about_the_anchor() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 110, "height": 80, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("document.settings", &json!({ "width": 1100, "height": 800, "scale_content": true, "anchor": "top_left" })).unwrap();
        let b = content_bounds(s.doc().document()).unwrap();
        // The document doubled (550 x 400 to 1100 x 800), so the 110 x 80 rectangle becomes 220 x 160 plus its stroke.
        assert!((b.width() - 220.0).abs() < 3.0 && (b.height() - 160.0).abs() < 3.0, "{b:?}");
        assert!(b.x0.abs() < 3.0 && b.y0.abs() < 3.0);
        s.undo();
        let b = content_bounds(s.doc().document()).unwrap();
        assert!((b.width() - 110.0).abs() < 3.0);
        assert!((s.doc().document().width - 550.0).abs() < f64::EPSILON);
    }

    #[test]
    fn settings_without_scaling_keep_the_anchor_point_fixed() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 100, "y": 100, "width": 50, "height": 50, "fill": "#00ff00", "object_drawing": true })).unwrap();
        // Centre anchor: growing the stage by 100 in each direction moves content by 50.
        s.execute("document.settings", &json!({ "width": 650, "height": 500, "anchor": "center" })).unwrap();
        let b = content_bounds(s.doc().document()).unwrap();
        assert!((b.x0 - 150.0).abs() < 1.0 && (b.y0 - 150.0).abs() < 1.0, "{b:?}");
        assert!(s.execute("document.settings", &json!({ "anchor": "middle" })).is_err());
        assert!(s.execute("document.settings", &json!({ "width": -4 })).is_err());
        assert!(s.execute("document.settings", &json!({ "doc_type": "flash9" })).is_err());
    }

    #[test]
    fn match_contents_fits_the_artwork() {
        let mut s = session();
        assert!(s.execute("document.match_contents", &json!({})).is_err(), "nothing to match");
        let mut doc = AniDoc::default();
        rect(&mut doc, (40.0, 60.0), (200.0, 100.0));
        let mut s = Session::new(doc, super::super::registry());
        s.execute("document.match_contents", &json!({})).unwrap();
        let d = s.doc().document();
        assert!((d.width - 200.0).abs() < 1.5 && (d.height - 100.0).abs() < 1.5, "{} x {}", d.width, d.height);
        let b = content_bounds(d).unwrap();
        assert!(b.x0.abs() < 1.0 && b.y0.abs() < 1.0);
    }

    #[test]
    fn presets_apply_and_play_options_are_undoable() {
        let mut s = session();
        s.execute("document.apply_preset", &json!({ "preset": "story" })).unwrap();
        let d = s.doc().document();
        assert_eq!((d.width, d.height), (1080.0, 1920.0));
        assert_eq!(doc_type(d), DocType::Html5Canvas);
        assert!(s.execute("document.apply_preset", &json!({ "preset": "nope" })).is_err());
        s.execute("control.play_options", &json!({ "mute_sounds": true, "simple_buttons": true })).unwrap();
        assert_eq!(play_options(s.doc().document()), PlayOptions { mute_sounds: true, simple_buttons: true, frame_actions: false });
        s.undo();
        assert_eq!(play_options(s.doc().document()), PlayOptions::default());
        let out = s.execute("document.describe", &json!({})).unwrap();
        assert_eq!(out["categories"].as_array().unwrap().len(), 7);
        assert_eq!(out["document_types"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn play_options_survive_a_save() {
        let mut doc = AniDoc::default();
        set_play_options(doc.document_mut(), PlayOptions { mute_sounds: true, simple_buttons: false, frame_actions: true });
        let (bytes, _) = anicraft_xfl::write_fla_bytes(doc.document(), &doc.media, &doc.sidecars, &anicraft_xfl::WriteOptions::default()).unwrap();
        let loaded = anicraft_xfl::read_bytes(bytes, &anicraft_xfl::ReadOptions::default()).unwrap();
        assert_eq!(play_options(&loaded.document), PlayOptions { mute_sounds: true, simple_buttons: false, frame_actions: true });
    }
}
