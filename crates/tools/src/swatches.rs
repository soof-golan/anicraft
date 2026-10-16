//! The Swatches panel's palette: the document's `swatchLists` element, default swatches, and the Photoshop `.aco` and
//! `.act` colour table formats.
//!
//! The palette lives in the document as the raw `<swatchLists>` child that the XFL reader keeps in
//! [`Document::extra`], so it survives a save and open round trip without a new field in the engine model.

use std::fmt::Write as _;

use anim_core::Rgba8;
use anim_display::GradientStop;
use anim_scene::Document;
use anim_shape::{FillStyle, GradientFill};
use thiserror::Error;

/// The most swatches a palette keeps (and a colour table file can hold).
pub const MAX_SWATCHES: usize = 1024;

const ROOT: &str = "swatchLists";

/// Why a colour table file could not be read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SwatchFileError {
    #[error("the file is too short to be a colour table")]
    Truncated,
    #[error("unsupported colour table version {0}")]
    Version(u16),
    #[error("the file holds no colours")]
    Empty,
}

/// The colour table formats the panel can load and save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableFormat {
    /// Photoshop colour swatches (`.aco`).
    Aco,
    /// Photoshop colour table (`.act`).
    Act,
}

impl TableFormat {
    /// The format a file name ends in, if it is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let ext = std::path::Path::new(name).extension()?;
        if ext.eq_ignore_ascii_case("aco") {
            Some(Self::Aco)
        } else if ext.eq_ignore_ascii_case("act") {
            Some(Self::Act)
        } else {
            None
        }
    }
}

/// The swatches a new document starts with: greys, the primaries and secondaries, a few mid tones and two gradients.
#[must_use]
pub fn default_swatches() -> Vec<FillStyle> {
    let mut out: Vec<FillStyle> = Vec::new();
    for step in 0..=5_u8 {
        let v = step.saturating_mul(51);
        out.push(FillStyle::Solid(Rgba8::opaque(v, v, v)));
    }
    for (r, g, b) in [
        (255, 0, 0),
        (0, 255, 0),
        (0, 0, 255),
        (255, 255, 0),
        (0, 255, 255),
        (255, 0, 255),
        (255, 128, 0),
        (128, 0, 255),
        (0, 153, 102),
        (153, 51, 51),
        (51, 102, 204),
        (255, 204, 153),
    ] {
        out.push(FillStyle::Solid(Rgba8::opaque(r, g, b)));
    }
    let stops = |a: Rgba8, b: Rgba8| vec![GradientStop { offset: 0.0, color: a }, GradientStop { offset: 1.0, color: b }];
    out.push(FillStyle::LinearGradient(GradientFill { stops: stops(Rgba8::opaque(255, 255, 255), Rgba8::opaque(0, 0, 0)), ..GradientFill::default() }));
    out.push(FillStyle::RadialGradient(GradientFill { stops: stops(Rgba8::opaque(255, 255, 0), Rgba8::opaque(255, 0, 0)), ..GradientFill::default() }));
    out
}

fn is_root(xml: &str) -> bool {
    xml.trim_start().starts_with("<swatchLists")
}

fn parse_colour(node: roxmltree::Node<'_, '_>) -> Option<Rgba8> {
    let color = Rgba8::from_hex(node.attribute("color")?).ok()?;
    let alpha: f32 = node.attribute("alpha").and_then(|a| a.parse().ok()).unwrap_or(1.0);
    Some(color.with_alpha_f32(alpha.clamp(0.0, 1.0)))
}

fn parse_gradient(node: roxmltree::Node<'_, '_>) -> Vec<GradientStop> {
    let mut stops: Vec<GradientStop> = node
        .children()
        .filter(|c| c.has_tag_name("GradientEntry"))
        .filter_map(|c| {
            let color = parse_colour(c)?;
            let ratio: f32 = c.attribute("ratio").and_then(|r| r.parse().ok()).unwrap_or(0.0);
            Some(GradientStop { offset: ratio.clamp(0.0, 1.0), color })
        })
        .collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

fn parse_swatch(node: roxmltree::Node<'_, '_>) -> Option<FillStyle> {
    match node.tag_name().name() {
        "LinearGradient" | "RadialGradient" => {
            let stops = parse_gradient(node);
            if stops.len() < 2 {
                return None;
            }
            let fill = GradientFill { stops, ..GradientFill::default() };
            Some(if node.tag_name().name() == "LinearGradient" { FillStyle::LinearGradient(fill) } else { FillStyle::RadialGradient(fill) })
        }
        _ => parse_colour(node).map(FillStyle::Solid),
    }
}

/// Reads the palette from XML text; `None` when it is not a palette.
#[must_use]
pub fn parse(xml: &str) -> Option<Vec<FillStyle>> {
    let tree = roxmltree::Document::parse(xml).ok()?;
    let root = tree.root_element();
    if root.tag_name().name() != ROOT {
        return None;
    }
    let list = root.children().find(roxmltree::Node::is_element)?;
    Some(list.children().filter(roxmltree::Node::is_element).filter_map(parse_swatch).take(MAX_SWATCHES).collect())
}

fn write_colour(out: &mut String, tag: &str, color: Rgba8, extra: &str) {
    let _ = write!(out, "<{tag} color=\"{}\" alpha=\"{}\"{extra}/>", color.to_hex_rgb().to_ascii_uppercase(), color.alpha_f32());
}

/// The XML text of a palette.
#[must_use]
pub fn to_xml(swatches: &[FillStyle]) -> String {
    let mut out = format!("<{ROOT}><SwatchList>");
    for fill in swatches.iter().take(MAX_SWATCHES) {
        match fill {
            FillStyle::Solid(c) => write_colour(&mut out, "SolidColor", *c, ""),
            FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => {
                let tag = if matches!(fill, FillStyle::LinearGradient(_)) { "LinearGradient" } else { "RadialGradient" };
                let _ = write!(out, "<{tag}>");
                for stop in &g.stops {
                    write_colour(&mut out, "GradientEntry", stop.color, &format!(" ratio=\"{}\"", stop.offset));
                }
                let _ = write!(out, "</{tag}>");
            }
            FillStyle::Bitmap(_) => {}
        }
    }
    let _ = write!(out, "</SwatchList></{ROOT}>");
    out
}

/// The document's palette, or the defaults when it has none.
#[must_use]
pub fn read(doc: &Document) -> Vec<FillStyle> {
    doc.extra.children.iter().find(|c| is_root(c)).and_then(|c| parse(c)).unwrap_or_else(default_swatches)
}

/// Stores a palette in the document, replacing the previous one.
pub fn write(doc: &mut Document, swatches: &[FillStyle]) {
    let xml = to_xml(swatches);
    match doc.extra.children.iter_mut().find(|c| is_root(c)) {
        Some(slot) => *slot = xml,
        None => doc.extra.children.push(xml),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Colour table files
// ---------------------------------------------------------------------------------------------------------------

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    Some(u16::from_be_bytes(bytes.get(at..end)?.try_into().ok()?))
}

fn high_byte(v: u16) -> u8 {
    u8::try_from(v >> 8).unwrap_or(u8::MAX)
}

/// The solid colours of a colour table file.
///
/// # Errors
/// When the file is not a colour table or holds no colours.
pub fn load_table(format: TableFormat, bytes: &[u8]) -> Result<Vec<FillStyle>, SwatchFileError> {
    let colours = match format {
        TableFormat::Act => load_act(bytes)?,
        TableFormat::Aco => load_aco(bytes)?,
    };
    if colours.is_empty() { Err(SwatchFileError::Empty) } else { Ok(colours.into_iter().map(FillStyle::Solid).collect()) }
}

fn load_act(bytes: &[u8]) -> Result<Vec<Rgba8>, SwatchFileError> {
    if bytes.len() < 3 {
        return Err(SwatchFileError::Truncated);
    }
    // A 772 byte file ends with the number of colours in use (and the transparent index).
    let count = if bytes.len() >= 772 { usize::from(be16(bytes, 768).unwrap_or(256)).min(256) } else { (bytes.len() / 3).min(256) };
    Ok(bytes.as_chunks::<3>().0.iter().take(count).map(|c| Rgba8::opaque(c[0], c[1], c[2])).collect())
}

fn load_aco(bytes: &[u8]) -> Result<Vec<Rgba8>, SwatchFileError> {
    let version = be16(bytes, 0).ok_or(SwatchFileError::Truncated)?;
    if version != 1 {
        return Err(SwatchFileError::Version(version));
    }
    let count = usize::from(be16(bytes, 2).ok_or(SwatchFileError::Truncated)?);
    // Version 2 data, when present, follows the version 1 block and carries names; the colours are the same.
    let mut out = Vec::new();
    for i in 0..count.min(MAX_SWATCHES) {
        let at = i.saturating_mul(10).saturating_add(4);
        let space = be16(bytes, at).ok_or(SwatchFileError::Truncated)?;
        let a = be16(bytes, at.saturating_add(2)).ok_or(SwatchFileError::Truncated)?;
        let b = be16(bytes, at.saturating_add(4)).ok_or(SwatchFileError::Truncated)?;
        let c = be16(bytes, at.saturating_add(6)).ok_or(SwatchFileError::Truncated)?;
        match space {
            0 => out.push(Rgba8::opaque(high_byte(a), high_byte(b), high_byte(c))),
            8 => {
                // Grayscale: 0 is black, 10000 is white.
                let v = u8::try_from(u32::from(a.min(10_000)).saturating_mul(255) / 10_000).unwrap_or(u8::MAX);
                out.push(Rgba8::opaque(v, v, v));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The bytes of a colour table file holding the solid swatches of a palette (gradients are skipped).
#[must_use]
pub fn save_table(format: TableFormat, swatches: &[FillStyle]) -> Vec<u8> {
    let colours: Vec<Rgba8> = swatches.iter().filter_map(FillStyle::solid_color).collect();
    match format {
        TableFormat::Act => {
            let mut out = Vec::with_capacity(772);
            for c in colours.iter().take(256) {
                out.extend_from_slice(&[c.r, c.g, c.b]);
            }
            out.resize(768, 0);
            let used = u16::try_from(colours.len().min(256)).unwrap_or(256);
            out.extend_from_slice(&used.to_be_bytes());
            out.extend_from_slice(&u16::MAX.to_be_bytes());
            out
        }
        TableFormat::Aco => {
            let used = colours.len().min(MAX_SWATCHES);
            let mut out = Vec::new();
            out.extend_from_slice(&1_u16.to_be_bytes());
            out.extend_from_slice(&u16::try_from(used).unwrap_or(0).to_be_bytes());
            for c in colours.iter().take(used) {
                out.extend_from_slice(&0_u16.to_be_bytes());
                for v in [c.r, c.g, c.b] {
                    out.extend_from_slice(&u16::from_be_bytes([v, v]).to_be_bytes());
                }
                out.extend_from_slice(&0_u16.to_be_bytes());
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anim_display::{Interpolation, Spread};

    fn palette() -> Vec<FillStyle> {
        vec![
            FillStyle::Solid(Rgba8::opaque(255, 0, 0)),
            FillStyle::Solid(Rgba8::new(0, 128, 255, 128)),
            FillStyle::LinearGradient(GradientFill {
                stops: vec![GradientStop { offset: 0.0, color: Rgba8::opaque(0, 0, 0) }, GradientStop { offset: 1.0, color: Rgba8::opaque(255, 255, 255) }],
                spread: Spread::Pad,
                interpolation: Interpolation::Rgb,
                ..GradientFill::default()
            }),
        ]
    }

    #[test]
    fn xml_round_trips() {
        let back = parse(&to_xml(&palette())).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.first(), palette().first());
        assert!(matches!(back.get(2), Some(FillStyle::LinearGradient(g)) if g.stops.len() == 2));
        let alpha = back.get(1).and_then(FillStyle::solid_color).unwrap();
        assert!((i32::from(alpha.a) - 128).abs() <= 1);
    }

    #[test]
    fn a_document_without_swatches_gets_the_defaults_and_keeps_edits() {
        let mut doc = Document::default();
        assert_eq!(read(&doc), default_swatches());
        write(&mut doc, &palette());
        write(&mut doc, &palette()[..1]);
        assert_eq!(read(&doc).len(), 1);
        assert_eq!(doc.extra.children.iter().filter(|c| is_root(c)).count(), 1);
    }

    #[test]
    fn colour_tables_round_trip() {
        let solids = vec![FillStyle::Solid(Rgba8::opaque(10, 20, 30)), FillStyle::Solid(Rgba8::opaque(200, 100, 50))];
        for format in [TableFormat::Aco, TableFormat::Act] {
            let bytes = save_table(format, &solids);
            assert_eq!(load_table(format, &bytes).unwrap(), solids, "{format:?}");
        }
        assert_eq!(load_table(TableFormat::Aco, &[0, 1]), Err(SwatchFileError::Truncated));
        assert_eq!(load_table(TableFormat::Aco, &[0, 3, 0, 0]), Err(SwatchFileError::Version(3)));
        assert_eq!(TableFormat::from_name("Mine.ACO"), Some(TableFormat::Aco));
    }
}
