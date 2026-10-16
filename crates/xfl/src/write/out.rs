//! A small, deterministic XML emitter: indentation, escaping, attribute lists and number formatting.
//!
//! It never panics and never produces ill-formed XML: characters XML cannot carry are dropped, attribute names that
//! are not XML names are skipped, duplicate attributes keep the first value, and raw fragments are only embedded
//! when they parse (see [`XmlOut::raw`]).

use std::borrow::Cow;

use anim_core::Rgba8;
use anim_scene::Extra;

/// One indentation level in symbol files.
pub const SYMBOL_INDENT: &str = "  ";
/// One indentation level in `DOMDocument.xml`.
pub const DOCUMENT_INDENT: &str = "     ";

/// Namespaces the XFL root elements declare. Raw fragments are checked inside this context.
pub const XFL_NS_DECLARATIONS: &str = r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns="http://ns.adobe.com/xfl/2008/""#;

/// True for characters XML 1.0 allows in text.
const fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

/// Escapes an attribute value for double quotes. Line breaks and tabs become character references so that a parser's
/// attribute-value normalisation does not turn them into spaces.
pub fn escape_attr(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if xml_char(c) => out.push(c),
            _ => {}
        }
    }
}

/// Escapes element text. A carriage return becomes a reference because parsers fold literal ones into line feeds.
pub fn escape_text(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if xml_char(c) => out.push(c),
            _ => {}
        }
    }
}

/// True when `name` is usable as an attribute name (a conservative subset of XML names).
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == ':');
    first_ok && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}

/// A number the way Animate prints one: shortest text that reads back as the same value, no exponent, no `-0`.
#[must_use]
pub fn fmt_f64(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".to_owned();
    }
    value.to_string()
}

/// [`fmt_f64`] for single precision values (read back through `f32`, the text stays short).
#[must_use]
pub fn fmt_f32(value: f32) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".to_owned();
    }
    value.to_string()
}

/// Rounds to `places` decimals before printing, to undo float noise of a unit conversion (twips to pixels).
#[must_use]
pub fn fmt_rounded(value: f64, places: i32) -> String {
    let scale = 10f64.powi(places);
    fmt_f64((value * scale).round() / scale)
}

/// True when two floats are the same for the purpose of "this is the default, leave it out".
#[must_use]
pub fn same_f64(a: f64, b: f64) -> bool {
    (a - b).abs() <= f64::EPSILON * a.abs().max(b.abs()).max(1.0)
}

/// The shortest text for the alpha channel `byte` that reads back (through `f32` and rounding) as `byte`.
/// `None` for an opaque colour, which XFL leaves out.
#[must_use]
pub fn alpha_text(byte: u8) -> Option<String> {
    if byte == 255 {
        return None;
    }
    let exact = f32::from(byte) / 255.0;
    for places in 0..=6usize {
        let text = format!("{exact:.places$}");
        let trimmed = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { text.as_str() };
        let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
        if trimmed.parse::<f32>().is_ok_and(|v| Rgba8::BLACK.with_alpha_f32(v).a == byte) {
            return Some(trimmed.to_owned());
        }
    }
    Some(fmt_f32(exact))
}

/// An attribute list under construction. Names borrow from the model; values are owned text.
#[derive(Debug, Default)]
pub struct Attrs<'a> {
    items: Vec<(&'a str, String)>,
}

impl<'a> Attrs<'a> {
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Adds a modelled attribute. It replaces an unmodelled one of the same name that `extra` added earlier, which
    /// happens when the reader could not parse that attribute's value and kept it raw.
    fn push(mut self, name: &'a str, value: String) -> Self {
        self.items.retain(|(n, _)| *n != name);
        self.items.push((name, value));
        self
    }

    /// A string attribute.
    #[must_use]
    pub fn s(self, name: &'a str, value: &str) -> Self {
        self.push(name, value.to_owned())
    }

    /// A string attribute when `value` is `Some`.
    #[must_use]
    pub fn s_opt(self, name: &'a str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.s(name, v),
            None => self,
        }
    }

    /// A string attribute unless it equals `default` (which a reader assumes when the attribute is absent).
    #[must_use]
    pub fn s_unless(self, name: &'a str, value: &str, default: &str) -> Self {
        if value == default { self } else { self.s(name, value) }
    }

    /// A number.
    #[must_use]
    pub fn f(self, name: &'a str, value: f64) -> Self {
        self.push(name, fmt_f64(value))
    }

    /// A number unless it equals `default`.
    #[must_use]
    pub fn f_unless(self, name: &'a str, value: f64, default: f64) -> Self {
        if same_f64(value, default) { self } else { self.f(name, value) }
    }

    /// A number when `value` is `Some`.
    #[must_use]
    pub fn f_opt(self, name: &'a str, value: Option<f64>) -> Self {
        match value {
            Some(v) => self.f(name, v),
            None => self,
        }
    }

    /// A single precision number.
    #[must_use]
    pub fn f32(self, name: &'a str, value: f32) -> Self {
        self.push(name, fmt_f32(value))
    }

    /// A single precision number unless it equals `default`.
    #[must_use]
    pub fn f32_unless(self, name: &'a str, value: f32, default: f32) -> Self {
        if same_f64(f64::from(value), f64::from(default)) { self } else { self.f32(name, value) }
    }

    /// An unsigned integer.
    #[must_use]
    pub fn u(self, name: &'a str, value: u32) -> Self {
        self.push(name, value.to_string())
    }

    /// An unsigned integer unless it equals `default`.
    #[must_use]
    pub fn u_unless(self, name: &'a str, value: u32, default: u32) -> Self {
        if value == default { self } else { self.u(name, value) }
    }

    /// An unsigned integer when `value` is `Some`.
    #[must_use]
    pub fn u_opt(self, name: &'a str, value: Option<u32>) -> Self {
        match value {
            Some(v) => self.u(name, v),
            None => self,
        }
    }

    /// A 64-bit unsigned integer when `value` is `Some`.
    #[must_use]
    pub fn u64_opt(self, name: &'a str, value: Option<u64>) -> Self {
        match value {
            Some(v) => self.push(name, v.to_string()),
            None => self,
        }
    }

    /// A signed integer unless it equals `default`.
    #[must_use]
    pub fn i_unless(self, name: &'a str, value: i32, default: i32) -> Self {
        if value == default { self } else { self.push(name, value.to_string()) }
    }

    /// `true` or `false`, always written.
    #[must_use]
    pub fn b(self, name: &'a str, value: bool) -> Self {
        self.push(name, value.to_string())
    }

    /// `true` or `false` unless it equals `default`.
    #[must_use]
    pub fn b_unless(self, name: &'a str, value: bool, default: bool) -> Self {
        if value == default { self } else { self.b(name, value) }
    }

    /// `true` or `false` when `value` is `Some`.
    #[must_use]
    pub fn b_opt(self, name: &'a str, value: Option<bool>) -> Self {
        match value {
            Some(v) => self.b(name, v),
            None => self,
        }
    }

    /// `#RRGGBB` (alpha is not part of the text).
    #[must_use]
    pub fn color(self, name: &'a str, value: Rgba8) -> Self {
        self.push(name, value.to_hex_rgb())
    }

    /// `#RRGGBB` unless it equals `default`.
    #[must_use]
    pub fn color_unless(self, name: &'a str, value: Rgba8, default: Rgba8) -> Self {
        if value.r == default.r && value.g == default.g && value.b == default.b { self } else { self.color(name, value) }
    }

    /// An `alpha` attribute for the colour's alpha byte (nothing when opaque).
    #[must_use]
    pub fn alpha(self, name: &'a str, value: Rgba8) -> Self {
        match alpha_text(value.a) {
            Some(text) => self.push(name, text),
            None => self,
        }
    }

    /// Appends the unmodelled attributes of `extra` in their original order. An attribute whose name is already
    /// present, or is not a valid XML name, is skipped so the element stays well formed.
    #[must_use]
    pub fn extra(self, extra: &'a Extra) -> Self {
        self.pairs(&extra.attrs)
    }

    /// Appends attributes given as plain name/value pairs, with the same checks as [`Attrs::extra`].
    #[must_use]
    pub fn pairs(mut self, pairs: &'a [(String, String)]) -> Self {
        for (name, value) in pairs {
            self = self.pair(name, value);
        }
        self
    }

    /// Appends one attribute unless the name is invalid or already present.
    #[must_use]
    pub fn pair(mut self, name: &'a str, value: &str) -> Self {
        if valid_name(name) && !self.items.iter().any(|(n, _)| *n == name) {
            self.items.push((name, value.to_owned()));
        }
        self
    }
}

struct Open {
    tag: &'static str,
    /// The `>` of the start tag has been written (the element has content).
    started: bool,
}

/// The emitter.
pub struct XmlOut {
    buf: String,
    stack: Vec<Open>,
    /// Things left out because they would have broken the output.
    pub warnings: Vec<String>,
    unit: &'static str,
}

impl Default for XmlOut {
    fn default() -> Self {
        Self::new()
    }
}

impl XmlOut {
    /// An emitter indenting two spaces per level, as Animate does in symbol files.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_indent(SYMBOL_INDENT)
    }

    /// An emitter with a given indentation unit (`DOMDocument.xml` uses five spaces per level).
    #[must_use]
    pub const fn with_indent(unit: &'static str) -> Self {
        Self { buf: String::new(), stack: Vec::new(), warnings: Vec::new(), unit }
    }

    /// The text so far. Every opened element must have been closed.
    #[must_use]
    pub fn finish(mut self) -> (String, Vec<String>) {
        while !self.stack.is_empty() {
            self.close();
        }
        (self.buf, self.warnings)
    }

    fn indent(&mut self) {
        for _ in 0..self.stack.len() {
            self.buf.push_str(self.unit);
        }
    }

    fn start_content(&mut self) {
        if let Some(top) = self.stack.last_mut()
            && !top.started
        {
            top.started = true;
            self.buf.push_str(">\n");
        }
    }

    /// Opens `<tag attrs>`. The start tag is completed lazily: an element that gets no children is written `<tag/>`.
    pub fn open(&mut self, tag: &'static str, attrs: &Attrs<'_>) {
        self.start_content();
        self.indent();
        self.buf.push('<');
        self.buf.push_str(tag);
        self.write_attrs(attrs);
        self.stack.push(Open { tag, started: false });
    }

    /// Opens the root element, declaring the XFL namespaces first.
    pub fn open_root(&mut self, tag: &'static str, attrs: &Attrs<'_>) {
        self.start_content();
        self.indent();
        self.buf.push('<');
        self.buf.push_str(tag);
        self.buf.push(' ');
        self.buf.push_str(XFL_NS_DECLARATIONS);
        self.write_attrs(attrs);
        self.stack.push(Open { tag, started: false });
    }

    fn write_attrs(&mut self, attrs: &Attrs<'_>) {
        for (name, value) in &attrs.items {
            self.buf.push(' ');
            self.buf.push_str(name);
            self.buf.push_str("=\"");
            escape_attr(value, &mut self.buf);
            self.buf.push('"');
        }
    }

    /// Closes the innermost open element.
    pub fn close(&mut self) {
        let Some(top) = self.stack.pop() else {
            return;
        };
        if top.started {
            self.indent();
            self.buf.push_str("</");
            self.buf.push_str(top.tag);
            self.buf.push_str(">\n");
        } else {
            self.buf.push_str("/>\n");
        }
    }

    /// An element without children.
    pub fn leaf(&mut self, tag: &'static str, attrs: &Attrs<'_>) {
        self.open(tag, attrs);
        self.close();
    }

    /// An element holding only text, written on one line. Empty text gives an empty element.
    pub fn text_leaf(&mut self, tag: &'static str, attrs: &Attrs<'_>, text: &str) {
        if text.is_empty() {
            self.leaf(tag, attrs);
            return;
        }
        self.start_content();
        self.indent();
        self.buf.push('<');
        self.buf.push_str(tag);
        self.write_attrs(attrs);
        self.buf.push('>');
        escape_text(text, &mut self.buf);
        self.buf.push_str("</");
        self.buf.push_str(tag);
        self.buf.push_str(">\n");
    }

    /// `<tag><![CDATA[text]]></tag>`, splitting any `]]>` inside the text. Text with a carriage return is escaped
    /// instead, because a parser would fold it inside CDATA.
    pub fn cdata_leaf(&mut self, tag: &'static str, text: &str) {
        if text.contains('\r') {
            self.text_leaf(tag, &Attrs::new(), text);
            return;
        }
        self.start_content();
        self.indent();
        self.buf.push('<');
        self.buf.push_str(tag);
        self.buf.push_str("><![CDATA[");
        let clean: String = text.chars().filter(|c| xml_char(*c)).collect();
        self.buf.push_str(&clean.replace("]]>", "]]]]><![CDATA[>"));
        self.buf.push_str("]]></");
        self.buf.push_str(tag);
        self.buf.push_str(">\n");
    }

    /// Embeds an element kept verbatim in the model. It is checked first: a fragment that is not exactly one
    /// well-formed element would corrupt the document, so it is left out and reported.
    pub fn raw(&mut self, what: &str, fragment: &str) {
        if !raw_is_element(fragment) {
            self.warnings.push(format!("{what}: a kept XML fragment is not one well-formed element and was left out"));
            return;
        }
        self.start_content();
        self.indent();
        self.buf.push_str(fragment.trim());
        self.buf.push('\n');
    }
}

/// True when `fragment` is a single well-formed element (in the XFL namespace context).
#[must_use]
pub fn raw_is_element(fragment: &str) -> bool {
    let trimmed = fragment.trim();
    if !trimmed.starts_with('<') || trimmed.starts_with("<?") || trimmed.starts_with("<!") {
        return false;
    }
    let wrapped: Cow<'_, str> = Cow::Owned(format!("<w {XFL_NS_DECLARATIONS}>{trimmed}</w>"));
    crate::xml::parse(&wrapped).is_ok_and(|doc| doc.root_element().children().filter(roxmltree::Node::is_element).count() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short_and_signless_zero() {
        assert_eq!(fmt_f64(1.0), "1");
        assert_eq!(fmt_f64(-0.0), "0");
        assert_eq!(fmt_f64(0.1), "0.1");
        assert_eq!(fmt_f64(f64::NAN), "0");
        assert_eq!(fmt_f32(0.25), "0.25");
        assert_eq!(fmt_rounded(7.000_000_000_000_001, 6), "7");
    }

    #[test]
    fn alpha_text_is_short_and_reads_back() {
        assert_eq!(alpha_text(255), None);
        assert_eq!(alpha_text(0).as_deref(), Some("0"));
        assert_eq!(alpha_text(128).as_deref(), Some("0.5"));
        for byte in 0..=254u8 {
            let text = alpha_text(byte).unwrap();
            let read = Rgba8::BLACK.with_alpha_f32(text.parse().unwrap());
            assert_eq!(read.a, byte, "{text}");
        }
    }

    #[test]
    fn escaping_keeps_control_whitespace() {
        let mut a = String::new();
        escape_attr("a<b>&\"x\"\n\t\r\u{1}", &mut a);
        assert_eq!(a, "a&lt;b&gt;&amp;&quot;x&quot;&#10;&#9;&#13;");
        let mut t = String::new();
        escape_text("x\r\ny < z", &mut t);
        assert_eq!(t, "x&#13;\ny &lt; z");
    }

    #[test]
    fn elements_nest_and_self_close() {
        let mut out = XmlOut::new();
        out.open("a", &Attrs::new().s("k", "v"));
        out.leaf("b", &Attrs::new());
        out.open("c", &Attrs::new());
        out.close();
        out.text_leaf("t", &Attrs::new(), "x<y");
        out.close();
        let (text, warnings) = out.finish();
        assert_eq!(text, "<a k=\"v\">\n  <b/>\n  <c/>\n  <t>x&lt;y</t>\n</a>\n");
        assert!(warnings.is_empty());
    }

    #[test]
    fn duplicate_and_invalid_attribute_names_are_skipped() {
        let mut extra = Extra::new();
        extra.set_attr("name", "dup");
        extra.set_attr("bad name", "x");
        extra.set_attr("ok", "1");
        let attrs = Attrs::new().s("name", "n").extra(&extra);
        let mut out = XmlOut::new();
        out.leaf("e", &attrs);
        assert_eq!(out.finish().0, "<e name=\"n\" ok=\"1\"/>\n");
    }

    #[test]
    fn cdata_splits_terminators() {
        let mut out = XmlOut::new();
        out.cdata_leaf("script", "a]]>b");
        let (text, _) = out.finish();
        let doc = roxmltree::Document::parse(&text).unwrap();
        assert_eq!(doc.root_element().text(), Some("a]]>b"));
    }

    #[test]
    fn raw_fragments_must_be_single_elements() {
        let mut out = XmlOut::new();
        out.open("r", &Attrs::new());
        out.raw("t", "<x a=\"1\"><y/></x>");
        out.raw("t", "<x/><y/>");
        out.raw("t", "<x");
        out.raw("t", "<?pi?>");
        out.raw("t", "<x xsi:type=\"q\"/>");
        let (text, warnings) = out.finish();
        assert_eq!(warnings.len(), 3);
        assert!(text.contains("<x a=\"1\"><y/></x>"));
        assert!(text.contains("xsi:type"));
    }
}
