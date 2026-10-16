//! Hostile input: random display lists (non-finite and huge coordinates, odd gradients, dashes, masks, meshes) must
//! export without panicking, and Ruffle's parser must accept what comes out. Random bytes must not trip the MP3 and
//! tag readers.
//!
//! Every strategy is boxed: the unboxed nested tuples are large enough to overflow a test thread's stack.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, clippy::too_many_lines)]
// proptest's `BoxedStrategy` is built on `Arc`; this is test code and the type never leaves the strategies.
#![allow(clippy::disallowed_types)]

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anicraft_swf::sound::{parse_mp3, split_mp3_stream};
use anicraft_swf::tag::scan_tags;
use anim_core::kurbo::{BezPath, PathEl};
use anim_core::{Affine, BlendMode, ColorTransform, Point, Rgba8, Size};
use anim_display::{
    Bitmap, Cap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, Image, ImageId, ImageStore, Interpolation, Join, Mesh, Node, Paint,
    Spread, Stroke, StrokeScale, StrokeStyle,
};
use proptest::prelude::*;

const BLEND_MODES: [BlendMode; 23] = [
    BlendMode::Normal,
    BlendMode::Layer,
    BlendMode::PassThrough,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Add,
    BlendMode::Subtract,
    BlendMode::Invert,
    BlendMode::Alpha,
    BlendMode::Erase,
];

fn coord() -> BoxedStrategy<f64> {
    prop_oneof![
        6 => -300.0..300.0_f64,
        2 => -1.0e7..1.0e7_f64,
        1 => Just(f64::NAN),
        1 => Just(f64::INFINITY),
        1 => Just(f64::NEG_INFINITY),
        1 => Just(1.0e300),
    ]
    .boxed()
}

fn point() -> BoxedStrategy<Point> {
    (coord(), coord()).prop_map(|(x, y)| Point::new(x, y)).boxed()
}

fn path_el() -> BoxedStrategy<PathEl> {
    prop_oneof![
        point().prop_map(PathEl::MoveTo),
        point().prop_map(PathEl::LineTo),
        (point(), point()).prop_map(|(a, b)| PathEl::QuadTo(a, b)),
        (point(), point(), point()).prop_map(|(a, b, c)| PathEl::CurveTo(a, b, c)),
        Just(PathEl::ClosePath),
    ]
    .boxed()
}

fn path() -> BoxedStrategy<BezPath> {
    // kurbo asserts that paths start with a MoveTo; the exporter's handling of paths that do not is tested in
    // `readback.rs` through deserialisation.
    (point(), prop::collection::vec(path_el(), 0..10))
        .prop_map(|(start, mut elements)| {
            elements.insert(0, PathEl::MoveTo(start));
            BezPath::from_vec(elements)
        })
        .boxed()
}

fn affine() -> BoxedStrategy<Affine> {
    let general = prop::array::uniform6(coord()).prop_map(Affine::new).boxed();
    prop_oneof![
        3 => Just(Affine::IDENTITY),
        3 => (coord(), coord()).prop_map(|(x, y)| Affine::translate((x, y))),
        2 => general,
    ]
    .boxed()
}

fn color() -> BoxedStrategy<Rgba8> {
    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b, a)| Rgba8::new(r, g, b, a)).boxed()
}

fn gradient() -> BoxedStrategy<Gradient> {
    let stop = (prop_oneof![-1.0..2.0_f32, Just(f32::NAN)], color()).prop_map(|(offset, color)| GradientStop { offset, color });
    (affine(), prop::collection::vec(stop, 0..40), 0..3_u8, any::<bool>(), -2.0..2.0_f64)
        .prop_map(|(transform, stops, spread, linear, focal)| Gradient {
            transform,
            stops,
            spread: [Spread::Pad, Spread::Reflect, Spread::Repeat][usize::from(spread)],
            interpolation: if linear { Interpolation::LinearRgb } else { Interpolation::Rgb },
            focal,
        })
        .boxed()
}

fn paint() -> BoxedStrategy<Paint> {
    let bitmap = (0..3_u64, affine(), any::<bool>(), any::<bool>()).prop_map(|(image, transform, repeat, smoothing)| Paint::Bitmap {
        image: ImageId(image),
        transform,
        repeat,
        smoothing,
    });
    prop_oneof![
        3 => color().prop_map(Paint::Solid),
        1 => gradient().prop_map(Paint::Linear),
        1 => gradient().prop_map(Paint::Radial),
        1 => bitmap,
    ]
    .boxed()
}

fn stroke_style() -> BoxedStrategy<StrokeStyle> {
    let width = prop_oneof![0.0..30.0_f64, -5.0..0.0_f64, Just(f64::NAN), Just(1.0e9)];
    let dash = prop_oneof![0.0..20.0_f64, Just(0.0), Just(-1.0), Just(f64::NAN), Just(1.0e-9)];
    (width, 0..3_u8, 0..3_u8, 0..4_u8, prop::collection::vec(dash, 0..4))
        .prop_map(|(width, cap, join, scale, dashes)| StrokeStyle {
            width,
            cap: [Cap::None, Cap::Round, Cap::Square][usize::from(cap)],
            join: [Join::Miter, Join::Round, Join::Bevel][usize::from(join)],
            miter_limit: 3.0,
            scale: [StrokeScale::Normal, StrokeScale::Horizontal, StrokeScale::Vertical, StrokeScale::None][usize::from(scale)],
            dashes,
        })
        .boxed()
}

fn filter() -> BoxedStrategy<Filter> {
    let shadow = (color(), coord(), coord(), coord(), any::<bool>()).prop_map(|(color, angle_deg, distance, blur, inner)| Filter::DropShadow {
        color,
        angle_deg,
        distance,
        blur_x: blur,
        blur_y: blur,
        strength: 1.0,
        inner,
        knockout: false,
        hide_object: false,
    });
    prop_oneof![(coord(), coord(), any::<u8>()).prop_map(|(x, y, quality)| Filter::Blur { x, y, quality }), shadow, Just(Filter::ColorMatrix([0.5; 20])),]
        .boxed()
}

fn leaf() -> BoxedStrategy<Node> {
    let fill = (affine(), path(), any::<bool>(), paint()).prop_map(|(transform, path, even_odd, paint)| {
        Node::Fill(Fill { transform, path, rule: if even_odd { FillRule::EvenOdd } else { FillRule::NonZero }, paint })
    });
    let stroke = (affine(), path(), stroke_style(), paint()).prop_map(|(transform, path, style, paint)| Node::Stroke(Stroke { transform, path, style, paint }));
    let bitmap =
        (affine(), 0..3_u64, any::<bool>()).prop_map(|(transform, image, smoothing)| Node::Bitmap(Bitmap { transform, image: ImageId(image), smoothing }));
    let mesh = (affine(), prop::collection::vec(point(), 0..6), prop::collection::vec(point(), 0..6), prop::collection::vec([0..8_u32, 0..8, 0..8], 0..6))
        .prop_map(|(transform, source, target, triangles)| {
            let content = vec![Node::Fill(Fill {
                transform: Affine::IDENTITY,
                path: BezPath::from_vec(vec![PathEl::MoveTo(Point::new(0.0, 0.0)), PathEl::LineTo(Point::new(5.0, 0.0)), PathEl::LineTo(Point::new(0.0, 5.0))]),
                rule: FillRule::NonZero,
                paint: Paint::Solid(Rgba8::BLACK),
            })];
            Node::Mesh(Mesh { transform, content, source, target, triangles })
        });
    prop_oneof![3 => fill, 2 => stroke, 1 => bitmap, 1 => mesh].boxed()
}

fn node() -> BoxedStrategy<Node> {
    leaf()
        .prop_recursive(3, 24, 4, |inner| {
            (
                affine(),
                prop::collection::vec(inner.clone(), 0..4),
                prop::option::of(prop::collection::vec(inner, 0..3)),
                prop::collection::vec(filter(), 0..3),
                0..23_usize,
            )
                .prop_map(|(transform, children, mask, filters, blend)| {
                    Node::Group(Group {
                        transform,
                        projection: None,
                        color: ColorTransform { mul: [0.5, 2.0, f32::NAN, 1.0], add: [0.1, -3.0, 0.0, f32::INFINITY] },
                        blend: BLEND_MODES[blend],
                        mask,
                        filters,
                        children,
                        key: None,
                    })
                })
                .boxed()
        })
        .boxed()
}

fn display_list() -> BoxedStrategy<DisplayList> {
    prop::collection::vec(node(), 0..6).prop_map(|nodes| DisplayList { size: Size::new(100.0, 100.0), background: Some(Rgba8::WHITE), nodes }).boxed()
}

fn store() -> ImageStore {
    let mut images = ImageStore::new();
    images.insert(Image::new(1, 1, vec![1, 2, 3, 255]).unwrap());
    images.insert(Image::new(3, 2, vec![200; 24]).unwrap());
    // id 2 is deliberately missing: exports referring to it must fail cleanly
    images
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn random_display_lists_export_and_parse(lists in prop::collection::vec(display_list(), 1..4)) {
        let images = store();
        let options = ExportOptions::for_list(&lists[0], 24.0);
        match export_display_lists(&lists, &images, options) {
            Ok(out) => {
                let buf = swf::decompress_swf(&out.swf[..]).expect("decompresses");
                let parsed = swf::parse_swf(&buf).expect("parses");
                let shows = parsed.tags.iter().filter(|t| matches!(t, swf::Tag::ShowFrame)).count();
                prop_assert_eq!(shows, lists.len());
                prop_assert_eq!(usize::from(buf.header.num_frames()), lists.len());
            }
            // the only error random but well-formed lists may legitimately produce
            Err(anicraft_swf::SwfError::MissingImage(_)) => {}
            Err(other) => prop_assert!(false, "unexpected error {other}"),
        }
    }

    #[test]
    fn random_bytes_do_not_break_the_readers(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = scan_tags(&data);
        let _ = parse_mp3(&data);
        let _ = split_mp3_stream(&data, 24.0, 10);
    }

    #[test]
    fn mp3_headers_everywhere_do_not_break_the_parser(prefix in prop::collection::vec(any::<u8>(), 0..64), count in 0..6_usize, tail in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut data = prefix;
        for _ in 0..count {
            let mut frame = vec![0xff, 0xfb, 0x90, 0x00];
            frame.resize(417, 0);
            data.extend_from_slice(&frame);
        }
        data.extend_from_slice(&tail);
        if let Ok(info) = parse_mp3(&data) {
            prop_assert!(info.frames.iter().all(|f| f.offset + f.len <= data.len()));
            prop_assert!(info.frames.windows(2).all(|w| w[0].offset + w[0].len <= w[1].offset));
        }
    }
}
