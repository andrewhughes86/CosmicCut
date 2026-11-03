//! Keeping the wire out of models a cut is not making. The wire is a straight line right across
//! the foam, so wherever it goes it cuts everything along that line: a cut fails if the wire, or
//! its kerf, would pass through any other model on the table.

use glam::{DVec2, DVec3};

use crate::cut::Built;
use crate::error::{EngineError, EngineResult};
use crate::model::Model;
use crate::wire::{WirePath, WirePos, u_axis};

/// The wire is checked at least this often along a move, mm and degrees.
const STEP: f64 = 1.0;
const TURN: f64 = 0.5;
/// Most checks along one move.
const MAX_STEPS: usize = 20_000;

/// A model as triangles in the table frame, with its box.
struct Body<'a> {
    name: &'a str,
    tris: Vec<[DVec3; 3]>,
    corners: [DVec3; 8],
    lo: f64,
    hi: f64,
}

impl Body<'_> {
    /// Whether the wire at `p`, with `clear` either side for the kerf, touches the model.
    fn hit(&self, p: &WirePos, clear: f64) -> bool {
        let a = u_axis(p.b);
        if p.h < self.lo - clear || p.h > self.hi + clear {
            return false;
        }
        let (umin, umax) = self
            .corners
            .iter()
            .map(|c| c.dot(a))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), u| (lo.min(u), hi.max(u)));
        if p.u < umin - clear || p.u > umax + clear {
            return false;
        }
        // Seen along the wire, the wire is a point; the model, its triangles.
        let w = DVec2::new(p.u, p.h);
        self.tris.iter().any(|t| {
            let q = t.map(|c| DVec2::new(c.dot(a), c.z));
            inside(w, q) || (clear > 0.0 && [(0, 1), (1, 2), (2, 0)].iter().any(|&(i, j)| near(w, q[i], q[j]) < clear))
        })
    }
}

fn inside(p: DVec2, t: [DVec2; 3]) -> bool {
    let side = |a: DVec2, b: DVec2| (b - a).perp_dot(p - a);
    let (d0, d1, d2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    let area = (t[1] - t[0]).perp_dot(t[2] - t[0]);
    area.abs() > 1.0e-12 && ((d0 > 0.0 && d1 > 0.0 && d2 > 0.0) || (d0 < 0.0 && d1 < 0.0 && d2 < 0.0))
}

/// Distance from `p` to the segment `a`–`b`.
fn near(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 1.0e-18 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn body(m: &Model) -> Body<'_> {
    let tris: Vec<[DVec3; 3]> = m.mesh.triangles().iter().map(|t| m.mesh.corners(*t).map(|c| c.as_dvec3())).collect();
    let b = m.bounds();
    let (lo, hi) = (b.min.as_dvec3(), b.max.as_dvec3());
    let corners = [0, 1, 2, 3, 4, 5, 6, 7].map(|k| {
        DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    });
    Body { name: &m.name, tris, corners, lo: lo.z, hi: hi.z }
}

/// The first place along `path` where the wire touches `b`.
fn first_hit(path: &WirePath, b: &Body<'_>, clear: f64) -> Option<WirePos> {
    let mut at = path.start;
    if b.hit(&at, clear) {
        return Some(at);
    }
    for m in &path.moves {
        let n = ((at.uh().distance(m.to.uh()) / STEP).max((m.to.b - at.b).abs() / TURN).ceil() as usize)
            .clamp(1, MAX_STEPS);
        for k in 1..=n {
            let p = at.lerp(&m.to, k as f64 / n as f64);
            if b.hit(&p, clear) {
                return Some(p);
            }
        }
        at = m.to;
    }
    None
}

/// Fails if the cut's wire (with `clear` either side) would touch any of `others`.
pub fn check(others: &[&Model], built: &Built, clear: f64) -> EngineResult<()> {
    for m in others {
        let b = body(m);
        let hit =
            first_hit(&built.path, &b, clear).or_else(|| built.release.as_ref().and_then(|r| first_hit(r, &b, clear)));
        if let Some(p) = hit {
            return Err(EngineError::InvalidParams(format!(
                "the wire would cut into {} (the table at B {:.1}°, the wire {:.1} mm up): move that model clear, or \
                 for a cut from models, cut it in this cut too",
                b.name, p.b, p.h
            )));
        }
    }
    Ok(())
}
