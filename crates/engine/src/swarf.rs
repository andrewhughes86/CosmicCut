//! Swarf cuts: the wire laid along faces of a model, with X, Y and B moving together.
//!
//! The wire is always a level straight line, so it can follow any face whose level slices are
//! straight lines: tapered, twisted and turning faces as well as flat ones. At each height the
//! selected faces are sliced, a line is fitted to the slice, and the wire goes there: the
//! table turned so the wire lies along the line, and half a kerf off it on the waste side.
//! Faces that line up with each other are cut in one pass; the rest each get their own.
//!
//! A pass comes straight down onto the top of its faces from above the foam, follows them down,
//! and leaves level out through the waste at the bottom.

use glam::{DVec2, DVec3};
use serde_json::Value;

use crate::cut::{Built, Context, Keep, Sweep};
use crate::error::{EngineError, EngineResult};
use crate::model::Model;
use crate::params;
use crate::wire::{MoveKind, WirePos, on_wire, u_axis};

/// Most faces one cut may follow.
pub const MAX_FACES: usize = 10_000;
/// Most heights one pass is sliced at.
const MAX_SAMPLES: usize = 5_000;
/// Slices are taken this far inside a face's top and bottom, so they cross it cleanly, mm.
const INSET: f64 = 1.0e-3;

/// A triangle of the model in the table frame, with its height range.
#[derive(Clone, Copy)]
struct Tri {
    p: [DVec3; 3],
    lo: f64,
    hi: f64,
}

impl Tri {
    fn new(p: [DVec3; 3]) -> Self {
        let lo = p[0].z.min(p[1].z).min(p[2].z);
        let hi = p[0].z.max(p[1].z).max(p[2].z);
        Self { p, lo, hi }
    }

    /// Where the level plane at `h` crosses it: a segment, or nothing.
    fn slice(&self, h: f64) -> Option<(DVec2, DVec2)> {
        if h < self.lo || h > self.hi {
            return None;
        }
        let mut hits = [DVec2::ZERO; 2];
        let mut n = 0;
        for (a, b) in [(self.p[0], self.p[1]), (self.p[1], self.p[2]), (self.p[2], self.p[0])] {
            let (da, db) = (a.z - h, b.z - h);
            if (da >= 0.0) != (db >= 0.0) && n < 2 {
                let t = da / (da - db);
                let q = a.lerp(b, t);
                if let Some(slot) = hits.get_mut(n) {
                    *slot = DVec2::new(q.x, q.y);
                }
                n += 1;
            }
        }
        (n == 2).then_some((hits[0], hits[1]))
    }

    /// Area-weighted outward normal (by the mesh winding).
    fn normal(&self) -> DVec3 {
        (self.p[1] - self.p[0]).cross(self.p[2] - self.p[0])
    }
}

/// A straight line fitted to points in plan: through `c` along unit `d`, and how far the
/// farthest point is off it.
#[derive(Clone, Copy, Debug)]
struct Fit {
    c: DVec2,
    d: DVec2,
    off: f64,
    /// Farthest any point is from the middle along the line.
    span: f64,
}

/// The best straight line through `pts` (principal axis), or None if they are all one point.
fn fit(pts: &[DVec2]) -> Option<Fit> {
    if pts.len() < 2 {
        return None;
    }
    let c = pts.iter().copied().sum::<DVec2>() / pts.len() as f64;
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for p in pts {
        let q = *p - c;
        xx += q.x * q.x;
        xy += q.x * q.y;
        yy += q.y * q.y;
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let d = DVec2::new(angle.cos(), angle.sin());
    let spread = pts.iter().map(|p| (*p - c).dot(d).abs()).fold(0.0, f64::max);
    if spread.is_nan() || spread <= 1.0e-6 {
        return None;
    }
    let n = d.perp();
    let off = pts.iter().map(|p| (*p - c).dot(n).abs()).fold(0.0, f64::max);
    Some(Fit { c, d, off, span: spread })
}

/// The selected faces' triangles, by face.
fn face_triangles(model: &Model, faces: &[usize]) -> Vec<Vec<Tri>> {
    let tris = model.mesh.triangles();
    faces
        .iter()
        .map(|&f| {
            model
                .faces
                .triangles_of(f)
                .filter_map(|t| tris.get(t))
                .map(|idx| {
                    let [a, b, c] = model.mesh.corners(*idx);
                    Tri::new([a.as_dvec3(), b.as_dvec3(), c.as_dvec3()])
                })
                .collect()
        })
        .collect()
}

/// The plan points where `tris` cross height `h`.
fn slice_points(tris: &[Tri], h: f64) -> Vec<DVec2> {
    let mut out = Vec::new();
    for t in tris {
        if let Some((a, b)) = t.slice(h) {
            out.push(a);
            out.push(b);
        }
    }
    out
}

fn range(tris: &[Tri]) -> (f64, f64) {
    tris.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), t| (lo.min(t.lo), hi.max(t.hi)))
}

/// How long a slice must be to show which way a face runs, mm: shorter ones (where a face tapers
/// to a corner) would line up with anything through that corner.
fn telling(tol: f64) -> f64 {
    (8.0 * tol).max(0.5)
}

/// The points where `tris` cross the first of `heights` at which they cross telling-long.
fn telling_slice(tris: &[Tri], heights: impl Iterator<Item = f64>, tol: f64) -> Option<Vec<DVec2>> {
    heights.map(|h| slice_points(tris, h)).find(|pts| fit(pts).is_some_and(|f| 2.0 * f.span >= telling(tol)))
}

/// Whether faces `a` and `b` lie along the same lines where their heights meet or overlap.
fn line_up(a: &[Tri], b: &[Tri], tol: f64) -> bool {
    let ((alo, ahi), (blo, bhi)) = (range(a), range(b));
    let (lo, hi) = (alo.max(blo), ahi.min(bhi));
    let together = |pa: &[DVec2], pb: &[DVec2]| {
        let mut pts = pa.to_vec();
        pts.extend_from_slice(pb);
        fit(&pts).is_some_and(|f| f.off <= tol * 2.0)
    };
    // Inward from an end of a face: where its slices are long enough to tell.
    let inward = |lo: f64, hi: f64, from_top: bool| {
        [0.0, 0.1, 0.25, 0.5].into_iter().map(move |f| {
            let d = INSET + (hi - lo - 2.0 * INSET).max(0.0) * f;
            if from_top { hi - d } else { lo + d }
        })
    };
    if hi - lo > 4.0 * INSET {
        // Overlapping: the same line wherever both cross telling-long.
        let mut told = 0;
        for h in [lo + INSET, (lo + hi) / 2.0, hi - INSET] {
            let (pa, pb) = (slice_points(a, h), slice_points(b, h));
            let long = |p: &[DVec2]| fit(p).is_some_and(|f| 2.0 * f.span >= telling(tol));
            if !(long(&pa) && long(&pb)) {
                continue;
            }
            if !together(&pa, &pb) {
                return false;
            }
            told += 1;
        }
        return told > 0;
    }
    // One standing on the other: the lower one's top against the upper one's bottom.
    let (upper, lower) = if (alo - bhi).abs() <= tol {
        ((a, alo, ahi), (b, blo, bhi))
    } else if (blo - ahi).abs() <= tol {
        ((b, blo, bhi), (a, alo, ahi))
    } else {
        return false;
    };
    let pu = telling_slice(upper.0, inward(upper.1, upper.2, false), tol);
    let pl = telling_slice(lower.0, inward(lower.1, lower.2, true), tol);
    pu.zip(pl).is_some_and(|(pu, pl)| together(&pu, &pl))
}

/// One wire position of a pass, and the face's line it keeps (for the simulation).
struct Station {
    wire: WirePos,
    face_u: f64,
}

/// A pass along faces that line up: the wire's positions from the top down, and which side of
/// it (+u: 1, −u: −1) is waste.
struct Pass {
    stations: Vec<Station>,
    waste: f64,
}

/// The wire's positions down one group of faces.
fn pass(tris: &[Tri], floor: f64, step: f64, tol: f64, kerf: f64) -> EngineResult<(Pass, f64)> {
    let (lo, hi) = range(tris);
    let low = lo.max(floor);
    let below = (floor - lo).max(0.0);
    if hi - low < 2.0 * INSET {
        return Err(EngineError::InvalidParams(
            if hi - lo < 2.0 * INSET {
                "a selected face is level: the wire can only follow faces that rise (cut level faces with Slabs)"
            } else {
                "the selected faces are lower than the wire reaches"
            }
            .into(),
        ));
    }
    let (top, bottom) = (hi - INSET, low + INSET);
    let n = (((top - bottom) / step).ceil() as usize).clamp(1, MAX_SAMPLES);
    let mut stations: Vec<Station> = Vec::with_capacity(n + 1);
    // The table angle at the height above.
    let mut prev: Option<f64> = None;
    let mut waste = 1.0;
    for k in 0..=n {
        let h = top + (bottom - top) * k as f64 / n as f64;
        let pts = slice_points(tris, h);
        let Some(f) = fit(&pts) else { continue };
        if f.off > tol {
            return Err(EngineError::InvalidParams(format!(
                "at {h:.1} mm up, the selected faces are {:.2} mm off a straight line: the wire is straight and \
                 level, so it can only follow faces whose level slices are straight (or raise the tolerance)",
                f.off
            )));
        }
        // Which way is out of the material here, and how steep the face is.
        let normal: DVec3 = tris.iter().filter(|t| t.lo <= h && t.hi >= h).map(Tri::normal).sum();
        let flat = DVec2::new(normal.x, normal.y);
        // The wire along the line: wire_dir(b) = (sin b, cos b).
        let mut b = f.d.x.atan2(f.d.y).to_degrees();
        let ua = |b: f64| {
            let a = u_axis(b);
            DVec2::new(a.x, a.y)
        };
        // Waste on the +u side; where the faces do not say which way is out, carry on as before.
        let flip = if flat.length_squared() > 1.0e-18 {
            flat.dot(ua(b)) < 0.0
        } else {
            prev.is_some_and(|pb| (b - pb - 360.0 * ((b - pb) / 360.0).round()).abs() > 90.0)
        };
        if flip {
            b += 180.0;
        }
        if let Some(pb) = prev {
            b -= 360.0 * ((b - pb) / 360.0).round();
        }
        prev = Some(b);
        // Half a kerf off the face, measured square to it: farther across on a sloping face.
        let slope = (flat.length() / normal.length().max(1.0e-30)).max(0.2);
        let face_u = f.c.dot(ua(b));
        stations.push(Station { wire: WirePos::new(face_u + kerf / 2.0 / slope, h, b), face_u });
    }
    if stations.len() < 2 {
        return Err(EngineError::InvalidParams("the selected faces have no straight slices to follow".into()));
    }
    // Of the two ways round (waste on +u at B, or on −u at B + 180), start nearer B0.
    let wrap = |b: f64| b - 360.0 * (b / 360.0).round();
    let first = stations.first().map_or(0.0, |s| s.wire.b);
    if wrap(first + 180.0).abs() < wrap(first).abs() {
        waste = -1.0;
        for s in &mut stations {
            s.wire = WirePos::new(-s.wire.u, s.wire.h, s.wire.b + 180.0);
            s.face_u = -s.face_u;
        }
    }
    let shift = stations.first().map_or(0.0, |s| s.wire.b - wrap(s.wire.b));
    for s in &mut stations {
        s.wire.b -= shift;
    }
    Ok((Pass { stations, waste }, below))
}

/// The wire's line at height `h` on a pass: `(face u, b)`, held at the top line above it.
fn line_at(stations: &[Station], h: f64) -> Option<(f64, f64)> {
    // Stations run from the top down.
    let k = stations.partition_point(|s| s.wire.h > h);
    match (k.checked_sub(1).and_then(|i| stations.get(i)), stations.get(k)) {
        (Some(a), Some(b)) => {
            let t = if (a.wire.h - b.wire.h).abs() > 1.0e-12 { (a.wire.h - h) / (a.wire.h - b.wire.h) } else { 0.0 };
            Some((a.face_u + (b.face_u - a.face_u) * t, a.wire.b + (b.wire.b - a.wire.b) * t))
        }
        (None, Some(s)) | (Some(s), None) => Some((s.face_u, s.wire.b)),
        (None, None) => None,
    }
}

/// How deep the wire would cut into the rest of the model on this pass, and where, if it would.
fn gouge(all: &[Tri], p: &Pass, step: f64, tol: f64, top: f64) -> Option<(f64, f64)> {
    let lowest = p.stations.last()?.wire.h;
    let n = (((top - lowest) / step).ceil() as usize).clamp(1, MAX_SAMPLES);
    let mut worst: Option<(f64, f64)> = None;
    for k in 0..=n {
        let h = lowest + (top - lowest) * k as f64 / n as f64;
        let Some((u, b)) = line_at(&p.stations, h) else { continue };
        let a = u_axis(b);
        for q in slice_points(all, h) {
            let into = (q.x * a.x + q.y * a.y - u) * p.waste;
            if into > tol * 2.0 + 1.0e-3 && worst.is_none_or(|(d, _)| into > d) {
                worst = Some((into, h));
            }
        }
    }
    worst
}

/// How far the faces' slices may be off a straight line unless the cut says, mm.
pub const DEFAULT_TOLERANCE: f64 = 0.25;

/// The faces a click on `face` picks: it, and the faces joined to it for as long as all of them
/// together still slice into straight lines. A twisted side split into many flat faces comes as
/// one; a corner, or a face curving away, stops it.
pub fn surface(model: &Model, face: usize, tol: f64) -> Vec<usize> {
    let neighbours = model.face_neighbours();
    let mut picked = vec![face];
    let mut tris: Vec<Tri> = face_triangles(model, &[face]).concat();
    let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::from([face]);
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::from([face]);
    while let Some(f) = queue.pop_front() {
        for &g in neighbours.get(f).map_or(&[][..], Vec::as_slice) {
            if picked.len() >= MAX_FACES || !seen.insert(g) {
                continue;
            }
            let next = face_triangles(model, &[g]).concat();
            let (lo, hi) = range(&next);
            // A level face never lines up: the wire cannot follow it.
            if hi - lo > 2.0 * INSET && line_up(&tris, &next, tol) {
                picked.push(g);
                tris.extend(next);
                queue.push_back(g);
            }
        }
    }
    picked
}

/// The face numbers a cut follows, from its `faces` parameter.
pub fn faces_param(p: &Value, model: &Model) -> EngineResult<Vec<usize>> {
    let list = p
        .get("faces")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| EngineError::InvalidParams("select the faces for the wire to follow".into()))?;
    if list.len() > MAX_FACES {
        return Err(EngineError::InvalidParams(format!("a swarf cut follows at most {MAX_FACES} faces")));
    }
    let count = model.faces.faces.len();
    let mut out: Vec<usize> = Vec::with_capacity(list.len());
    for v in list {
        let f = v
            .as_u64()
            .and_then(|f| usize::try_from(f).ok())
            .filter(|f| *f < count)
            .ok_or_else(|| EngineError::InvalidParams(format!("the model has no face {v}")))?;
        if !out.contains(&f) {
            out.push(f);
        }
    }
    Ok(out)
}

/// Feed for a cut move that turns the table as it goes: the F the controller is given applies to
/// X and Y, so it is slowed until no part of the wire across the foam goes faster than `feed`.
fn simultaneous_feed(from: &WirePos, to: &WirePos, feed: f64, half: f64) -> f64 {
    let along = from.uh().distance(to.uh());
    let fastest = [-half, 0.0, half].iter().map(|t| on_wire(from, *t).distance(on_wire(to, *t))).fold(0.0, f64::max);
    if fastest <= along + 1.0e-12 || fastest <= 0.0 { feed } else { (feed * along / fastest).max(1.0) }
}

pub(crate) fn swarf(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = crate::cut::common(cx, p)?;
    let model = match params::opt_str(p, "model")? {
        Some(id) => cx.models.iter().find(|m| m.id == id || m.name == id),
        None => cx.models.first(),
    }
    .ok_or_else(|| EngineError::InvalidParams("import a model first".into()))?;
    let faces = faces_param(p, model)?;
    let step = params::opt_f64(p, "height_step", 0.05, 1000.0)?.unwrap_or(2.0);
    let tol = params::opt_f64(p, "tolerance", 0.001, 50.0)?.unwrap_or(DEFAULT_TOLERANCE);
    let kerf = if c.compensate { c.kerf } else { 0.0 };
    let by_face = face_triangles(model, &faces);

    // Faces that line up are one pass.
    let mut group: Vec<usize> = (0..by_face.len()).collect();
    fn root(g: &[usize], mut i: usize) -> usize {
        while let Some(&up) = g.get(i) {
            if up == i {
                break;
            }
            i = up;
        }
        i
    }
    for i in 0..by_face.len() {
        for j in i + 1..by_face.len() {
            let (ri, rj) = (root(&group, i), root(&group, j));
            if ri != rj
                && let (Some(a), Some(b)) = (by_face.get(i), by_face.get(j))
                && line_up(a, b, tol)
                && let Some(slot) = group.get_mut(rj)
            {
                *slot = ri;
            }
        }
    }
    let mut groups: Vec<Vec<Tri>> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    for (i, tris) in by_face.iter().enumerate() {
        let r = root(&group, i);
        match roots.iter().position(|x| *x == r) {
            Some(k) => {
                if let Some(g) = groups.get_mut(k) {
                    g.extend_from_slice(tris);
                }
            }
            None => {
                roots.push(r);
                groups.push(tris.clone());
            }
        }
    }

    let floor = cx.machine.wire_y0.max(cx.stock.bottom());
    let mut out =
        Built { path: crate::wire::WirePath::new(crate::cut::park(cx.machine, cx.stock, 0.0)), ..Built::default() };
    let mut passes: Vec<Pass> = Vec::new();
    let mut sunk = 0.0_f64;
    for g in &groups {
        let (p, below) = pass(g, floor, step, tol, kerf)?;
        sunk = sunk.max(below);
        passes.push(p);
    }
    // The table turns one way through the passes.
    passes.sort_by(|a, b| {
        let first = |p: &Pass| p.stations.first().map_or(0.0, |s| s.wire.b);
        first(a).total_cmp(&first(b))
    });
    let angles: Vec<f64> = passes.iter().flat_map(|p| p.stations.iter().map(|s| s.wire.b)).collect();
    crate::cut::needs_b(cx, &angles)?;
    if sunk > c.kerf + 1.0e-6 {
        out.warnings.push(format!(
            "the bottom {sunk:.1} mm of the faces is lower than the wire reaches; the wire stops level there"
        ));
    }

    // The rest of the model, to check the wire stays out of it.
    let all: Vec<Tri> = model
        .mesh
        .triangles()
        .iter()
        .map(|idx| {
            let [a, b, c] = model.mesh.corners(*idx);
            Tri::new([a.as_dvec3(), b.as_dvec3(), c.as_dvec3()])
        })
        .collect();
    let model_top = f64::from(model.mesh.bounds().max.z);
    let reach = cx.stock.reach();
    let safe = cx.stock.top() + c.clearance;
    let mut worst: Option<(f64, f64)> = None;
    for p in &passes {
        let (Some(first), Some(last)) = (p.stations.first(), p.stations.last()) else { continue };
        if let Some((d, h)) = gouge(&all, p, step, tol, model_top)
            && worst.is_none_or(|(w, _)| d > w)
        {
            worst = Some((d, h));
        }
        // Down from above the foam onto the top of the faces, down them, out level through the
        // waste at the bottom.
        let above = WirePos::new(first.wire.u, safe, first.wire.b);
        crate::cut::travel(cx, &c, &mut out.path, above, "top");
        out.path.push(MoveKind::Lead, first.wire, c.feed);
        let mut at = first.wire;
        for s in p.stations.iter().skip(1) {
            let feed = simultaneous_feed(&at, &s.wire, c.feed, reach);
            out.path.push(MoveKind::Cut, s.wire, feed);
            at = s.wire;
        }
        let clear = (reach + c.clearance).max(last.wire.u * p.waste + c.clearance);
        out.path.push(MoveKind::Lead, WirePos::new(clear * p.waste, last.wire.h, last.wire.b), c.feed);
        // Stations run down; the simulation looks lines up from the bottom.
        let lines: Vec<WirePos> = p.stations.iter().rev().map(|s| WirePos::new(s.face_u, s.wire.h, s.wire.b)).collect();
        out.keeps.push(Keep {
            b: first.wire.b,
            regions: Vec::new(),
            outside: false,
            at: out.path.moves.len(),
            sweep: Some(Sweep { lines, waste: p.waste }),
        });
    }
    if let Some((d, h)) = worst {
        out.warnings.push(format!(
            "the wire would cut {d:.1} mm into the part {h:.1} mm up: other faces stand in its way along the wire"
        ));
    }
    crate::cut::finish(cx, &c, &mut out);
    crate::cut::check_fits(cx, &mut out);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A block 100 wide, 60 deep and 100 high on the table, its top turned `twist` degrees about
    /// the vertical axis: each side a twisted face made of level straight lines.
    pub(crate) fn twisted_block(twist: f64, rows: usize) -> CosmicCut_geom::Mesh {
        let corner = |k: usize, h: f64| {
            let (x, y) = [(-50.0, -30.0), (50.0, -30.0), (50.0, 30.0), (-50.0, 30.0)][k % 4];
            let a = (twist * h / 100.0).to_radians();
            let (s, c) = a.sin_cos();
            CosmicCut_geom::Vec3::new((x * c - y * s) as f32, (x * s + y * c) as f32, h as f32)
        };
        let mut pts = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for r in 0..=rows {
            for k in 0..4 {
                pts.push(corner(k, 100.0 * r as f64 / rows as f64));
            }
        }
        let at = |r: usize, k: usize| (r * 4 + k % 4) as u32;
        for r in 0..rows {
            for k in 0..4 {
                let (a, b, c, d) = (at(r, k), at(r, k + 1), at(r + 1, k + 1), at(r + 1, k));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        // Bottom (facing down) and top (facing up).
        let t = at(rows, 0);
        tris.push([0, 2, 1]);
        tris.push([0, 3, 2]);
        tris.push([t, t + 1, t + 2]);
        tris.push([t, t + 2, t + 3]);
        CosmicCut_geom::Mesh::new(pts, tris).unwrap_or_else(|_| CosmicCut_geom::Mesh::default())
    }

    fn setup(twist: f64, rows: usize) -> EngineResult<(crate::machine::Machine, crate::stock::Stock, Model)> {
        let stock =
            crate::stock::Stock::block(DVec3::new(300.0, 300.0, 300.0), DVec2::ZERO, 0.0, 0.0).unwrap_or_default();
        let mut model = Model::new("model.1".into(), "twist".into(), String::new(), None, twisted_block(twist, rows))?;
        model.place(0.0);
        Ok((crate::machine::Machine::default(), stock, model))
    }

    /// The model's faces that rise (its sides), by number.
    fn sides(model: &Model) -> Vec<usize> {
        (0..model.faces.faces.len())
            .filter(|&f| {
                let tris = face_triangles(model, &[f]);
                tris.first().is_some_and(|t| {
                    let (lo, hi) = range(t);
                    hi - lo > 1.0
                })
            })
            .collect()
    }

    #[test]
    fn a_twisted_block_is_cut_side_by_side_with_the_table_turning() -> EngineResult<()> {
        let (m, s, model) = setup(30.0, 80)?;
        // However the sides were split into faces, the faces of each side line up: four passes.
        let faces = sides(&model);
        let models = [model];
        let cx = Context { machine: &m, stock: &s, shapes: &[], models: &models };
        let p = serde_json::json!({ "faces": faces, "kerf": 1.0, "height_step": 2.0 });
        let built = swarf(&cx, &p)?;
        assert!(built.warnings.is_empty(), "{:?}", built.warnings);
        // One pass per side, each turning the table through the twist as it goes down.
        assert_eq!(built.keeps.len(), 4);
        for k in &built.keeps {
            let lines = &k.sweep.as_ref().map(|s| s.lines.clone()).unwrap_or_default();
            let (bottom, top) = (lines.first().copied().unwrap_or_default(), lines.last().copied().unwrap_or_default());
            assert!(((top.b - bottom.b).abs() - 30.0).abs() < 0.5, "{} -> {}", bottom.b, top.b);
        }
        // Every cutting move keeps the wire half a kerf off the face: the wire's line at that
        // height runs alongside one of the block's sides, 0.5 mm out.
        let model = &models[0];
        let cuts: Vec<_> = built.path.moves.iter().filter(|q| q.kind == MoveKind::Cut).collect();
        assert!(cuts.len() > 100);
        for q in cuts.iter().step_by(17) {
            let a = u_axis(q.to.b);
            let pts = slice_points(&face_triangles(model, &faces).concat(), q.to.h);
            // All of the block is on the far side of the wire, the nearest of it half a kerf off.
            let gaps: Vec<f64> = pts.iter().map(|p| (q.to.u - (p.x * a.x + p.y * a.y)).abs()).collect();
            let nearest = gaps.iter().copied().fold(f64::INFINITY, f64::min);
            // (less the facets' own bulge, and more where the face leans: half a kerf square to it)
            assert!(nearest > 0.4 && nearest < 0.7, "{nearest} at {:?}", q.to);
        }
        // The waste falls away and the block stays.
        let sweep = built.keeps.iter().filter_map(|k| k.sweep.as_ref());
        let centre = DVec3::new(0.0, 0.0, 50.0);
        let corner = DVec3::new(120.0, 120.0, 50.0);
        let kept = |p: DVec3| sweep.clone().all(|s| s.inside(p) > 0.0);
        assert!(kept(centre) && !kept(corner));
        // The feed is slowed where the table turns, never sped up.
        assert!(cuts.iter().all(|q| q.feed <= m.default_feed + 1e-9));
        assert!(cuts.iter().any(|q| q.feed < m.default_feed * 0.9));
        Ok(())
    }

    #[test]
    fn a_click_on_a_twisted_side_takes_the_whole_side() -> EngineResult<()> {
        let (_, _, model) = setup(45.0, 60)?;
        let rising = sides(&model);
        // However the sides were split into faces, a click on any of them takes its whole side:
        // four sides, and the top and bottom left out.
        let mut picked: Vec<Vec<usize>> = Vec::new();
        for f in &rising {
            if picked.iter().any(|s| s.contains(f)) {
                continue;
            }
            let mut side = surface(&model, *f, DEFAULT_TOLERANCE);
            side.sort_unstable();
            picked.push(side);
        }
        assert_eq!(picked.len(), 4, "{} faces: {picked:?}", model.faces.faces.len());
        assert_eq!(picked.iter().map(Vec::len).sum::<usize>(), rising.len());
        Ok(())
    }

    #[test]
    fn faces_that_are_not_straight_across_are_refused() -> EngineResult<()> {
        // A round column: every level slice of its side is an arc.
        let (n, r) = (48usize, 40.0f32);
        let mut pts = Vec::new();
        for z in [0.0f32, 100.0] {
            for k in 0..n {
                let a = (k as f32 / n as f32) * std::f32::consts::TAU;
                pts.push(CosmicCut_geom::Vec3::new(r * a.cos(), r * a.sin(), z));
            }
        }
        let mut tris = Vec::new();
        for k in 0..n {
            let (a, b) = (k as u32, ((k + 1) % n) as u32);
            let (c, d) = (b + n as u32, a + n as u32);
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        }
        for k in 1..n as u32 - 1 {
            tris.push([0, k + 1, k]);
            tris.push([n as u32, n as u32 + k, n as u32 + k + 1]);
        }
        let mesh = CosmicCut_geom::Mesh::new(pts, tris).unwrap_or_default();
        let (m, s, _) = setup(0.0, 1)?;
        let mut model = Model::new("model.1".into(), "column".into(), String::new(), None, mesh)?;
        model.place(0.0);
        let side: Vec<usize> = sides(&model);
        assert!(!side.is_empty());
        let models = [model];
        let cx = Context { machine: &m, stock: &s, shapes: &[], models: &models };
        // The whole round side as one face can't be followed; its flat facets one by one cut
        // into each other.
        let r = swarf(&cx, &serde_json::json!({ "faces": side }));
        let refused = r.as_ref().err().is_some_and(|e| e.to_string().contains("straight"));
        let warned =
            r.as_ref().is_ok_and(|b| b.warnings.iter().any(|w| w.contains("cut") && w.contains("into the part")));
        assert!(refused || warned, "{:?}", r.map(|b| b.warnings));
        assert!(swarf(&cx, &serde_json::json!({ "faces": [] })).is_err());
        assert!(swarf(&cx, &serde_json::json!({ "faces": [9999] })).is_err());
        Ok(())
    }

    #[test]
    fn a_fitted_line_says_how_straight_the_points_are() {
        let f = fit(&[DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(2.0, 2.0)]);
        assert!(f.is_some_and(|f| f.off < 1e-9 && (f.d.x.abs() - f.d.y.abs()).abs() < 1e-9));
        let bent = fit(&[DVec2::new(0.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(2.0, 0.0)]);
        assert!(bent.is_some_and(|f| f.off > 0.3));
        assert!(fit(&[DVec2::ONE, DVec2::ONE]).is_none());
    }

    #[test]
    fn slices_cross_a_triangle_once() {
        let t = Tri::new([DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 10.0)]);
        let (a, b) = t.slice(5.0).unwrap_or_default();
        assert!((a.distance(b) - 5.0).abs() < 1e-9);
        assert!(t.slice(11.0).is_none());
    }
}
