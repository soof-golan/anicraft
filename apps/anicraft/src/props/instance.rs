//! Symbol instance properties: name, type, looping, colour effect, blending, visibility and the filter list.

use anim_core::{BlendMode, ColorTransform, Rgba8, num};
use anim_display::Filter;
use anim_scene::{ColorEffect, InstanceFilter, Library, LoopMode, SymbolInstance, SymbolKind};
use anim_ui::property::{PropValue, Property};

use super::{ReadOnly, Unit, boolean, choice, colour, fraction, number, number_row, pct, percent_row, text, wrong};

const BLENDS: [(&str, BlendMode); 12] = [
    ("Normal", BlendMode::Normal),
    ("Layer", BlendMode::Layer),
    ("Multiply", BlendMode::Multiply),
    ("Screen", BlendMode::Screen),
    ("Overlay", BlendMode::Overlay),
    ("Darken", BlendMode::Darken),
    ("Lighten", BlendMode::Lighten),
    ("Hard Light", BlendMode::HardLight),
    ("Difference", BlendMode::Difference),
    ("Add", BlendMode::Add),
    ("Subtract", BlendMode::Subtract),
    ("Invert", BlendMode::Invert),
];
const EFFECTS: [&str; 5] = ["None", "Brightness", "Tint", "Alpha", "Advanced"];
const LOOPS: [&str; 3] = ["Loop", "Play Once", "Single Frame"];
const KINDS: [(&str, SymbolKind); 3] = [("Graphic", SymbolKind::Graphic), ("Movie Clip", SymbolKind::MovieClip), ("Button", SymbolKind::Button)];
const QUALITIES: [&str; 3] = ["Low", "Medium", "High"];
const ADD_FILTER: [&str; 4] = ["Add filter...", "Blur", "Drop Shadow", "Glow"];
/// Channel names of the advanced colour effect, in the order of [`ColorTransform`].
const CHANNELS: [(&str, &str); 4] = [("r", "Red"), ("g", "Green"), ("b", "Blue"), ("a", "Alpha")];
const INSTANCE: &str = "Instance";
const LOOPING: &str = "Looping";
const COLOR: &str = "Color effect";
const DISPLAY: &str = "Display";
const FILTERS: &str = "Filters";

fn colour_choice(effect: ColorEffect) -> usize {
    match effect {
        ColorEffect::None => 0,
        ColorEffect::Brightness(_) => 1,
        ColorEffect::Tint { .. } => 2,
        ColorEffect::Alpha(_) => 3,
        ColorEffect::Advanced(_) => 4,
    }
}

const FRAME: Unit = Unit::new(1.0, 100_000.0, 0, "");
const PERCENT: Unit = Unit::new(0.0, 100.0, 0, " %");
const SIGNED_PERCENT: Unit = Unit::new(-100.0, 100.0, 0, " %");
const OFFSET: Unit = Unit::new(-255.0, 255.0, 0, "");
const BLUR: Unit = Unit::new(0.0, 255.0, 1, " px");
const STRENGTH: Unit = Unit::new(0.0, 1000.0, 0, " %");
const ANGLE: Unit = Unit::new(-360.0, 360.0, 0, " deg");

/// The rows of a symbol instance.
pub(super) fn describe(i: &SymbolInstance, library: &Library, out: &mut Vec<Property>) {
    let symbols: Vec<String> = library.symbols().map(|s| anicraft_xfl::display_name(&s.name).into_owned()).collect();
    let symbol_names: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let current = library.symbols().position(|s| s.name == i.library_item_name).unwrap_or(0);
    out.push(Property::new("symbol", "Symbol", PropValue::choice(current, &symbol_names)).in_section(INSTANCE));
    out.push(Property::new("name", "Instance name", PropValue::Text(i.name.clone().unwrap_or_default())).in_section(INSTANCE));
    let kind = i.effective_kind(library);
    let kind_index = KINDS.iter().position(|(_, k)| *k == kind).unwrap_or(1);
    let kind_names: Vec<&str> = KINDS.iter().map(|(n, _)| *n).collect();
    out.push(Property::new("kind", "Type", PropValue::choice(kind_index, &kind_names)).in_section(INSTANCE));
    out.push(Property::new("visible", "Visible", PropValue::Bool(i.visible)).in_section(INSTANCE));
    out.push(Property::new("cache_as_bitmap", "Cache as bitmap", PropValue::Bool(i.cache_as_bitmap)).in_section(INSTANCE));
    if matches!(kind, SymbolKind::Graphic) {
        let mode = match i.effective_loop_mode() {
            LoopMode::Loop => 0,
            LoopMode::PlayOnce => 1,
            LoopMode::SingleFrame => 2,
        };
        out.push(Property::new("loop", "Looping", PropValue::choice(mode, &LOOPS)).in_section(LOOPING));
        out.push(number_row("first_frame", "First frame", f64::from(i.first_frame) + 1.0, FRAME, LOOPING));
        if matches!(i.effective_loop_mode(), LoopMode::PlayOnce) {
            out.push(number_row("last_frame", "Last frame", f64::from(i.last_frame.unwrap_or(i.first_frame)) + 1.0, FRAME, LOOPING));
        }
    }
    out.push(Property::new("effect", "Style", PropValue::choice(colour_choice(i.color_effect), &EFFECTS)).in_section(COLOR));
    match i.color_effect {
        ColorEffect::Brightness(b) => out.push(number_row("brightness", "Brightness", pct(b), SIGNED_PERCENT, COLOR)),
        ColorEffect::Tint { color, amount } => {
            out.push(Property::new("tint", "Tint colour", PropValue::Color(color)).in_section(COLOR));
            out.push(number_row("tint_amount", "Tint", pct(amount), PERCENT, COLOR));
        }
        ColorEffect::Alpha(a) => out.push(percent_row("alpha", "Alpha", a, COLOR)),
        ColorEffect::Advanced(t) => {
            for (n, (key, label)) in CHANNELS.iter().enumerate() {
                let multiplier = t.mul.get(n).copied().map_or(100.0, pct);
                let offset = t.add.get(n).copied().map_or(0.0, |v| f64::from(v) * 255.0);
                out.push(number_row(&format!("adv_{key}"), &format!("{label} multiplier"), multiplier, SIGNED_PERCENT, COLOR));
                out.push(number_row(&format!("adv_{key}_off"), &format!("{label} offset"), offset, OFFSET, COLOR));
            }
        }
        ColorEffect::None => {}
    }
    let blend = BLENDS.iter().position(|(_, b)| *b == i.blend_mode).unwrap_or(0);
    let names: Vec<&str> = BLENDS.iter().map(|(n, _)| *n).collect();
    out.push(Property::new("blend", "Blending", PropValue::choice(blend, &names)).in_section(DISPLAY));
    out.push(Property::new("filters", "Filters", PropValue::Text(format!("{} applied", i.filters.len()))).in_section(FILTERS).read_only());
    out.push(Property::new("filter.add", "Add", PropValue::choice(0, &ADD_FILTER)).in_section(FILTERS));
    for (n, filter) in i.filters.iter().enumerate() {
        describe_filter(n, filter, out);
    }
}

fn describe_filter(n: usize, filter: &InstanceFilter, out: &mut Vec<Property>) {
    let (title, mapped) = match filter {
        InstanceFilter::Mapped { filter, .. } => (
            match filter {
                Filter::Blur { .. } => "Blur",
                Filter::DropShadow { .. } => "Drop Shadow",
                Filter::Glow { .. } => "Glow",
                Filter::ColorMatrix(_) => "Adjust Color",
            },
            Some(filter),
        ),
        InstanceFilter::Raw(raw) => (raw.tag.strip_suffix("Filter").unwrap_or(&raw.tag), None),
    };
    let section = format!("Filter {}: {title}", n.saturating_add(1));
    let id = |field: &str| format!("filter.{n}.{field}");
    let blur = |out: &mut Vec<Property>, x: f64, y: f64| {
        out.push(number_row(&id("blur_x"), "Blur X", x, BLUR, &section));
        out.push(number_row(&id("blur_y"), "Blur Y", y, BLUR, &section));
    };
    let flags = |out: &mut Vec<Property>, inner: bool, knockout: bool| {
        out.push(Property::new(&id("inner"), "Inner", PropValue::Bool(inner)).in_section(&section));
        out.push(Property::new(&id("knockout"), "Knockout", PropValue::Bool(knockout)).in_section(&section));
    };
    match mapped {
        Some(Filter::Blur { x, y, quality }) => {
            blur(out, *x, *y);
            let level = usize::from((*quality).clamp(1, 3).saturating_sub(1));
            out.push(Property::new(&id("quality"), "Quality", PropValue::choice(level, &QUALITIES)).in_section(&section));
        }
        Some(Filter::DropShadow { color, angle_deg, distance, blur_x, blur_y, strength, inner, knockout, hide_object }) => {
            out.push(Property::new(&id("color"), "Colour", PropValue::Color(*color)).in_section(&section));
            blur(out, *blur_x, *blur_y);
            out.push(number_row(&id("strength"), "Strength", strength * 100.0, STRENGTH, &section));
            out.push(number_row(&id("angle"), "Angle", *angle_deg, ANGLE, &section));
            out.push(number_row(&id("distance"), "Distance", *distance, BLUR, &section));
            flags(out, *inner, *knockout);
            out.push(Property::new(&id("hide_object"), "Hide object", PropValue::Bool(*hide_object)).in_section(&section));
        }
        Some(Filter::Glow { color, blur_x, blur_y, strength, inner, knockout }) => {
            out.push(Property::new(&id("color"), "Colour", PropValue::Color(*color)).in_section(&section));
            blur(out, *blur_x, *blur_y);
            out.push(number_row(&id("strength"), "Strength", strength * 100.0, STRENGTH, &section));
            flags(out, *inner, *knockout);
        }
        Some(Filter::ColorMatrix(_)) => {
            out.push(Property::new(&id("matrix"), "Matrix", PropValue::Text("4 x 5 colour matrix".to_owned())).in_section(&section).read_only());
        }
        None => out.push(Property::new(&id("kept"), "Kept as is", PropValue::Text("not editable, still applied".to_owned())).in_section(&section).read_only()),
    }
    out.push(Property::new(&id("remove"), "Remove", PropValue::Bool(false)).in_section(&section));
}

/// Applies an instance edit.
pub(super) fn apply(i: &mut SymbolInstance, id: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(id);
    match id {
        "name" => {
            let t = text(value).ok_or_else(wrong)?;
            i.name = if t.is_empty() { None } else { Some(t) };
        }
        "kind" => {
            let kind = KINDS.get(choice(value).ok_or_else(wrong)?).map(|(_, k)| *k).ok_or_else(wrong)?;
            i.symbol_type = Some(kind);
            if kind == SymbolKind::Graphic && i.loop_mode.is_none() {
                i.loop_mode = Some(LoopMode::Loop);
            }
        }
        "visible" => i.visible = boolean(value).ok_or_else(wrong)?,
        "cache_as_bitmap" => i.cache_as_bitmap = boolean(value).ok_or_else(wrong)?,
        "loop" => {
            i.loop_mode = Some(match choice(value).ok_or_else(wrong)? {
                0 => LoopMode::Loop,
                1 => LoopMode::PlayOnce,
                2 => LoopMode::SingleFrame,
                _ => return Err(wrong()),
            });
        }
        "first_frame" => i.first_frame = frame_number(value).ok_or_else(wrong)?,
        "last_frame" => i.last_frame = Some(frame_number(value).ok_or_else(wrong)?),
        "effect" => {
            let effect = match choice(value).ok_or_else(wrong)? {
                0 => ColorEffect::None,
                1 => ColorEffect::Brightness(0.0),
                2 => ColorEffect::Tint { color: Rgba8 { r: 255, g: 0, b: 0, a: 255 }, amount: 0.5 },
                3 => ColorEffect::Alpha(1.0),
                4 => ColorEffect::Advanced(i.color),
                _ => return Err(wrong()),
            };
            i.set_color_effect(effect);
        }
        "brightness" => i.set_color_effect(ColorEffect::Brightness(fraction(number(value).ok_or_else(wrong)?, -100.0, 100.0))),
        "alpha" => i.set_color_effect(ColorEffect::Alpha(fraction(number(value).ok_or_else(wrong)?, 0.0, 100.0))),
        "tint" => {
            let ColorEffect::Tint { amount, .. } = i.color_effect else { return Err("set the style to Tint first".to_owned()) };
            i.set_color_effect(ColorEffect::Tint { color: colour(value).ok_or_else(wrong)?, amount });
        }
        "tint_amount" => {
            let ColorEffect::Tint { color, .. } = i.color_effect else { return Err("set the style to Tint first".to_owned()) };
            i.set_color_effect(ColorEffect::Tint { color, amount: fraction(number(value).ok_or_else(wrong)?, 0.0, 100.0) });
        }
        "blend" => i.blend_mode = BLENDS.get(choice(value).ok_or_else(wrong)?).map(|(_, b)| *b).ok_or_else(wrong)?,
        "filter.add" => add_filter(i, choice(value).ok_or_else(wrong)?)?,
        _ => {
            if let Some(rest) = id.strip_prefix("adv_") {
                return advanced(i, rest, value);
            }
            if let Some(rest) = id.strip_prefix("filter.") {
                return edit_filter(i, rest, value);
            }
            return Err(format!("instances have no property {id:?}"));
        }
    }
    Ok(())
}

/// A 1-based frame number as the 0-based frame the model keeps.
fn frame_number(value: &PropValue) -> Option<u32> {
    let n = number(value).filter(|v| *v >= 1.0)?;
    Some(num::round_f64_to_u32(n).saturating_sub(1))
}

/// `adv_r`, `adv_r_off`, ...: one multiplier or offset of the advanced colour effect.
fn advanced(i: &mut SymbolInstance, rest: &str, value: &PropValue) -> Result<(), String> {
    let ColorEffect::Advanced(mut t) = i.color_effect else { return Err("set the style to Advanced first".to_owned()) };
    let v = number(value).ok_or_else(|| wrong(rest))?;
    let (key, offset) = rest.strip_suffix("_off").map_or((rest, false), |k| (k, true));
    let channel = CHANNELS.iter().position(|(k, _)| *k == key).ok_or_else(|| format!("no colour channel {key:?}"))?;
    let ColorTransform { mul, add } = &mut t;
    let slot = if offset { add.get_mut(channel) } else { mul.get_mut(channel) };
    let slot = slot.ok_or_else(|| format!("no colour channel {key:?}"))?;
    *slot = if offset { num::f64_to_f32(v.clamp(-255.0, 255.0) / 255.0) } else { fraction(v, -100.0, 100.0) };
    i.set_color_effect(ColorEffect::Advanced(t));
    Ok(())
}

fn add_filter(i: &mut SymbolInstance, which: usize) -> Result<(), String> {
    let filter = match which {
        0 => return Ok(()),
        1 => Filter::Blur { x: 5.0, y: 5.0, quality: 1 },
        2 => Filter::DropShadow {
            color: Rgba8 { r: 0, g: 0, b: 0, a: 255 },
            angle_deg: 45.0,
            distance: 5.0,
            blur_x: 5.0,
            blur_y: 5.0,
            strength: 1.0,
            inner: false,
            knockout: false,
            hide_object: false,
        },
        3 => Filter::Glow { color: Rgba8 { r: 255, g: 0, b: 0, a: 255 }, blur_x: 5.0, blur_y: 5.0, strength: 1.0, inner: false, knockout: false },
        _ => return Err(format!("no filter number {which}")),
    };
    i.filters.push(InstanceFilter::mapped(filter));
    Ok(())
}

/// `N.field` of a `filter.N.field` id.
fn edit_filter(i: &mut SymbolInstance, rest: &str, value: &PropValue) -> Result<(), String> {
    let (index, field) = rest.split_once('.').ok_or_else(|| format!("no filter property {rest:?}"))?;
    let n: usize = index.parse().map_err(|_| format!("no filter {index:?}"))?;
    if n >= i.filters.len() {
        return Err(format!("the instance has no filter {n}; it has {}", i.filters.len()));
    }
    if field == "remove" {
        if boolean(value).ok_or_else(|| wrong(field))? {
            i.filters.remove(n);
        }
        return Ok(());
    }
    let Some(InstanceFilter::Mapped { filter, .. }) = i.filters.get_mut(n) else {
        return Err("this filter is kept as it was read and cannot be edited".to_owned());
    };
    let bad = || wrong(field);
    let blur_range = |v: f64| (0.0..=255.0).contains(&v);
    match (filter, field) {
        (Filter::Blur { x, .. }, "blur_x") => *x = number(value).filter(|v| blur_range(*v)).ok_or_else(bad)?,
        (Filter::Blur { y, .. }, "blur_y") => *y = number(value).filter(|v| blur_range(*v)).ok_or_else(bad)?,
        (Filter::Blur { quality, .. }, "quality") => {
            *quality = u8::try_from(choice(value).ok_or_else(bad)?.saturating_add(1)).ok().filter(|q| *q <= 3).ok_or_else(bad)?;
        }
        (Filter::DropShadow { color, .. } | Filter::Glow { color, .. }, "color") => *color = colour(value).ok_or_else(bad)?,
        (Filter::DropShadow { blur_x, .. } | Filter::Glow { blur_x, .. }, "blur_x") => *blur_x = number(value).filter(|v| blur_range(*v)).ok_or_else(bad)?,
        (Filter::DropShadow { blur_y, .. } | Filter::Glow { blur_y, .. }, "blur_y") => *blur_y = number(value).filter(|v| blur_range(*v)).ok_or_else(bad)?,
        (Filter::DropShadow { strength, .. } | Filter::Glow { strength, .. }, "strength") => {
            *strength = number(value).filter(|v| (0.0..=1000.0).contains(v)).ok_or_else(bad)? / 100.0;
        }
        (Filter::DropShadow { inner, .. } | Filter::Glow { inner, .. }, "inner") => *inner = boolean(value).ok_or_else(bad)?,
        (Filter::DropShadow { knockout, .. } | Filter::Glow { knockout, .. }, "knockout") => *knockout = boolean(value).ok_or_else(bad)?,
        (Filter::DropShadow { angle_deg, .. }, "angle") => *angle_deg = number(value).ok_or_else(bad)?,
        (Filter::DropShadow { distance, .. }, "distance") => *distance = number(value).filter(|v| blur_range(*v)).ok_or_else(bad)?,
        (Filter::DropShadow { hide_object, .. }, "hide_object") => *hide_object = boolean(value).ok_or_else(bad)?,
        _ => return Err(format!("this filter has no property {field:?}")),
    }
    Ok(())
}
