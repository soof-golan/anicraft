//! Writes a demo animation as a SWF file.
//!
//! ```sh
//! cargo run -p anicraft-swf --example demo -- out.swf
//! ```
//!
//! The animation touches most of what the writer can express: gradient fills, an even-odd star, a masked group,
//! a colour-transformed and blurred group, a nearest-neighbour bitmap, and dashed and gradient strokes.

#![allow(clippy::suboptimal_flops, clippy::missing_const_for_fn)]

use std::error::Error;
use std::f64::consts::TAU;

use anicraft_swf::{ExportOptions, export_display_lists};
use anim_core::kurbo::{BezPath, Circle, Rect, Shape};
use anim_core::{Affine, BlendMode, ColorTransform, Point, Rgba8, Size};
use anim_display::{
    Bitmap, Cap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, Image, ImageStore, Interpolation, Join, Node, Paint, Spread, Stroke,
    StrokeScale, StrokeStyle,
};

const WIDTH: f64 = 480.0;
const HEIGHT: f64 = 270.0;
const FRAMES: u32 = 48;

/// The Flash gradient square (±819.2) mapped onto a `size` x `size` box centred at `centre`.
fn gradient_box(centre: Point, size: f64, rotation: f64) -> Affine {
    let scale = size / 2.0 / anim_display::GRADIENT_EXTENT;
    Affine::translate(centre.to_vec2()) * Affine::rotate(rotation) * Affine::scale(scale)
}

fn stops(colors: &[(f32, Rgba8)]) -> Vec<GradientStop> {
    colors.iter().map(|&(offset, color)| GradientStop { offset, color }).collect()
}

fn fill(path: BezPath, paint: Paint) -> Node {
    Node::Fill(Fill { transform: Affine::IDENTITY, path, rule: FillRule::NonZero, paint })
}

fn star(centre: Point, radius: f64) -> BezPath {
    let mut path = BezPath::new();
    for i in 0..5_u32 {
        let angle = f64::from(i) * 2.0 * TAU / 5.0 - TAU / 4.0;
        let p = Point::new(centre.x + radius * angle.cos(), centre.y + radius * angle.sin());
        if i == 0 { path.move_to(p) } else { path.line_to(p) }
    }
    path.close_path();
    path
}

fn checker() -> Image {
    let mut pixels = Vec::new();
    for y in 0..8_u8 {
        for x in 0..8_u8 {
            let on = (x ^ y) & 1 == 0;
            let (r, g, b) = if on { (255, 200, 40) } else { (40, 40, 90) };
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image { width: 8, height: 8, premultiplied_rgba: pixels }
}

fn frame(t: f64, checker_id: anim_display::ImageId) -> DisplayList {
    let mut nodes = Vec::new();

    // sky: a vertical linear gradient
    let sky = Gradient {
        transform: Affine::translate((WIDTH / 2.0, HEIGHT / 2.0)) * Affine::rotate(TAU / 4.0) * Affine::scale(HEIGHT / 2.0 / anim_display::GRADIENT_EXTENT),
        stops: stops(&[(0.0, Rgba8::opaque(10, 20, 60)), (0.55, Rgba8::opaque(110, 40, 120)), (1.0, Rgba8::opaque(250, 140, 90))]),
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    };
    nodes.push(fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT).to_path(0.1), Paint::Linear(sky)));

    // a bouncing sun: radial gradient with an off-centre focus, a dashed outline drawn over it
    let sun_centre = Point::new(60.0 + t * 360.0, 120.0 + 50.0 * (t * 2.0 * TAU).sin());
    let sun = Gradient {
        transform: gradient_box(sun_centre, 80.0, 0.0),
        stops: stops(&[(0.0, Rgba8::opaque(255, 250, 200)), (0.6, Rgba8::opaque(255, 190, 40)), (1.0, Rgba8::new(255, 120, 0, 0))]),
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.35,
    };
    let sun_path = Circle::new(sun_centre, 38.0).to_path(0.05);
    nodes.push(fill(sun_path.clone(), Paint::Radial(sun)));
    nodes.push(Node::Stroke(Stroke {
        transform: Affine::IDENTITY,
        path: sun_path,
        style: StrokeStyle { width: 2.5, cap: Cap::Round, join: Join::Round, miter_limit: 3.0, scale: StrokeScale::Normal, dashes: vec![6.0, 5.0] },
        paint: Paint::Solid(Rgba8::new(255, 255, 255, 220)),
    }));

    // a spinning pentagram: even-odd, so its middle is a hole; translucent, additive, with a soft shadow
    let pentagram = Node::Fill(Fill {
        transform: Affine::IDENTITY,
        path: star(Point::ORIGIN, 46.0),
        rule: FillRule::EvenOdd,
        paint: Paint::Linear(Gradient {
            transform: gradient_box(Point::ORIGIN, 100.0, 0.0),
            stops: stops(&[(0.0, Rgba8::opaque(80, 255, 220)), (1.0, Rgba8::opaque(60, 120, 255))]),
            spread: Spread::Reflect,
            interpolation: Interpolation::LinearRgb,
            focal: 0.0,
        }),
    });
    nodes.push(Node::Group(Group {
        transform: Affine::translate((380.0, 90.0)) * Affine::rotate(t * TAU),
        color: ColorTransform::alpha(0.85),
        blend: BlendMode::Normal,
        filters: vec![Filter::DropShadow {
            color: Rgba8::new(0, 0, 0, 160),
            angle_deg: 45.0,
            distance: 6.0,
            blur_x: 8.0,
            blur_y: 8.0,
            strength: 1.0,
            inner: false,
            knockout: false,
            hide_object: false,
        }],
        children: vec![pentagram],
        ..Group::default()
    }));

    // stripes seen through a moving circular mask
    let stripes: Vec<Node> = (0..12_u8)
        .map(|i| {
            let hue = f64::from(i) / 12.0;
            let color = Rgba8::opaque(
                anim_core::num::unit_f32_to_u8(anim_core::num::f64_to_f32(0.5 + 0.5 * (hue * TAU).sin())),
                anim_core::num::unit_f32_to_u8(anim_core::num::f64_to_f32(0.5 + 0.5 * (hue * TAU + 2.1).sin())),
                anim_core::num::unit_f32_to_u8(anim_core::num::f64_to_f32(0.5 + 0.5 * (hue * TAU + 4.2).sin())),
            );
            fill(Rect::new(f64::from(i) * 14.0, 0.0, f64::from(i) * 14.0 + 12.0, 70.0).to_path(0.1), Paint::Solid(color))
        })
        .collect();
    let mask_centre = Point::new(34.0 + 100.0 * (0.5 + 0.5 * (t * TAU).sin()), 35.0);
    nodes.push(Node::Group(Group {
        transform: Affine::translate((24.0, 180.0)),
        mask: Some(vec![fill(Circle::new(mask_centre, 32.0).to_path(0.05), Paint::Solid(Rgba8::BLACK))]),
        children: stripes,
        ..Group::default()
    }));

    // a pixel-art bitmap that is not smoothed
    nodes.push(Node::Bitmap(Bitmap {
        transform: Affine::translate((200.0, 175.0)) * Affine::rotate(0.1 * (t * TAU).sin()) * Affine::scale(7.0),
        image: checker_id,
        smoothing: false,
    }));

    // a wave drawn with a gradient stroke
    let mut wave = BezPath::new();
    wave.move_to((20.0, 255.0));
    for i in 1..=24_u8 {
        let x = 20.0 + f64::from(i) * 18.0;
        let y = 250.0 + 10.0 * (f64::from(i) * 0.7 + t * TAU).sin();
        wave.line_to((x, y));
    }
    nodes.push(Node::Stroke(Stroke {
        transform: Affine::IDENTITY,
        path: wave,
        style: StrokeStyle { width: 4.0, cap: Cap::Round, join: Join::Round, miter_limit: 3.0, scale: StrokeScale::Normal, dashes: vec![] },
        paint: Paint::Linear(Gradient {
            transform: gradient_box(Point::new(WIDTH / 2.0, 250.0), WIDTH, 0.0),
            stops: stops(&[(0.0, Rgba8::opaque(255, 90, 90)), (0.5, Rgba8::opaque(255, 240, 90)), (1.0, Rgba8::opaque(90, 200, 255))]),
            spread: Spread::Pad,
            interpolation: Interpolation::Rgb,
            focal: 0.0,
        }),
    }));

    DisplayList { size: Size::new(WIDTH, HEIGHT), background: Some(Rgba8::BLACK), nodes }
}

fn main() -> Result<(), Box<dyn Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "demo.swf".to_owned());
    let mut images = ImageStore::new();
    let checker_id = images.insert(checker());
    let lists: Vec<DisplayList> = (0..FRAMES).map(|i| frame(f64::from(i) / f64::from(FRAMES), checker_id)).collect();
    let first = lists.first().ok_or("no frames")?;
    let output = export_display_lists(&lists, &images, ExportOptions::for_list(first, 24.0))?;
    std::fs::write(&out, &output.swf)?;
    println!("wrote {out}: {} bytes, {:?}", output.swf.len(), output.report);
    Ok(())
}
