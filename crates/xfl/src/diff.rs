//! Semantic comparison of XFL documents: the same elements, attributes and text, whatever the indentation, the
//! attribute order or the way a number is spelled.
//!
//! Used to check the writer against the files it read ([`compare_archives`]) and handy for tools that want to say
//! what changed between two saves.

use std::collections::BTreeMap;

use anim_archive::Container;
use anim_archive::names::lookup_key;
use roxmltree::Node;
use serde::{Deserialize, Serialize};

use crate::xml::{element_children, parse};

/// Deepest nesting compared; below it the subtrees are reported as not compared.
const MAX_COMPARE_DEPTH: usize = 512;

/// What kind of difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DiffKind {
    /// The elements have different names.
    Tag,
    /// An attribute of the first document is missing from the second.
    AttributeMissing,
    /// An attribute only the second document has.
    AttributeAdded,
    /// The attribute is in both with different values.
    AttributeChanged,
    /// The elements have different numbers of children.
    ChildCount,
    /// Different text content.
    Text,
    /// Nesting too deep to compare.
    TooDeep,
    /// One side is not well-formed XML.
    Parse,
}

/// One difference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlDifference {
    /// Where, as `DOMDocument/timelines[0]/DOMTimeline[0]/...` with the position among the parent's children.
    pub path: String,
    pub kind: DiffKind,
    pub detail: String,
}

/// How to compare.
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Most differences kept in the result; the counters keep counting.
    pub max_reported: usize,
    /// Relative tolerance for numbers.
    pub tolerance: f64,
    /// Treat an attribute that is absent on one side and equal to the format's default on the other as equal
    /// (see [`is_default`]).
    pub ignore_defaults: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        // Gradient ratios, alphas and tint amounts are held as `f32`, so numbers agree to about seven digits.
        Self { max_reported: 200, tolerance: 1e-6, ignore_defaults: true }
    }
}

/// The result of comparing two XML texts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct XmlDiff {
    /// The first [`DiffOptions::max_reported`] differences.
    pub differences: Vec<XmlDifference>,
    /// All differences found.
    pub total: u64,
    /// Count per `kind: Tag@attribute` (or `kind: Tag`), for seeing what is systematic.
    pub by_key: BTreeMap<String, u64>,
}

impl XmlDiff {
    /// True when nothing differs.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// The `n` most frequent difference keys.
    #[must_use]
    pub fn top(&self, n: usize) -> Vec<(String, u64)> {
        let mut all: Vec<(String, u64)> = self.by_key.iter().map(|(k, v)| (k.clone(), *v)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        all.truncate(n);
        all
    }

    fn note(&mut self, options: &DiffOptions, path: &[String], kind: DiffKind, key: &str, detail: String) {
        self.total = self.total.saturating_add(1);
        let slot = self.by_key.entry(format!("{kind:?}: {key}")).or_insert(0);
        *slot = slot.saturating_add(1);
        if self.differences.len() < options.max_reported {
            self.differences.push(XmlDifference { path: path.join("/"), kind, detail });
        }
    }
}

/// True when `value` is what a reader assumes for attribute `name` of element `tag` when it is absent, so that
/// writing or omitting it means the same.
#[must_use]
#[allow(
    clippy::match_same_arms,
    clippy::unnested_or_patterns,
    reason = "a table with one row per attribute is easier to check against the reader than merged patterns"
)]
pub fn is_default(tag: &str, name: &str, value: &str) -> bool {
    let number = value.trim().parse::<f64>().ok();
    let is = |n: f64| number.is_some_and(|v| (v - n).abs() < 1e-12);
    match (tag, name) {
        ("DOMFrame", "duration") => is(1.0),
        ("DOMFrame", "motionTweenScale") => value == "true",
        ("DOMFrame", "motionTweenRotate") => value == "auto",
        ("DOMFrame", "acceleration") => is(0.0),
        ("DOMFrame", "motionTweenSnap" | "motionTweenOrientToPath" | "motionTweenSync" | "hasCustomEase") => value == "false",
        ("DOMFrame", "soundSync") => value == "event",
        ("DOMFrame", "keyMode") => is(9728.0),
        ("DOMFrame", "tweenType") => value == "none",
        ("DOMLayer", "layerType") => value == "normal",
        ("DOMLayer", "locked" | "outline") => value == "false",
        ("DOMLayer", "visible" | "autoNamed") => value == "true",
        ("DOMTimeline", "currentFrame") => is(0.0),
        ("DOMSymbolItem", "symbolType") => value == "movie clip",
        ("DOMSymbolInstance", "firstFrame") => is(0.0),
        ("DOMSymbolInstance", "isVisible") => value == "true",
        ("DOMSymbolInstance", "cacheAsBitmap") => value == "false",
        ("DOMSymbolInstance", "blendMode") => value == "normal",
        ("DOMBitmapItem", "allowSmoothing") => value == "false",
        ("DOMShape", "isDrawingObject") => value == "false",
        ("Matrix", "a" | "d") => is(1.0),
        ("Matrix", "b" | "c" | "tx" | "ty") => is(0.0),
        ("Point", "x" | "y") => is(0.0),
        ("SolidColor", "alpha") | ("GradientEntry", "alpha") | ("DOMTextAttrs", "alpha") | ("DropShadowFilter" | "GlowFilter", "alpha") => is(1.0),
        ("SolidColor", "color") | ("GradientEntry", "color") => value.eq_ignore_ascii_case("#000000"),
        ("LinearGradient" | "RadialGradient", "spreadMethod") => value == "extend",
        ("LinearGradient" | "RadialGradient", "interpolationMethod") => value == "RGB",
        ("LinearGradient" | "RadialGradient", "focalPointRatio") => is(0.0),
        ("BitmapFill", "bitmapIsClipped") => value == "false",
        ("BitmapFill", "allowSmoothing") => value == "true",
        ("SolidStroke", "pixelHinting") => value == "false",
        ("SolidStroke", "weight") => is(1.0),
        ("SoundEnvelopePoint", "mark44") => is(0.0),
        ("DOMDocument", "backgroundColor") => value.eq_ignore_ascii_case("#FFFFFF"),
        ("DOMDocument", "width") => is(550.0),
        ("DOMDocument", "height") => is(400.0),
        ("DOMDocument", "frameRate") => is(24.0),
        ("DOMTextAttrs", "bold" | "italic") => value == "false",
        ("DOMTextAttrs", "letterSpacing") => is(0.0),
        ("DOMTextAttrs", "alignment") => value == "left",
        ("DOMStaticText" | "DOMDynamicText" | "DOMInputText", "autoExpand") => value == "false",
        ("DOMStaticText" | "DOMDynamicText" | "DOMInputText", "orientation") => value == "horizontal",
        ("DropShadowFilter" | "GlowFilter", "inner" | "knockout" | "hideObject") => value == "false",
        ("DOMFolderItem", "isExpanded") => value == "false",
        ("Edge", "fillStyle0" | "fillStyle1" | "strokeStyle") => is(0.0),
        _ => false,
    }
}

fn numbers_equal(a: &str, b: &str, tolerance: f64) -> bool {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => (x - y).abs() <= tolerance * x.abs().max(y.abs()).max(1.0),
        _ => false,
    }
}

fn values_equal(a: &str, b: &str, options: &DiffOptions) -> bool {
    a == b || (a.starts_with('#') && a.eq_ignore_ascii_case(b)) || numbers_equal(a, b, options.tolerance)
}

/// Compares two XML texts.
#[must_use]
pub fn compare_xml(a: &str, b: &str, options: &DiffOptions) -> XmlDiff {
    let mut diff = XmlDiff::default();
    let (first, second) = match (parse(a), parse(b)) {
        (Ok(first), Ok(second)) => (first, second),
        (first, second) => {
            let detail = format!(
                "first: {}, second: {}",
                first.err().map_or_else(|| "ok".to_owned(), |e| e.to_string()),
                second.err().map_or_else(|| "ok".to_owned(), |e| e.to_string())
            );
            diff.note(options, &[], DiffKind::Parse, "document", detail);
            return diff;
        }
    };
    let mut path = vec![first.root_element().tag_name().name().to_owned()];
    compare_nodes(first.root_element(), second.root_element(), &mut path, options, &mut diff);
    diff
}

fn compare_nodes(a: Node<'_, '_>, b: Node<'_, '_>, path: &mut Vec<String>, options: &DiffOptions, diff: &mut XmlDiff) {
    let tag = a.tag_name().name();
    if tag != b.tag_name().name() {
        diff.note(options, path, DiffKind::Tag, tag, format!("{tag} vs {}", b.tag_name().name()));
        return;
    }
    if path.len() > MAX_COMPARE_DEPTH {
        diff.note(options, path, DiffKind::TooDeep, tag, "nested too deeply to compare".to_owned());
        return;
    }
    for attribute in a.attributes() {
        let name = attribute.name();
        match b.attribute(name) {
            Some(other) if values_equal(attribute.value(), other, options) => {}
            Some(other) => diff.note(options, path, DiffKind::AttributeChanged, &format!("{tag}@{name}"), format!("{:?} vs {other:?}", attribute.value())),
            None if options.ignore_defaults && is_default(tag, name, attribute.value()) => {}
            None => diff.note(options, path, DiffKind::AttributeMissing, &format!("{tag}@{name}"), format!("{name}={:?} is missing", attribute.value())),
        }
    }
    for attribute in b.attributes() {
        let name = attribute.name();
        if a.attribute(name).is_none() && !(options.ignore_defaults && is_default(tag, name, attribute.value())) {
            diff.note(options, path, DiffKind::AttributeAdded, &format!("{tag}@{name}"), format!("{name}={:?} is new", attribute.value()));
        }
    }
    let kids_a = comparable_children(a, options);
    let kids_b = comparable_children(b, options);
    if kids_a.is_empty() && kids_b.is_empty() {
        let text_a = text_content(a);
        let text_b = text_content(b);
        if text_a != text_b {
            diff.note(options, path, DiffKind::Text, tag, format!("{:?} vs {:?}", clip(&text_a), clip(&text_b)));
        }
        return;
    }
    if kids_a.len() != kids_b.len() {
        diff.note(options, path, DiffKind::ChildCount, tag, format!("{} children vs {}", kids_a.len(), kids_b.len()));
    }
    for (index, (x, y)) in kids_a.into_iter().zip(kids_b).enumerate() {
        path.push(format!("{}[{index}]", x.tag_name().name()));
        compare_nodes(x, y, path, options, diff);
        path.pop();
    }
}

/// The element children of `node`, without a `<matrix>` that repeats the matrix of the group the node is a member of.
/// The reader places such a member by the group's matrix alone (see `read_group`), so the two spellings mean the same.
fn comparable_children<'a, 'input>(node: Node<'a, 'input>, options: &DiffOptions) -> Vec<Node<'a, 'input>> {
    let group = node
        .parent_element()
        .filter(|p| p.tag_name().name() == "members")
        .and_then(|p| p.parent_element())
        .filter(|g| g.tag_name().name() == "DOMGroup")
        .and_then(|g| element_children(g).find(|c| c.tag_name().name() == "matrix"))
        .map(matrix_numbers);
    element_children(node)
        .filter(|c| {
            let redundant = c.tag_name().name() == "matrix"
                && group.is_some_and(|g| g.iter().zip(matrix_numbers(*c)).all(|(x, y)| (x - y).abs() <= options.tolerance * x.abs().max(1.0)));
            !redundant
        })
        .collect()
}

/// `a b c d tx ty` of the `<Matrix>` inside a `<matrix>` wrapper, with the identity's values for missing ones.
fn matrix_numbers(wrapper: Node<'_, '_>) -> [f64; 6] {
    let mut out = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    if let Some(m) = element_children(wrapper).find(|c| c.tag_name().name() == "Matrix") {
        for (slot, name) in out.iter_mut().zip(["a", "b", "c", "d", "tx", "ty"]) {
            if let Some(v) = m.attribute(name).and_then(|v| v.trim().parse::<f64>().ok()) {
                *slot = v;
            }
        }
    }
    out
}

fn text_content(node: Node<'_, '_>) -> String {
    node.children().filter(Node::is_text).filter_map(|t| t.text()).collect()
}

fn clip(text: &str) -> String {
    let mut out: String = text.chars().take(60).collect();
    if text.chars().count() > 60 {
        out.push_str("...");
    }
    out
}

/// What differs between two archives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveDiff {
    /// `DOMDocument.xml`.
    pub document: XmlDiff,
    /// Symbol files (`LIBRARY/*.xml`) that differ, by entry name of the first archive.
    pub symbols: Vec<(String, XmlDiff)>,
    /// Entries only the first archive has.
    pub only_in_first: Vec<String>,
    /// Entries only the second archive has.
    pub only_in_second: Vec<String>,
    /// Entries in both whose bytes differ (non-XML entries and `PublishSettings.xml`).
    pub bytes_differ: Vec<String>,
    /// Entries in both that could not be read.
    pub unreadable: Vec<String>,
}

/// Compares two archives (zips or folders).
///
/// `DOMDocument.xml` and every symbol file are compared semantically, everything else byte for byte. Entries are
/// matched by name the way lookups are (case and escape insensitive).
#[must_use]
pub fn compare_archives(first: &Container, second: &Container, options: &DiffOptions) -> ArchiveDiff {
    let mut result = ArchiveDiff::default();
    let key = |name: &str| lookup_key(name);
    let in_second: BTreeMap<String, &anim_archive::Entry> = second.files().map(|e| (key(&e.name), e)).collect();
    let mut matched: Vec<String> = Vec::new();
    for entry in first.files() {
        let k = key(&entry.name);
        let Some(other) = in_second.get(&k) else {
            result.only_in_first.push(entry.name.clone());
            continue;
        };
        matched.push(k);
        let (Ok(a), Ok(b)) = (first.read_entry(entry), second.read_entry(other)) else {
            result.unreadable.push(entry.name.clone());
            continue;
        };
        let lower = entry.name.to_ascii_lowercase();
        let is_xml = std::path::Path::new(&lower).extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        let is_model_xml = lower == "domdocument.xml" || (lower.starts_with("library/") && is_xml);
        if is_model_xml {
            let (Ok(text_a), Ok(text_b)) = (String::from_utf8(a), String::from_utf8(b)) else {
                result.unreadable.push(entry.name.clone());
                continue;
            };
            let diff = compare_xml(&text_a, &text_b, options);
            if lower == "domdocument.xml" {
                result.document = diff;
            } else if !diff.is_empty() {
                result.symbols.push((entry.name.clone(), diff));
            }
        } else if a != b {
            result.bytes_differ.push(entry.name.clone());
        }
    }
    for (k, entry) in &in_second {
        if !matched.contains(k) {
            result.only_in_second.push(entry.name.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_order_and_whitespace_do_not_matter() {
        let a = r##"<R a="1" b="#aabbcc"><C x="0.5"/>  <D>text</D></R>"##;
        let b = "<R b=\"#AABBCC\" a=\"1.0\">\n\t<C x=\"0.50\"/><D>text</D>\n</R>";
        assert!(compare_xml(a, b, &DiffOptions::default()).is_empty());
    }

    #[test]
    fn differences_are_located_and_counted() {
        let a = r#"<R><C x="1" y="2"/><C/><D>one</D></R>"#;
        let b = r#"<R><C x="2" z="3"/><E/></R>"#;
        let diff = compare_xml(a, b, &DiffOptions::default());
        let kinds: Vec<DiffKind> = diff.differences.iter().map(|d| d.kind).collect();
        assert!(kinds.contains(&DiffKind::AttributeChanged));
        assert!(kinds.contains(&DiffKind::AttributeMissing));
        assert!(kinds.contains(&DiffKind::AttributeAdded));
        assert!(kinds.contains(&DiffKind::ChildCount));
        assert!(kinds.contains(&DiffKind::Tag));
        let changed = diff.differences.iter().find(|d| d.kind == DiffKind::AttributeChanged);
        assert_eq!(changed.map(|d| d.path.as_str()), Some("R/C[0]"));
        assert_eq!(diff.total, u64::try_from(diff.differences.len()).unwrap());
        assert!(!diff.top(3).is_empty());
    }

    #[test]
    fn defaults_can_be_ignored_or_reported() {
        let a = r#"<DOMFrame index="0" duration="1" keyMode="9728"/>"#;
        let b = r#"<DOMFrame index="0"/>"#;
        assert!(compare_xml(a, b, &DiffOptions::default()).is_empty());
        let strict = DiffOptions { ignore_defaults: false, ..DiffOptions::default() };
        assert_eq!(compare_xml(a, b, &strict).total, 2);
    }

    #[test]
    fn broken_input_and_deep_nesting_are_reported_not_fatal() {
        assert_eq!(compare_xml("<a", "<a/>", &DiffOptions::default()).differences.first().map(|d| d.kind), Some(DiffKind::Parse));
        let deep = format!("{}{}", "<g>".repeat(600), "</g>".repeat(600));
        let diff = compare_xml(&deep, &deep, &DiffOptions::default());
        assert_eq!(diff.differences.first().map(|d| d.kind), Some(DiffKind::TooDeep));
    }
}
