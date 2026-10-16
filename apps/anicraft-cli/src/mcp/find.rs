//! Find and Replace.
//!
//! Searches the document for text, fonts, colours, symbols, bitmaps, sounds and video, and replaces
//! them. One walker serves finding and replacing, so what Find All lists is exactly what Replace All changes.

use anim_core::Rgba8;
use anim_scene::edit;
use anim_scene::{Document, Element, Frame, Layer, LibraryItem, Timeline, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use anim_shape::{EdgeShape, FillStyle, StrokeStyle};
use regex::{NoExpand, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

/// Most hits one search lists.
pub const MAX_HITS: usize = 5000;

/// What Find and Replace looks for (the "For" menu of the dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Characters of text fields, and the other texts selected in [`Query::text_in`].
    #[default]
    Text,
    /// Font faces of text runs.
    Font,
    /// Solid fill and stroke colours of shapes, and text colours.
    Color,
    /// Instances of a library symbol.
    Symbol,
    /// Instances and fills of a library bitmap.
    Bitmap,
    /// Frame sounds that play a library sound.
    Sound,
    /// Instances of a library video.
    Video,
}

impl Kind {
    /// Every kind, in the order of the dialog.
    pub const ALL: [Self; 7] = [Self::Text, Self::Font, Self::Color, Self::Symbol, Self::Sound, Self::Video, Self::Bitmap];

    /// The name the dialog shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Font => "Font",
            Self::Color => "Color",
            Self::Symbol => "Symbol",
            Self::Sound => "Sound",
            Self::Video => "Video",
            Self::Bitmap => "Bitmap",
        }
    }
}

/// Where a text search looks (the check boxes of the Text search).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextIn {
    /// Characters of static, dynamic and input text.
    TextFields,
    /// Layer names.
    LayerNames,
    /// Frame labels.
    FrameLabels,
    /// Instance names of symbols and text fields.
    InstanceNames,
    /// Names of library items (replacing renames them and fixes their uses).
    LibraryNames,
    /// Frame scripts.
    Scripts,
}

impl TextIn {
    /// Every place, in the order of the dialog.
    pub const ALL: [Self; 6] = [Self::TextFields, Self::LayerNames, Self::FrameLabels, Self::InstanceNames, Self::LibraryNames, Self::Scripts];

    /// The name the dialog shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TextFields => "Text fields",
            Self::LayerNames => "Layer names",
            Self::FrameLabels => "Frame labels",
            Self::InstanceNames => "Instance names",
            Self::LibraryNames => "Library item names",
            Self::Scripts => "ActionScript",
        }
    }
}

/// Which part of the document is searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "scene")]
pub enum Scope {
    /// Every scene and every symbol.
    #[default]
    Document,
    /// One scene only.
    Scene(usize),
}

/// A search, with the replacement when replacing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub kind: Kind,
    /// Text, font name, `#rrggbb`, or a library item name, according to `kind`.
    pub find: String,
    pub replace: String,
    /// For text searches: which texts are searched.
    pub text_in: Vec<TextIn>,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub scope: Scope,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            kind: Kind::Text,
            find: String::new(),
            replace: String::new(),
            text_in: vec![TextIn::TextFields],
            match_case: false,
            whole_word: false,
            regex: false,
            scope: Scope::Document,
        }
    }
}

/// One thing found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    /// The timeline it is on.
    pub timeline: TimelineId,
    pub layer: usize,
    /// The keyframe (its frame index, 0-based).
    pub frame: u32,
    /// The element path: index in the keyframe, then group members. Empty for frame, layer and library hits.
    pub path: Vec<usize>,
    /// What matched, for the results list.
    pub what: String,
    /// The text around the match, or the matched name.
    pub context: String,
}

/// Why a search cannot run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct QueryError(pub String);

struct Matcher {
    re: Regex,
    literal: bool,
}

impl Matcher {
    fn new(query: &Query) -> Result<Self, QueryError> {
        if query.find.is_empty() {
            return Err(QueryError("there is nothing to find".to_owned()));
        }
        let body = if query.regex { query.find.clone() } else { regex::escape(&query.find) };
        let pattern = if query.whole_word { format!(r"\b(?:{body})\b") } else { body };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!query.match_case)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| QueryError(format!("the regular expression is not valid: {e}")))?;
        Ok(Self { re, literal: !query.regex })
    }

    fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    fn replace(&self, text: &str, with: &str) -> String {
        if self.literal { self.re.replace_all(text, NoExpand(with)).into_owned() } else { self.re.replace_all(text, with).into_owned() }
    }
}

fn snippet(text: &str) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(60).collect();
    if text.chars().count() > 60 { format!("{flat}...") } else { flat }
}

/// Parses `#rrggbb` (alpha ignored).
fn rgb(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 && digits.len() != 8 {
        return None;
    }
    let byte = |i: usize| digits.get(i..i.saturating_add(2)).and_then(|s| u8::from_str_radix(s, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn same_rgb(c: Rgba8, want: [u8; 3]) -> bool {
    [c.r, c.g, c.b] == want
}

struct Walk<'q> {
    query: &'q Query,
    matcher: Option<Matcher>,
    find_rgb: Option<[u8; 3]>,
    replace_rgb: Option<[u8; 3]>,
    /// Replace instead of only listing.
    apply: bool,
    /// With `apply`: change only the hit with this number.
    only: Option<usize>,
    hits: Vec<Hit>,
    changed: usize,
}

impl Walk<'_> {
    const fn full(&self) -> bool {
        self.hits.len() >= MAX_HITS
    }

    /// Records a hit; returns true when the caller should change the thing now.
    fn hit(&mut self, at: &Where, path: &[usize], what: &str, context: &str) -> bool {
        if self.full() {
            return false;
        }
        let number = self.hits.len();
        self.hits.push(Hit {
            timeline: at.timeline.clone(),
            layer: at.layer,
            frame: at.frame,
            path: path.to_vec(),
            what: what.to_owned(),
            context: snippet(context),
        });
        let go = self.apply && self.only.is_none_or(|n| n == number);
        if go {
            self.changed = self.changed.saturating_add(1);
        }
        go
    }

    fn text_in(&self, place: TextIn) -> bool {
        self.query.text_in.contains(&place)
    }

    /// Changes `field` when it matches; used for every plain string target.
    fn string(&mut self, field: &mut String, at: &Where, path: &[usize], what: &str) {
        let Some(m) = &self.matcher else { return };
        if !m.is_match(field) {
            return;
        }
        let context = field.clone();
        let new = m.replace(field, &self.query.replace);
        if self.hit(at, path, what, &context) {
            *field = new;
        }
    }

    fn exact(&mut self, field: &mut String, at: &Where, path: &[usize], what: &str) {
        if *field != self.query.find {
            return;
        }
        let context = field.clone();
        if self.hit(at, path, what, &context) {
            self.query.replace.clone_into(field);
        }
    }

    fn colour(&mut self, c: &mut Rgba8, at: &Where, path: &[usize], what: &str) {
        let Some(want) = self.find_rgb else { return };
        if !same_rgb(*c, want) {
            return;
        }
        let context = format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
        if self.hit(at, path, what, &context)
            && let Some([r, g, b]) = self.replace_rgb
        {
            *c = Rgba8 { r, g, b, a: c.a };
        }
    }

    fn shape(&mut self, shape: &mut EdgeShape, at: &Where, path: &[usize]) {
        match self.query.kind {
            Kind::Color => {
                for fill in &mut shape.fills {
                    if let FillStyle::Solid(c) = fill {
                        self.colour(c, at, path, "fill colour");
                    }
                }
                for stroke in &mut shape.strokes {
                    let base = match stroke {
                        StrokeStyle::Solid(s) => s,
                        StrokeStyle::Other(o) => &mut o.base,
                    };
                    if let FillStyle::Solid(c) = &mut base.fill {
                        self.colour(c, at, path, "stroke colour");
                    }
                }
            }
            Kind::Bitmap => {
                for fill in &mut shape.fills {
                    if let FillStyle::Bitmap(b) = fill {
                        self.exact(&mut b.path, at, path, "bitmap fill");
                    }
                }
                for stroke in &mut shape.strokes {
                    let base = match stroke {
                        StrokeStyle::Solid(s) => s,
                        StrokeStyle::Other(o) => &mut o.base,
                    };
                    if let FillStyle::Bitmap(b) = &mut base.fill {
                        self.exact(&mut b.path, at, path, "bitmap stroke");
                    }
                }
            }
            _ => {}
        }
    }

    fn element(&mut self, element: &mut Element, at: &Where, path: &[usize], depth: usize) {
        match element {
            Element::Shape(s) => self.shape(&mut s.shape, at, path),
            Element::SymbolInstance(i) => match self.query.kind {
                Kind::Symbol => self.exact(&mut i.library_item_name, at, path, "symbol instance"),
                Kind::Text if self.text_in(TextIn::InstanceNames) => {
                    if let Some(name) = &mut i.name {
                        self.string(name, at, path, "instance name");
                    }
                }
                _ => {}
            },
            Element::Bitmap(b) => {
                if self.query.kind == Kind::Bitmap {
                    self.exact(&mut b.library_item_name, at, path, "bitmap instance");
                }
            }
            Element::Text(t) => {
                match self.query.kind {
                    Kind::Text if self.text_in(TextIn::TextFields) => {
                        for run in &mut t.text_runs {
                            self.string(&mut run.characters, at, path, "text");
                        }
                    }
                    Kind::Font => {
                        for run in &mut t.text_runs {
                            if let Some(m) = &self.matcher
                                && m.is_match(&run.face)
                            {
                                let face = run.face.clone();
                                let new = m.replace(&face, &self.query.replace);
                                if self.hit(at, path, "font", &face) {
                                    run.face = new;
                                }
                            }
                        }
                    }
                    Kind::Color => {
                        for run in &mut t.text_runs {
                            self.colour(&mut run.fill_color, at, path, "text colour");
                        }
                    }
                    _ => {}
                }
                if self.query.kind == Kind::Text
                    && self.text_in(TextIn::InstanceNames)
                    && let Some(name) = &mut t.instance_name
                {
                    self.string(name, at, path, "instance name");
                }
            }
            Element::Group(g) => {
                if depth < anim_scene::MAX_GROUP_DEPTH {
                    for (i, member) in g.members.iter_mut().enumerate() {
                        let mut inner = path.to_vec();
                        inner.push(i);
                        self.element(member, at, &inner, depth.saturating_add(1));
                    }
                }
            }
            Element::Unknown(raw) => {
                if self.query.kind == Kind::Video && raw.tag == "DOMVideoInstance" {
                    let needle = format!("libraryItemName=\"{}\"", self.query.find);
                    if raw.xml.contains(&needle) {
                        let context = self.query.find.clone();
                        if self.hit(at, path, "video instance", &context) {
                            raw.xml = raw.xml.replace(&needle, &format!("libraryItemName=\"{}\"", self.query.replace));
                        }
                    }
                }
            }
        }
    }

    fn frame(&mut self, frame: &mut Frame, at: &Where) {
        let at = Where { frame: frame.index, ..at.clone() };
        match self.query.kind {
            Kind::Text => {
                if self.text_in(TextIn::FrameLabels)
                    && let Some(label) = &mut frame.label
                {
                    self.string(label, &at, &[], "frame label");
                }
                if self.text_in(TextIn::Scripts)
                    && let Some(script) = &mut frame.actionscript
                {
                    self.string(script, &at, &[], "frame script");
                }
            }
            Kind::Sound => {
                if let Some(sound) = &mut frame.sound {
                    self.exact(&mut sound.name, &at, &[], "frame sound");
                }
            }
            _ => {}
        }
        for (i, element) in frame.elements.iter_mut().enumerate() {
            self.element(element, &at, &[i], 0);
            if self.full() {
                return;
            }
        }
    }

    fn layer(&mut self, layer: &mut Layer, index: usize, at: &Where) {
        let at = Where { layer: index, ..at.clone() };
        if self.query.kind == Kind::Text && self.text_in(TextIn::LayerNames) {
            self.string(&mut layer.name, &at, &[], "layer name");
        }
        for frame in &mut layer.frames {
            self.frame(frame, &at);
            if self.full() {
                return;
            }
        }
    }

    fn timeline(&mut self, timeline: &mut Timeline, id: TimelineId) {
        let at = Where { timeline: id, layer: 0, frame: 0 };
        for (i, layer) in timeline.layers.iter_mut().enumerate() {
            self.layer(layer, i, &at);
            if self.full() {
                return;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Where {
    timeline: TimelineId,
    layer: usize,
    frame: u32,
}

fn run(document: &mut Document, query: &Query, apply: bool, only: Option<usize>) -> Result<(Vec<Hit>, usize), QueryError> {
    let text_kind = matches!(query.kind, Kind::Text | Kind::Font);
    let matcher = text_kind.then(|| Matcher::new(query)).transpose()?;
    let (find_rgb, replace_rgb) = if query.kind == Kind::Color {
        let find = rgb(&query.find).ok_or_else(|| QueryError("a colour looks like #rrggbb".to_owned()))?;
        let replace = if apply { Some(rgb(&query.replace).ok_or_else(|| QueryError("the replacement colour looks like #rrggbb".to_owned()))?) } else { None };
        (Some(find), replace)
    } else {
        if !text_kind && query.find.is_empty() {
            return Err(QueryError("there is nothing to find".to_owned()));
        }
        (None, None)
    };
    if apply
        && matches!(query.kind, Kind::Symbol | Kind::Bitmap | Kind::Sound | Kind::Video)
        && !query.replace.is_empty()
        && document.library.find(&query.replace).is_none()
    {
        return Err(QueryError(format!("the library has no item called {:?} to replace with", query.replace)));
    }
    let mut walk = Walk { query, matcher, find_rgb, replace_rgb, apply, only, hits: Vec::new(), changed: 0 };
    for (i, scene) in document.scenes.iter_mut().enumerate() {
        if let Scope::Scene(only_scene) = query.scope
            && only_scene != i
        {
            continue;
        }
        walk.timeline(scene, TimelineId::Scene(i));
    }
    if query.scope == Scope::Document {
        for item in &mut document.library.items {
            if let LibraryItem::Symbol(symbol) = item {
                let id = TimelineId::Symbol(symbol.name.clone());
                walk.timeline(&mut symbol.timeline, id);
            }
        }
    }
    let mut hits = std::mem::take(&mut walk.hits);
    let mut changed = walk.changed;
    if query.kind == Kind::Text && query.text_in.contains(&TextIn::LibraryNames) && query.scope == Scope::Document {
        let m = walk.matcher.as_ref().ok_or_else(|| QueryError("there is nothing to find".to_owned()))?;
        let names: Vec<String> = document.library.items.iter().map(|i| i.name().to_owned()).filter(|n| m.is_match(leaf(n))).collect();
        for name in names {
            let number = hits.len();
            let go = apply && only.is_none_or(|n| n == number);
            hits.push(Hit {
                timeline: TimelineId::Scene(0),
                layer: 0,
                frame: 0,
                path: Vec::new(),
                what: "library item name".to_owned(),
                context: snippet(&name),
            });
            if go {
                let (folder, leaf_name) = name.rsplit_once('/').map_or(("", name.as_str()), |(f, l)| (f, l));
                let renamed = m.replace(leaf_name, &query.replace);
                let new = if folder.is_empty() { renamed } else { format!("{folder}/{renamed}") };
                if new != name && edit::rename_library_item(document, &name, &new).is_ok() {
                    changed = changed.saturating_add(1);
                }
            }
        }
    }
    Ok((hits, changed))
}

fn leaf(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, l)| l)
}

/// Lists everything the query matches.
///
/// # Errors
/// When the query is empty or its pattern or colour is not valid.
pub fn find_all(document: &mut Document, query: &Query) -> Result<Vec<Hit>, QueryError> {
    run(document, query, false, None).map(|(hits, _)| hits)
}

/// Replaces everything the query matches (`only` limits it to the hit with that number in [`find_all`]'s list).
/// Returns the hits that were listed and how many were changed.
///
/// # Errors
/// As [`find_all`], and when the replacement names a library item that does not exist.
pub fn replace(document: &mut Document, query: &Query, only: Option<usize>) -> Result<(Vec<Hit>, usize), QueryError> {
    run(document, query, true, only)
}

fn query_of(params: &Value) -> Result<Query, CommandError> {
    parse_params(params)
}

fn search_command(doc: &mut AniDoc, params: &Value) -> Out {
    let query = query_of(params)?;
    let mut scratch = doc.editor.document.clone();
    let hits = find_all(&mut scratch, &query).map_err(|e| CommandError::invalid_params(e.0))?;
    Ok(json!({ "count": hits.len(), "truncated": hits.len() >= MAX_HITS, "hits": serde_json::to_value(&hits).unwrap_or(Value::Null) }))
}

#[derive(Deserialize)]
struct ReplaceParams {
    #[serde(flatten)]
    query: Query,
    /// Replace only the hit with this number (0-based, in the order `find_search` lists them).
    #[serde(default)]
    only: Option<usize>,
}

fn replace_command(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ReplaceParams = parse_params(params)?;
    let (hits, changed) = replace(&mut doc.editor.document, &p.query, p.only).map_err(|e| CommandError::invalid_params(e.0))?;
    if changed == 0 {
        return Err(CommandError::failed("nothing was replaced"));
    }
    Ok(json!({ "found": hits.len(), "replaced": changed }))
}

/// Registers the commands of this module.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    const SCHEMA: &str = r#"{"type":"object","properties":{"kind":{"enum":["text","font","color","symbol","bitmap","sound","video"]},"find":{"type":"string"},"replace":{"type":"string"},"text_in":{"type":"array","items":{"enum":["text_fields","layer_names","frame_labels","instance_names","library_names","scripts"]}},"match_case":{"type":"boolean"},"whole_word":{"type":"boolean"},"regex":{"type":"boolean"},"scope":{"type":"object","description":"{\"kind\":\"document\"} or {\"kind\":\"scene\",\"scene\":0}"},"only":{"type":"integer","minimum":0}},"required":["find"]}"#;
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry
        .register_all(vec![
            CommandSpec::new("find.search", "Find All", search_command)
                .transient()
                .describe("Find and Replace, Find All: lists where text, a font, a colour (#rrggbb), a symbol, a bitmap, a sound or a video is used. For text, `text_in` picks text_fields, layer_names, frame_labels, instance_names, library_names and scripts. The hits carry timeline, layer, frame and element path.")
                .with_schema(SCHEMA),
            CommandSpec::new("find.replace", "Replace All", replace_command)
                .in_menu(&["Edit"])
                .describe("Find and Replace, Replace All (or Replace of the hit numbered `only`). Replaces text (regular expressions may use $1), a font name, a colour, or points every use of a symbol, bitmap, sound or video at another library item.")
                .with_schema(SCHEMA),
        ])
        .expect("find command ids are unique");
}

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;
    use crate::mcp::registry;

    fn doc_with_text() -> AniDoc {
        let mut s = Session::new(AniDoc::default(), registry());
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 40, "height": 40, "fill": "#ff0000", "stroke": "#00ff00", "object_drawing": false }))
            .unwrap();
        s.execute("draw.rectangle", &json!({ "x": 50, "y": 0, "width": 40, "height": 40, "fill": "#ff0000", "object_drawing": true })).unwrap();
        let mut doc = s.doc().clone();
        let frame = &mut doc.editor.document.scenes[0].layers[0].frames[0];
        let run = anim_scene::TextRun::new("Hello world, hello World", "Arial", 24.0);
        frame.elements.push(Element::Text(Box::new(anim_scene::TextElement::new(
            anim_scene::TextKind::Static,
            anim_core::Affine::IDENTITY,
            200.0,
            40.0,
            vec![run],
        ))));
        frame.label = Some("intro".to_owned());
        doc.editor.document.scenes[0].layers[0].name = "Hello layer".to_owned();
        doc
    }

    #[test]
    fn text_search_honours_case_and_whole_word() {
        let mut doc = doc_with_text();
        let q = Query { find: "hello".to_owned(), ..Query::default() };
        let hits = find_all(&mut doc.editor.document, &q).unwrap();
        assert_eq!(hits.len(), 1, "one text run; case folded");
        let q = Query { find: "hello".to_owned(), match_case: true, whole_word: true, ..Query::default() };
        assert_eq!(find_all(&mut doc.editor.document, &q).unwrap().len(), 1, "the lower case word");
        let q = Query { find: "Hell".to_owned(), whole_word: true, ..Query::default() };
        assert_eq!(find_all(&mut doc.editor.document, &q).unwrap().len(), 0);
        let q = Query { find: "(".to_owned(), regex: true, ..Query::default() };
        assert!(find_all(&mut doc.editor.document, &q).is_err(), "a bad pattern is an error");
        assert!(find_all(&mut doc.editor.document, &Query::default()).is_err(), "empty find");
    }

    #[test]
    fn replace_text_labels_layers_and_undo() {
        let mut s = Session::new(doc_with_text(), registry());
        s.execute("find.replace", &json!({ "find": "hello", "replace": "Bye", "text_in": ["text_fields", "layer_names", "frame_labels"] })).unwrap();
        let d = &s.doc().editor.document;
        assert_eq!(d.scenes[0].layers[0].name, "Bye layer");
        let Element::Text(t) = d.scenes[0].layers[0].frames[0].elements.last().unwrap() else { panic!("text") };
        assert_eq!(t.plain_text(), "Bye world, Bye World");
        s.undo();
        assert_eq!(s.doc().editor.document.scenes[0].layers[0].name, "Hello layer");
        let r = s.execute("find.replace", &json!({ "find": "zzz", "replace": "x" }));
        assert!(r.is_err(), "nothing to replace is an error and records no step");
    }

    #[test]
    fn replace_one_hit_and_regex_groups() {
        let mut doc = doc_with_text();
        let q = Query { find: r"(\w+) (\w+)".to_owned(), replace: "$2 $1".to_owned(), regex: true, ..Query::default() };
        let (hits, changed) = replace(&mut doc.editor.document, &q, None).unwrap();
        assert_eq!((hits.len(), changed), (1, 1));
        let Element::Text(t) = doc.editor.document.scenes[0].layers[0].frames[0].elements.last().unwrap() else { panic!("text") };
        assert_eq!(t.plain_text(), "world Hello, World hello");
        let q = Query { find: "x".to_owned(), replace: "$1".to_owned(), ..Query::default() };
        let hits = find_all(&mut doc.editor.document, &Query { find: "o".to_owned(), ..Query::default() }).unwrap();
        assert_ne!(hits.len(), 0);
        let _ = q;
    }

    #[test]
    fn colours_are_found_and_replaced_in_fills_and_strokes() {
        let mut doc = doc_with_text();
        let q = Query { kind: Kind::Color, find: "#ff0000".to_owned(), replace: "#0000ff".to_owned(), ..Query::default() };
        let hits = find_all(&mut doc.editor.document, &q).unwrap();
        assert_eq!(hits.len(), 2, "two red fills");
        let (_, changed) = replace(&mut doc.editor.document, &q, None).unwrap();
        assert_eq!(changed, 2);
        assert_eq!(find_all(&mut doc.editor.document, &q).unwrap().len(), 0);
        let green = Query { kind: Kind::Color, find: "#00ff00".to_owned(), ..Query::default() };
        assert_eq!(find_all(&mut doc.editor.document, &green).unwrap().len(), 1, "the stroke");
        assert!(find_all(&mut doc.editor.document, &Query { kind: Kind::Color, find: "red".to_owned(), ..Query::default() }).is_err());
    }

    #[test]
    fn fonts_and_symbols() {
        let mut s = Session::new(doc_with_text(), registry());
        s.execute("find.replace", &json!({ "kind": "font", "find": "arial", "replace": "Georgia" })).unwrap();
        let Element::Text(t) = s.doc().editor.document.scenes[0].layers[0].frames[0].elements.last().unwrap() else { panic!("text") };
        assert_eq!(t.text_runs[0].face, "Georgia");
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.convert_to_symbol", &json!({ "name": "box", "kind": "graphic" })).unwrap();
        s.execute("insert.symbol", &json!({ "name": "other" })).ok();
        let found = s.execute("find.search", &json!({ "kind": "symbol", "find": "box" })).unwrap();
        assert_eq!(found["count"], 1);
        let missing = s.execute("find.replace", &json!({ "kind": "symbol", "find": "box", "replace": "nope" }));
        assert!(missing.is_err(), "the replacement item must exist");
        let found = s.execute("find.search", &json!({ "kind": "symbol", "find": "box", "scope": { "kind": "scene", "scene": 5 } })).unwrap();
        assert_eq!(found["count"], 0);
    }
}
