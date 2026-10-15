SWF writer for [AniCraft](https://github.com/soof-golan/anicraft).

`anicraft-swf` writes SWF files (the format Adobe Animate exports and Flash Player and [Ruffle](https://ruffle.rs)
play). It has three layers:

1. **Low level**: the header (`FWS`, or zlib `CWS`), bit-packed `RECT`, `MATRIX` and `CXFORMWITHALPHA`, tag headers,
   `DefineShape4` with solid, linear, radial and focal gradient and bitmap fills and `LINESTYLE2` lines,
   `DefineBitsLossless2`/`JPEG2`/`JPEG3`, `PlaceObject3` with filters and blend modes, `RemoveObject2`,
   `DefineSprite`, `DefineMorphShape2`, `DefineButton2`, `DefineSound`, `StartSound`,
   `SoundStreamHead2`/`SoundStreamBlock`, `DoAction`, labels, scenes, XMP metadata and `ExportAssets`.
2. **[`Movie`]**: owns the dictionary and the timeline. Definitions and frames are appended in order; identical
   shapes and sprites are written once. Use it for *symbol mode*: sprites with their own timelines, sounds, tweens.
3. **Producers**: [`export_display_lists`] turns one `anim_display::DisplayList` per frame into a frame by frame SWF,
   [`NodeConverter`] does the same per timeline so that symbols (movie clips whose frames are display lists) and the
   main timeline share shapes and bitmaps, and [`edge::shape_from_edges`] turns Animate's edge shapes
   (`anim_shape::EdgeShape`) into native SWF shapes.

# Frame by frame export

```rust
use anicraft_swf::{ExportOptions, export_display_lists};
use anim_core::kurbo::{Rect, Shape};
use anim_core::{Affine, Rgba8, Size};
use anim_display::{DisplayList, Fill, FillRule, ImageStore, Node, Paint};

// ten frames of a red rectangle sliding to the right
let frames: Vec<DisplayList> = (0..10)
    .map(|i| DisplayList {
        size: Size::new(200.0, 100.0),
        background: Some(Rgba8::WHITE),
        nodes: vec![Node::Fill(Fill {
            transform: Affine::translate((10.0 * f64::from(i), 20.0)),
            path: Rect::new(0.0, 0.0, 60.0, 40.0).to_path(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid(Rgba8::opaque(220, 40, 40)),
        })],
    })
    .collect();
let options = ExportOptions::for_list(&frames[0], 24.0);
let out = export_display_lists(&frames, &ImageStore::new(), options)?;
assert_eq!(&out.swf[..3], b"CWS");
assert_eq!(out.report.frames, 10);
// the rectangle is defined once and moved by a small PlaceObject3 per frame
assert_eq!(out.report.characters, 1);
# Ok::<(), anicraft_swf::SwfError>(())
```

Every `Fill`, `Stroke` and `Bitmap` becomes a shape in its own local space and the node's transform goes on its
placement, so shapes that only move are shared across frames. A `Fill` followed by a `Stroke` of the same path is one
shape. Groups become sprites carrying the group's transform, colour transform, blend mode and filters; group masks
become `ClipDepth` masks; meshes are emulated with one masked, transformed copy of their content per triangle. The
[`ExportReport`] lists what SWF could not carry exactly (an unavailable blend mode, a gradient cut to 15 stops, ...).

# Symbol mode

```rust
use anicraft_swf::movie::{ACTION_STOP, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::shape::filled_path_shape;
use anicraft_swf::style::FillStyle;
use anicraft_swf::types::Matrix;
use anicraft_swf::Placement;
use anim_core::kurbo::{Rect, Shape};
use anim_core::{Affine, Rgba8};
use anim_display::FillRule;

let mut movie = Movie::new(MovieSettings::default())?;
let square = filled_path_shape(&Rect::new(0.0, 0.0, 30.0, 30.0).to_path(0.1), FillStyle::Solid(Rgba8::opaque(30, 90, 200)), FillRule::NonZero)?;
let square = movie.define_shape(&square)?;

// a movie clip with a two frame timeline: the square, then the square moved
let clip = movie.define_sprite(&[
    Frame::new(vec![FrameItem::Place(Placement::new(1, square, Matrix::IDENTITY))]),
    Frame::new(vec![FrameItem::Place(Placement {
        depth: 1,
        is_move: true,
        matrix: Some(Matrix::from_affine(Affine::translate((40.0, 0.0)))),
        ..Placement::default()
    })]),
])?;

// the main timeline places the clip and stops
let mut place = Placement::new(1, clip, Matrix::from_affine(Affine::translate((20.0, 20.0))));
place.name = Some("walker".into());
movie.add_frame(&Frame::new(vec![FrameItem::Place(place), FrameItem::DoAction(ACTION_STOP.to_vec())]))?;
let swf = movie.finish()?;
assert_eq!(&swf[..3], b"CWS");
# Ok::<(), anicraft_swf::SwfError>(())
```

Movie clips whose frames are display lists (what evaluating an Animate symbol yields) come from
[`NodeConverter::define_sprite`](export::NodeConverter::define_sprite).

Sounds are passed through: `Movie::define_sound_mp3` for event sounds, and [`sound::split_mp3_stream`] to cut an MP3
into per-frame `SoundStreamBlock`s for a streaming track. Shape tweens come from [`morph::MorphShapeDef`] (two
shapes of the same structure) placed with a `ratio`.

# Conventions

- Coordinates are pixels in the API and twips (1/20 px) in the file. Gradient matrices keep Flash's gradient space
  (+-819.2 px); bitmap fill matrices map image pixels into the shape.
- Hostile input never panics: non-finite and huge coordinates are clamped, long edges are split to fit the 17-bit
  delta limit, gradient stops are reduced to 15, strings with NUL are errors.
- Text arrives as glyph outlines in the display list (shaped, so right-to-left scripts are already correct); no
  `DefineText` or embedded fonts are written.

# Testing

`cargo test -p anicraft-swf` parses every output back with Ruffle's `swf` crate. Set `ANICRAFT_RUFFLE_EXPORTER` to a
built Ruffle `exporter` binary (`cargo +nightly build -p exporter` in a Ruffle checkout) to also render pixels and
compare them (`tests/render_oracle.rs`, `edge_shapes.rs`, `morph.rs`), and `ANICRAFT_SAMPLES` to convert the shapes of
the Adobe-authored sample files (`tests/samples.rs`, plus a contact sheet when `ANICRAFT_SWF_PREVIEW` names a folder).
`cargo run -p anicraft-swf --example demo -- out.swf` writes an animated demo.

[`NodeConverter`]: export::NodeConverter
[`Movie`]: movie::Movie
[`export_display_lists`]: export::export_display_lists
[`ExportReport`]: export::ExportReport
