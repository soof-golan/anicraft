//! Text properties: font, size, colour, alignment, direction (including right-to-left) and spacing.

use anim_core::num;
use anim_scene::{TextAlign, TextDirection, TextElement, TextKind, TextOrientation, TextRun};
use anim_ui::property::{PropValue, Property};

use super::{ReadOnly, Unit, boolean, choice, colour, number, number_row, percent_row, text, wrong};

const KINDS: [(&str, TextKind); 3] = [("Static", TextKind::Static), ("Dynamic", TextKind::Dynamic), ("Input", TextKind::Input)];
const ORIENTATIONS: [(&str, TextOrientation); 3] = [
    ("Horizontal", TextOrientation::Horizontal),
    ("Vertical, left to right", TextOrientation::VerticalLeftToRight),
    ("Vertical, right to left", TextOrientation::VerticalRightToLeft),
];
const DIRECTIONS: [(&str, TextDirection); 3] =
    [("Auto", TextDirection::Auto), ("Left to right", TextDirection::LeftToRight), ("Right to left", TextDirection::RightToLeft)];
const ALIGNS: [(&str, TextAlign); 4] = [("Left", TextAlign::Left), ("Right", TextAlign::Right), ("Center", TextAlign::Center), ("Justify", TextAlign::Justify)];
const SIZE: Unit = Unit::new(1.0, 1000.0, 1, " pt");
const SPACING: Unit = Unit::new(-1000.0, 1000.0, 1, " px");
const MARGIN: Unit = Unit::new(0.0, 10_000.0, 1, " px");
const FIELD: &str = "Text field";
const CHARACTER: &str = "Character";
const PARAGRAPH: &str = "Paragraph";

/// The attributes XFL keeps on the text attributes that the model has no field for; all in pixels.
const LINE_SPACING: &str = "lineSpacing";
const INDENT: &str = "indent";
const LEFT_MARGIN: &str = "leftMargin";
const RIGHT_MARGIN: &str = "rightMargin";

fn names<T: Copy>(table: &[(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(n, _)| *n).collect()
}

/// A paragraph spacing stored in the run's extra attributes; 0 when absent.
fn spacing(run: &TextRun, name: &str) -> f64 {
    run.extra.attr(name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// Stores a paragraph spacing; 0 removes the attribute so files stay as small as they were.
fn set_spacing(run: &mut TextRun, name: &str, v: f64) {
    if v.abs() < 1e-9 {
        run.extra.remove_attr(name);
    } else {
        run.extra.set_attr(name, format!("{v}"));
    }
}

/// The rows of a text field.
pub(super) fn describe(t: &TextElement, out: &mut Vec<Property>) {
    let kind = KINDS.iter().position(|(_, k)| *k == t.kind).unwrap_or(0);
    out.push(Property::new("text_type", "Type", PropValue::choice(kind, &names(&KINDS))).in_section(FIELD));
    if t.kind != TextKind::Static {
        out.push(Property::new("name", "Instance name", PropValue::Text(t.instance_name.clone().unwrap_or_default())).in_section(FIELD));
    }
    let orientation = ORIENTATIONS.iter().position(|(_, o)| *o == t.orientation).unwrap_or(0);
    out.push(Property::new("orientation", "Orientation", PropValue::choice(orientation, &names(&ORIENTATIONS))).in_section(FIELD));
    out.push(Property::new("auto_expand", "Auto-size", PropValue::Bool(t.auto_expand)).in_section(FIELD));
    // The box is shown for information: its size changes with the Width and Height rows.
    out.push(Property::new("box", "Box", PropValue::Text(format!("{:.0} x {:.0} px", t.width, t.height))).in_section(FIELD).read_only());
    out.push(Property::new("text", "Text", PropValue::Text(t.plain_text())).in_section(CHARACTER));
    let Some(run) = t.text_runs.first() else { return };
    out.push(Property::new("font", "Family", PropValue::Text(run.face.clone())).in_section(CHARACTER));
    out.push(number_row("size", "Size", f64::from(run.size), SIZE, CHARACTER));
    out.push(Property::new("text_color", "Colour", PropValue::Color(run.fill_color)).in_section(CHARACTER));
    out.push(percent_row("text_alpha", "Alpha", run.alpha, CHARACTER));
    out.push(Property::new("bold", "Bold", PropValue::Bool(run.bold)).in_section(CHARACTER));
    out.push(Property::new("italic", "Italic", PropValue::Bool(run.italic)).in_section(CHARACTER));
    out.push(number_row("letter_spacing", "Letter spacing", f64::from(run.letter_spacing), SPACING, CHARACTER));
    let align = ALIGNS.iter().position(|(_, a)| *a == run.alignment).unwrap_or(0);
    out.push(Property::new("align", "Align", PropValue::choice(align, &names(&ALIGNS))).in_section(PARAGRAPH));
    let direction = DIRECTIONS.iter().position(|(_, d)| *d == run.direction).unwrap_or(0);
    out.push(Property::new("direction", "Direction", PropValue::choice(direction, &names(&DIRECTIONS))).in_section(PARAGRAPH));
    out.push(number_row("line_spacing", "Line spacing", spacing(run, LINE_SPACING), SPACING, PARAGRAPH));
    out.push(number_row("indent", "Indent", spacing(run, INDENT), SPACING, PARAGRAPH));
    out.push(number_row("margin_left", "Left margin", spacing(run, LEFT_MARGIN), MARGIN, PARAGRAPH));
    out.push(number_row("margin_right", "Right margin", spacing(run, RIGHT_MARGIN), MARGIN, PARAGRAPH));
}

/// Applies a text edit: field-level ids to the element, everything else to every run.
pub(super) fn apply(t: &mut TextElement, id: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(id);
    match id {
        "text_type" => t.kind = KINDS.get(choice(value).ok_or_else(wrong)?).map(|(_, k)| *k).ok_or_else(wrong)?,
        "name" => {
            let name = text(value).ok_or_else(wrong)?;
            t.instance_name = if name.is_empty() { None } else { Some(name) };
        }
        "orientation" => t.orientation = ORIENTATIONS.get(choice(value).ok_or_else(wrong)?).map(|(_, o)| *o).ok_or_else(wrong)?,
        "auto_expand" => t.auto_expand = boolean(value).ok_or_else(wrong)?,
        "text" => {
            let new = text(value).ok_or_else(wrong)?;
            let first = t.text_runs.first_mut().ok_or("the text has no style to keep")?;
            first.characters = new;
            t.text_runs.truncate(1);
        }
        _ => {
            if t.text_runs.is_empty() {
                return Err("the text has no style to change".to_owned());
            }
            for run in &mut t.text_runs {
                apply_run(run, id, value)?;
            }
        }
    }
    Ok(())
}

fn apply_run(run: &mut TextRun, id: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(id);
    match id {
        "font" => run.face = text(value).ok_or_else(wrong)?,
        "size" => run.size = num::f64_to_f32(number(value).filter(|v| *v >= 1.0).ok_or_else(wrong)?),
        "text_color" => run.fill_color = colour(value).ok_or_else(wrong)?,
        "text_alpha" => run.alpha = num::f64_to_f32(number(value).ok_or_else(wrong)?.clamp(0.0, 100.0) / 100.0),
        "bold" => run.bold = boolean(value).ok_or_else(wrong)?,
        "italic" => run.italic = boolean(value).ok_or_else(wrong)?,
        "letter_spacing" => run.letter_spacing = num::f64_to_f32(number(value).ok_or_else(wrong)?),
        "direction" => run.direction = DIRECTIONS.get(choice(value).ok_or_else(wrong)?).map(|(_, d)| *d).ok_or_else(wrong)?,
        "align" => run.alignment = ALIGNS.get(choice(value).ok_or_else(wrong)?).map(|(_, a)| *a).ok_or_else(wrong)?,
        "line_spacing" => set_spacing(run, LINE_SPACING, number(value).ok_or_else(wrong)?),
        "indent" => set_spacing(run, INDENT, number(value).ok_or_else(wrong)?),
        "margin_left" => set_spacing(run, LEFT_MARGIN, number(value).filter(|v| *v >= 0.0).ok_or_else(wrong)?),
        "margin_right" => set_spacing(run, RIGHT_MARGIN, number(value).filter(|v| *v >= 0.0).ok_or_else(wrong)?),
        _ => return Err(format!("text has no property {id:?}")),
    }
    Ok(())
}
