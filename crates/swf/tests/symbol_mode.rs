//! Symbol mode: sprites with their own timelines, played by a real player (Ruffle's exporter steps the movie).
//! Set `ANICRAFT_RUFFLE_EXPORTER` to run it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, clippy::missing_const_for_fn)]

mod common;

use anicraft_swf::movie::{ACTION_STOP, Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::shape::{PathStyle, ShapeBuilder};
use anicraft_swf::style::FillStyle;
use anicraft_swf::types::{CharacterId, Matrix};
use anim_core::kurbo::{Rect, Shape};
use anim_core::{Affine, Rgba8};
use common::render_swf_frames;

const WHITE: [u8; 3] = [255, 255, 255];

fn red_square(movie: &mut Movie) -> CharacterId {
    let mut b = ShapeBuilder::new();
    let fill = b.add_fill(FillStyle::Solid(Rgba8::opaque(255, 0, 0))).unwrap();
    b.add_path(&Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1), PathStyle { fill1: fill, ..PathStyle::default() });
    movie.define_shape(&b.finish()).unwrap()
}

fn mover(x: f64) -> Frame {
    Frame::new(vec![FrameItem::Place(Placement {
        depth: 1,
        is_move: true,
        matrix: Some(Matrix::from_affine(Affine::translate((x, 20.0)))),
        ..Placement::default()
    })])
}

/// A movie clip whose square moves 20 px per frame over four frames (stopping on frame 2 when `stop_on_third`), played
/// on a main timeline that has four frames and places the clip once.
fn clip_movie(stop_on_third: bool) -> Vec<u8> {
    let mut movie = Movie::new(MovieSettings { width: 100.0, height: 100.0, compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let square = red_square(&mut movie);
    let mut frames = vec![
        Frame::new(vec![FrameItem::Place(Placement::new(1, square, Matrix::from_affine(Affine::translate((10.0, 20.0)))))]),
        mover(30.0),
        mover(50.0),
        mover(70.0),
    ];
    if stop_on_third {
        frames[2].items.push(FrameItem::DoAction(ACTION_STOP.to_vec()));
    }
    let clip = movie.define_sprite(&frames).unwrap();
    movie.add_frame(&Frame::new(vec![FrameItem::Place(Placement::new(1, clip, Matrix::IDENTITY))])).unwrap();
    for _ in 0..3 {
        movie.add_frame(&Frame::default()).unwrap();
    }
    movie.finish().unwrap()
}

#[test]
fn a_clip_plays_its_own_timeline_alongside_the_main_one() {
    let Some(frames) = render_swf_frames("clip_plays", &clip_movie(false), 4) else { return };
    for (index, frame) in frames.iter().enumerate() {
        let x = 10 + 20 * index;
        frame.assert_near(x + 5, 25, [255, 0, 0], 3, &format!("frame {index}: the square"));
        // and nowhere else
        for other in (0..4).filter(|o| *o != index) {
            frame.assert_near(10 + 20 * other + 5, 25, WHITE, 3, &format!("frame {index}: empty where frame {other} has the square"));
        }
    }
}

/// A movie clip built from one display list per frame (what an Animate symbol evaluates to), placed on the main
/// timeline with an offset and a colour transform.
#[test]
fn clips_can_be_built_from_display_lists() {
    use anicraft_swf::NodeConverter;
    use anim_core::{ColorTransform, Size};
    use anim_display::{DisplayList, Fill, FillRule, ImageStore, Node, Paint};

    let list = |x: f64| DisplayList {
        size: Size::new(100.0, 100.0),
        background: None,
        nodes: vec![Node::Fill(Fill {
            transform: Affine::translate((x, 0.0)),
            path: Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            rule: FillRule::NonZero,
            paint: Paint::Solid(Rgba8::opaque(255, 0, 0)),
        })],
    };
    let mut movie = Movie::new(MovieSettings { width: 100.0, height: 100.0, compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let mut converter = NodeConverter::new();
    let images = ImageStore::new();
    let clip = converter.define_sprite(&mut movie, &[list(0.0), list(20.0), list(40.0)], &images).unwrap();
    // the clip's three frames share one shape
    assert_eq!(movie.character_count(), 2);
    let mut place = Placement::new(1, clip, Matrix::from_affine(Affine::translate((10.0, 30.0))));
    place.color = Some(anicraft_swf::types::CxForm::from_color_transform(&ColorTransform::alpha(0.5)));
    movie.add_frame(&Frame::new(vec![FrameItem::Place(place)])).unwrap();
    movie.add_frame(&Frame::default()).unwrap();
    movie.add_frame(&Frame::default()).unwrap();
    let Some(frames) = render_swf_frames("clip_from_lists", &movie.finish().unwrap(), 3) else { return };
    for (index, frame) in frames.iter().enumerate() {
        let x = 10 + 20 * index;
        // half transparent red over white
        frame.assert_near(x + 5, 35, [255, 128, 128], 4, &format!("frame {index}"));
    }
    frames[0].assert_near(35, 35, WHITE, 3, "the clip has not moved on yet");
}

#[test]
fn stop_actions_hold_a_clip_on_its_frame() {
    let Some(frames) = render_swf_frames("clip_stops", &clip_movie(true), 4) else { return };
    // frames 2 and 3 of the main timeline both show the clip's frame 2
    frames[2].assert_near(10 + 40 + 5, 25, [255, 0, 0], 3, "held on the third frame");
    frames[3].assert_near(10 + 40 + 5, 25, [255, 0, 0], 3, "still held");
    frames[3].assert_near(10 + 60 + 5, 25, WHITE, 3, "never reaches the fourth frame");
}
