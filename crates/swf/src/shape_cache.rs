//! A bounded cache of the shapes the exporter has defined, keyed by the content of the `Fill` and `Stroke` nodes they
//! came from. Animations repeat most of their shapes from frame to frame; recognising a repeat skips cleaning the
//! path, building and encoding the shape and looking its bytes up in the movie.

use std::hash::{Hash, Hasher};

use anim_core::kurbo::{BezPath, PathEl};
use anim_display::{Fill, FillRule, Paint, Stroke};
use rustc_hash::{FxHashMap, FxHasher};

use crate::types::CharacterId;

/// Path elements kept at most; the cache starts over when it would hold more.
const MAX_ELEMENTS: usize = 4_000_000;

/// The inputs of one defined shape and the character it became. The transforms are not part of a shape's identity
/// (they go on the placement), so they are neither hashed nor compared.
#[derive(Debug)]
struct Entry {
    fill: Option<Fill>,
    stroke: Option<Stroke>,
    character: CharacterId,
}

impl Entry {
    fn matches(&self, fill: Option<&Fill>, stroke: Option<&Stroke>) -> bool {
        let same_fill = match (&self.fill, fill) {
            (None, None) => true,
            (Some(a), Some(b)) => a.path == b.path && a.rule == b.rule && a.paint == b.paint,
            _ => false,
        };
        let same_stroke = match (&self.stroke, stroke) {
            (None, None) => true,
            (Some(a), Some(b)) => a.path == b.path && a.style == b.style && a.paint == b.paint,
            _ => false,
        };
        same_fill && same_stroke
    }
}

#[derive(Debug, Default)]
pub struct ShapeCache {
    buckets: FxHashMap<u64, Vec<Entry>>,
    elements: usize,
}

impl ShapeCache {
    pub fn get(&self, fill: Option<&Fill>, stroke: Option<&Stroke>) -> Option<CharacterId> {
        let bucket = self.buckets.get(&fingerprint(fill, stroke))?;
        bucket.iter().find(|entry| entry.matches(fill, stroke)).map(|entry| entry.character)
    }

    pub fn insert(&mut self, fill: Option<&Fill>, stroke: Option<&Stroke>, character: CharacterId) {
        let size = fill.map_or(0, |f| f.path.elements().len()).saturating_add(stroke.map_or(0, |s| s.path.elements().len()));
        if self.elements.saturating_add(size) > MAX_ELEMENTS {
            self.buckets.clear();
            self.elements = 0;
        }
        self.elements = self.elements.saturating_add(size);
        self.buckets.entry(fingerprint(fill, stroke)).or_default().push(Entry { fill: fill.cloned(), stroke: stroke.cloned(), character });
    }
}

fn fingerprint(fill: Option<&Fill>, stroke: Option<&Stroke>) -> u64 {
    let mut hasher = FxHasher::default();
    if let Some(fill) = fill {
        hasher.write_u8(matches!(fill.rule, FillRule::NonZero).into());
        hash_path(&mut hasher, &fill.path);
        hash_paint(&mut hasher, &fill.paint);
    }
    if let Some(stroke) = stroke {
        hasher.write_u64(stroke.style.width.to_bits());
        hash_path(&mut hasher, &stroke.path);
        hash_paint(&mut hasher, &stroke.paint);
    }
    hasher.finish()
}

/// Hashes the path's shape; paints that differ only beyond their kind and solid colour share a bucket and are told
/// apart by comparison.
fn hash_paint(hasher: &mut FxHasher, paint: &Paint) {
    match paint {
        Paint::Solid(color) => [color.r, color.g, color.b, color.a].hash(hasher),
        Paint::Linear(_) => hasher.write_u8(1),
        Paint::Radial(_) => hasher.write_u8(2),
        Paint::Bitmap { image, .. } => image.hash(hasher),
    }
}

fn hash_path(hasher: &mut FxHasher, path: &BezPath) {
    let mut point = |p: anim_core::kurbo::Point| {
        hasher.write_u64(p.x.to_bits());
        hasher.write_u64(p.y.to_bits());
    };
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => point(p),
            PathEl::QuadTo(c, p) => {
                point(c);
                point(p);
            }
            PathEl::CurveTo(c1, c2, p) => {
                point(c1);
                point(c2);
                point(p);
            }
            PathEl::ClosePath => point(anim_core::kurbo::Point::ORIGIN),
        }
    }
}
