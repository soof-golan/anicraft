//! Helpers shared by the element readers.
//!
//! A reading context collects warnings and "unknown" statistics, an attribute cursor hands whatever it did not
//! consume to an [`Extra`], and small functions read the XFL value types (`Matrix`, `Point`).

use std::collections::BTreeMap;

use anim_core::{Affine, Point, Rgba8};
use anim_scene::Extra;
use roxmltree::Node;
use serde::{Deserialize, Serialize};

/// Attributes that only describe the editor's selection and current-layer state. They are kept in `extra` like any
/// other unknown attribute (so files round-trip) but counted apart from the real unknowns.
const UI_STATE_ATTRS: [&str; 4] = ["selected", "current", "isSelected", "isFloating"];

/// Most warnings kept; further ones are only counted.
const MAX_WARNINGS: usize = 2_000;

/// Something the reader noticed and worked around.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Archive entry the warning is about (`DOMDocument.xml`, `LIBRARY/foo.xml`, `bin/M 1.dat`).
    pub file: String,
    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.message)
    }
}

/// What ended up in `extra` because the model has no slot for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unknowns {
    /// `Tag@attribute` -> occurrences (editor selection state excluded, see [`Unknowns::ui_state`]).
    pub attrs: BTreeMap<String, u64>,
    /// `Parent/Child` -> occurrences of child elements kept as raw XML.
    pub children: BTreeMap<String, u64>,
    /// Selection / current-layer flags kept in `extra`.
    pub ui_state: u64,
}

impl Unknowns {
    /// Total number of unknown attributes and elements (without selection state).
    #[must_use]
    pub fn total(&self) -> u64 {
        self.attrs.values().chain(self.children.values()).fold(0u64, |a, b| a.saturating_add(*b))
    }

    /// The `n` most frequent unknowns, attributes and elements together, most frequent first.
    #[must_use]
    pub fn top(&self, n: usize) -> Vec<(String, u64)> {
        let mut all: Vec<(String, u64)> = self.attrs.iter().chain(self.children.iter()).map(|(k, v)| (k.clone(), *v)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        all.truncate(n);
        all
    }
}

/// Everything the readers report, shared across files.
#[derive(Debug, Default)]
pub struct Sink {
    pub warnings: Vec<Warning>,
    pub dropped_warnings: u64,
    pub unknowns: Unknowns,
}

impl Sink {
    pub fn warn(&mut self, file: &str, message: impl Into<String>) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(Warning { file: file.to_owned(), message: message.into() });
        } else {
            self.dropped_warnings = self.dropped_warnings.saturating_add(1);
        }
    }
}

/// Reading context for one XML file.
pub struct Ctx<'a> {
    /// The file's text, which node byte ranges index into.
    pub src: &'a str,
    /// Archive entry name, for warnings.
    pub file: &'a str,
    pub sink: &'a mut Sink,
}

impl<'a> Ctx<'a> {
    pub const fn new(src: &'a str, file: &'a str, sink: &'a mut Sink) -> Self {
        Self { src, file, sink }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.sink.warn(self.file, message);
    }

    /// The complete source text of `node` including its children.
    #[must_use]
    pub fn raw(&self, node: Node<'_, '_>) -> String {
        self.src.get(node.range()).unwrap_or_default().to_owned()
    }

    /// Keeps a child element nobody modelled as raw XML in `extra`.
    pub fn keep_child(&mut self, extra: &mut Extra, parent: &str, child: Node<'_, '_>) {
        let key = format!("{parent}/{}", child.tag_name().name());
        let slot = self.sink.unknowns.children.entry(key).or_insert(0);
        *slot = slot.saturating_add(1);
        extra.children.push(self.raw(child));
    }

    /// Counts an unknown attribute that has nowhere to be kept (a field of a value type such as `Matrix`).
    pub fn lost_attr(&mut self, tag: &str, name: &str) {
        let slot = self.sink.unknowns.attrs.entry(format!("{tag}@{name}")).or_insert(0);
        *slot = slot.saturating_add(1);
    }
}

/// Cursor over the attributes of one element. Reading an attribute marks it as consumed; [`Attrs::rest`] hands the
/// remaining ones to an [`Extra`] in file order.
pub struct Attrs<'a, 'i> {
    node: Node<'a, 'i>,
    taken: Vec<bool>,
}

impl<'a, 'i> Attrs<'a, 'i> {
    #[must_use]
    pub fn new(node: Node<'a, 'i>) -> Self {
        let taken = vec![false; node.attributes().len()];
        Self { node, taken }
    }

    /// The value of `name`, marking it consumed.
    pub fn take(&mut self, name: &str) -> Option<&'a str> {
        let (position, attribute) = self.node.attributes().enumerate().find(|(_, a)| a.name() == name)?;
        if let Some(flag) = self.taken.get_mut(position) {
            *flag = true;
        }
        Some(attribute.value())
    }

    /// Reads and consumes `name` when `parse` accepts the value; an attribute `parse` rejects stays unconsumed (so it
    /// lands in `extra` unchanged) and is reported.
    pub fn take_with<T>(&mut self, ctx: &mut Ctx<'_>, name: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        let (position, attribute) = self.node.attributes().enumerate().find(|(_, a)| a.name() == name)?;
        if let Some(value) = parse(attribute.value()) {
            if let Some(flag) = self.taken.get_mut(position) {
                *flag = true;
            }
            Some(value)
        } else {
            ctx.warn(format!("{}: cannot read {name}=\"{}\"", self.node.tag_name().name(), attribute.value()));
            None
        }
    }

    pub fn string(&mut self, name: &str) -> Option<String> {
        self.take(name).map(str::to_owned)
    }

    pub fn f64(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<f64> {
        self.take_with(ctx, name, parse_f64)
    }

    pub fn f32(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<f32> {
        self.take_with(ctx, name, |s| parse_f64(s).map(anim_core::num::f64_to_f32))
    }

    pub fn u32(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<u32> {
        self.take_with(ctx, name, |s| s.trim().parse::<u32>().ok())
    }

    pub fn u64(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<u64> {
        self.take_with(ctx, name, |s| s.trim().parse::<u64>().ok())
    }

    pub fn i32(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<i32> {
        self.take_with(ctx, name, |s| s.trim().parse::<i32>().ok())
    }

    pub fn bool(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<bool> {
        self.take_with(ctx, name, parse_bool)
    }

    pub fn color(&mut self, ctx: &mut Ctx<'_>, name: &str) -> Option<Rgba8> {
        self.take_with(ctx, name, |s| Rgba8::from_hex(s).ok())
    }

    /// Moves every attribute not consumed so far into `extra`, counting it in the statistics.
    pub fn rest(self, ctx: &mut Ctx<'_>, extra: &mut Extra) {
        let tag = self.node.tag_name().name();
        for (attribute, taken) in self.node.attributes().zip(self.taken.iter()) {
            if *taken {
                continue;
            }
            extra.attrs.push((attribute.name().to_owned(), attribute.value().to_owned()));
            if UI_STATE_ATTRS.contains(&attribute.name()) {
                ctx.sink.unknowns.ui_state = ctx.sink.unknowns.ui_state.saturating_add(1);
            } else {
                let slot = ctx.sink.unknowns.attrs.entry(format!("{tag}@{}", attribute.name())).or_insert(0);
                *slot = slot.saturating_add(1);
            }
        }
    }

    /// Like [`Attrs::rest`] for a value type without an `Extra`: the attributes are only counted.
    pub fn rest_lost(self, ctx: &mut Ctx<'_>) {
        let tag = self.node.tag_name().name();
        for (attribute, taken) in self.node.attributes().zip(self.taken.iter()) {
            if !*taken {
                ctx.lost_attr(tag, attribute.name());
            }
        }
    }
}

/// Parses a finite number.
#[must_use]
pub fn parse_f64(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Parses `true` / `false` (and `1` / `0`).
#[must_use]
pub fn parse_bool(text: &str) -> Option<bool> {
    match text.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Element children of `node`.
pub fn element_children<'a, 'i>(node: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(Node::is_element)
}

/// The first element child called `name`.
#[must_use]
pub fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    element_children(node).find(|c| c.tag_name().name() == name)
}

/// Concatenated text content of `node` (text and CDATA children).
#[must_use]
pub fn text_of(node: Node<'_, '_>) -> String {
    node.children().filter(Node::is_text).filter_map(|t| t.text()).collect()
}

/// Reads a `<Matrix a b c d tx ty/>` element. Missing attributes are the identity's.
pub fn read_matrix_element(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Affine {
    let mut attrs = Attrs::new(node);
    let a = attrs.f64(ctx, "a").unwrap_or(1.0);
    let b = attrs.f64(ctx, "b").unwrap_or(0.0);
    let c = attrs.f64(ctx, "c").unwrap_or(0.0);
    let d = attrs.f64(ctx, "d").unwrap_or(1.0);
    let tx = attrs.f64(ctx, "tx").unwrap_or(0.0);
    let ty = attrs.f64(ctx, "ty").unwrap_or(0.0);
    attrs.rest_lost(ctx);
    Affine::new([a, b, c, d, tx, ty])
}

/// Reads a `<matrix>` wrapper holding one `<Matrix>`; the identity when it is empty.
pub fn read_matrix(ctx: &mut Ctx<'_>, wrapper: Node<'_, '_>) -> Affine {
    let mut matrix = None;
    for c in element_children(wrapper) {
        if c.tag_name().name() == "Matrix" && matrix.is_none() {
            matrix = Some(read_matrix_element(ctx, c));
        } else {
            ctx.lost_attr(wrapper.tag_name().name(), c.tag_name().name());
        }
    }
    matrix.unwrap_or(Affine::IDENTITY)
}

/// Reads a `<Point x y/>` element. Missing attributes are zero.
pub fn read_point_element(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> Point {
    let mut attrs = Attrs::new(node);
    let x = attrs.f64(ctx, "x").unwrap_or(0.0);
    let y = attrs.f64(ctx, "y").unwrap_or(0.0);
    attrs.rest_lost(ctx);
    Point::new(x, y)
}

/// Reads a wrapper such as `<transformationPoint>` holding one `<Point>`.
pub fn read_point(ctx: &mut Ctx<'_>, wrapper: Node<'_, '_>) -> Point {
    let mut point = None;
    for c in element_children(wrapper) {
        if c.tag_name().name() == "Point" && point.is_none() {
            point = Some(read_point_element(ctx, c));
        } else {
            ctx.lost_attr(wrapper.tag_name().name(), c.tag_name().name());
        }
    }
    point.unwrap_or(Point::ORIGIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_booleans_parse_defensively() {
        assert_eq!(parse_f64(" 1.5 "), Some(1.5));
        assert_eq!(parse_f64("NaN"), None);
        assert_eq!(parse_f64("inf"), None);
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("yes"), None);
    }

    #[test]
    fn unconsumed_attributes_go_to_extra_in_file_order() {
        let doc = roxmltree::Document::parse(r#"<A one="1" two="2" selected="true" bad="x"/>"#).unwrap();
        let mut sink = Sink::default();
        let mut ctx = Ctx::new("", "t.xml", &mut sink);
        let mut attrs = Attrs::new(doc.root_element());
        assert_eq!(attrs.u32(&mut ctx, "one"), Some(1));
        assert_eq!(attrs.u32(&mut ctx, "bad"), None);
        let mut extra = Extra::new();
        attrs.rest(&mut ctx, &mut extra);
        let names: Vec<&str> = extra.attrs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["two", "selected", "bad"]);
        assert_eq!(sink.unknowns.ui_state, 1);
        assert_eq!(sink.unknowns.attrs.get("A@two"), Some(&1));
        assert!(!sink.warnings.is_empty());
    }

    #[test]
    fn matrix_defaults_to_identity_parts() {
        let doc = roxmltree::Document::parse(r#"<matrix><Matrix tx="3" d="2"/></matrix>"#).unwrap();
        let mut sink = Sink::default();
        let mut ctx = Ctx::new("", "t.xml", &mut sink);
        let m = read_matrix(&mut ctx, doc.root_element());
        assert_eq!(m, Affine::new([1.0, 0.0, 0.0, 2.0, 3.0, 0.0]));
    }
}
