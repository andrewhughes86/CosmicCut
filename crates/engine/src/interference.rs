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
    /// Whether the wire at `p`, with `clear` either side for the kerf, touches the model. A
    /// negative `clear` asks instead whether the wire goes inside it by more than that: running
    /// along its surface (in a corner, say) does not count.
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
        let edges = [(0, 1), (1, 2), (2, 0)];
        self.tris.iter().any(|t| {
            let q = t.map(|c| DVec2::new(c.dot(a), c.z));
            if clear < 0.0 {
                inside(w, q) && edges.iter().all(|&(i, j)| near(w, q[i], q[j]) > -clear)
            } else {
                inside(w, q) || (clear > 0.0 && edges.iter().any(|&(i, j)| near(w, q[i], q[j]) < clear))
            }
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

fn body<'a>(name: &'a str, tris: Vec<[DVec3; 3]>) -> Body<'a> {
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for c in tris.iter().flatten() {
        lo = lo.min(*c);
        hi = hi.max(*c);
    }
    let corners = [0, 1, 2, 3, 4, 5, 6, 7].map(|k| {
        DVec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        )
    });
    Body { name, tris, corners, lo: lo.z, hi: hi.z }
}

fn triangles(m: &Model, except: &std::collections::HashSet<usize>) -> Vec<[DVec3; 3]> {
    m.mesh
        .triangles()
        .iter()
        .enumerate()
        .filter(|(i, _)| !except.contains(i))
        .map(|(_, t)| m.mesh.corners(*t).map(|c| c.as_dvec3()))
        .collect()
}

/// Models the wire must keep out of, ready to test moves against.
pub struct Obstacles<'a> {
    bodies: Vec<Body<'a>>,
}

impl<'a> Obstacles<'a> {
    pub fn new(models: &[&'a Model]) -> Self {
        let none = std::collections::HashSet::new();
        Self {
            bodies: models.iter().map(|m| body(&m.name, triangles(m, &none))).filter(|b| !b.tris.is_empty()).collect(),
        }
    }

    /// Adds `m` less its triangles `except` (the faces a cut is following, which the wire runs
    /// alongside).
    pub fn with_part(mut self, m: &'a Model, except: &std::collections::HashSet<usize>) -> Self {
        let b = body(&m.name, triangles(m, except));
        if !b.tris.is_empty() {
            self.bodies.push(b);
        }
        self
    }

    /// The first model the wire touches (with `clear` either side) going from `from` to `to`,
    /// and where.
    pub fn along(&self, from: WirePos, to: WirePos, clear: f64) -> Option<(&'a str, WirePos)> {
        let n = ((from.uh().distance(to.uh()) / STEP).max((to.b - from.b).abs() / TURN).ceil() as usize)
            .clamp(1, MAX_STEPS);
        (0..=n).find_map(|k| {
            let p = from.lerp(&to, k as f64 / n as f64);
            self.bodies.iter().find(|b| b.hit(&p, clear)).map(|b| (b.name, p))
        })
    }

    /// The first model the wire touches along `path`.
    pub fn path(&self, path: &WirePath, clear: f64) -> Option<(&'a str, WirePos)> {
        let mut at = path.start;
        for m in &path.moves {
            if let Some(hit) = self.along(at, m.to, clear) {
                return Some(hit);
            }
            at = m.to;
        }
        None
    }
}

/// Fails if the cut's wire (with `clear` either side) would touch any of `others`.
pub fn check(others: &[&Model], built: &Built, clear: f64) -> EngineResult<()> {
    let o = Obstacles::new(others);
    let hit = o.path(&built.path, clear).or_else(|| built.release.as_ref().and_then(|r| o.path(r, clear)));
    if let Some((name, p)) = hit {
        return Err(EngineError::InvalidParams(format!(
            "the wire would cut into {name} (the table at B {:.1}°, the wire {:.1} mm up): move that model clear, or \
             for a cut from models, cut it in this cut too",
            p.b, p.h
        )));
    }
    Ok(())
}
