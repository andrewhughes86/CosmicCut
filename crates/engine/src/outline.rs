//! Profiles: an outline at a table angle (the models' seen along the wire, or a shape's) broken
//! into lines and curves, and the wire's way along the ones chosen. Lines are chosen as chains
//! (runs of lines joined end to end), each cut the way round it is set, with a lead in before it
//! and a lead out after; or, in older cuts, every line but those left out. A model's bottom is
//! left alone unless chosen. Chosen lines that end on the wire's floor (machine Y0) next to each
//! other are joined by a bridge along the floor, so the wire goes from one model to the next
//! without coming out of the foam.

use CosmicCut_geom::offset::signed_area2;
use CosmicCut_geom::region::{self, Shape as Region};
use glam::DVec2;
use serde_json::Value;

use crate::cut::{Built, Common, Context, Keep};
use crate::error::{EngineError, EngineResult};
use crate::wire::{MoveKind, WirePos};

/// Line ids of a cut's second and later passes start at this many times the pass.
pub const PASS: u32 = 100_000;
/// A turn sharper than this, degrees, ends one line and starts the next.
const CORNER: f64 = 25.0;
/// Most lines in one pass.
const MAX_LINES: usize = 5000;
/// Most chains in a cut.
pub const MAX_CHAINS: usize = 500;

/// One line or curve of a pass's outline, where the wire runs along it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub id: u32,
    /// Which wire loop (or open path) it is part of; lines are numbered round it in order.
    pub ring: usize,
    /// Its ring is a closed loop.
    pub closed: bool,
    pub points: Vec<DVec2>,
    /// It runs along the bottom of a model.
    pub bottom: bool,
}

/// A chain of lines to cut, in order, and whether it is cut the other way.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub lines: Vec<u32>,
    pub reverse: bool,
}

/// A cut's `chains`: `[{"lines": [ids], "reverse": bool}]`.
pub fn chains(p: &Value) -> Vec<Chain> {
    p.get("chains")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_CHAINS)
        .filter_map(|c| {
            let lines: Vec<u32> = c
                .get("lines")?
                .as_array()?
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|v| u32::try_from(v).ok())
                .take(MAX_LINES)
                .collect();
            let reverse = c.get("reverse").and_then(Value::as_bool).unwrap_or(false);
            (!lines.is_empty()).then_some(Chain { lines, reverse })
        })
        .collect()
}

/// The outline of `mesh` seen along the wire at table angle `b`, as the wire runs it (half a kerf
/// out when compensating, never below the floor), broken into lines. Ids are `pass * PASS` on.
pub(crate) fn lines(cx: &Context<'_>, c: &Common, mesh: &CosmicCut_geom::Mesh, b: f64, pass: u32) -> Vec<Line> {
    let low = crate::cut::wire_floor(cx, c.kerf);
    let paths = crate::model::clip_above(crate::model::silhouette(mesh, b, false, 0.02), low);
    let loops: Vec<Vec<DVec2>> =
        paths.iter().filter(|q| q.closed && q.points.len() >= 3).map(|q| q.points.clone()).collect();
    from_paths(cx, c, &loops, &[], pass, true)
}

/// Lines of closed outlines (parts inside them, holes and all) as the wire runs them, and of
/// open ones as they are. With `outer_only`, holes are left out (a model's outline has none).
pub(crate) fn from_paths(
    cx: &Context<'_>,
    c: &Common,
    loops: &[Vec<DVec2>],
    open: &[Vec<DVec2>],
    pass: u32,
    outer_only: bool,
) -> Vec<Line> {
    let floor = cx.machine.wire_y0;
    let grow = if c.compensate { c.kerf / 2.0 } else { 0.0 };
    let mut rings: Vec<(Vec<DVec2>, bool)> = Vec::new();
    for shape in region::offset(&crate::shape::nest(loops), grow, 0.01) {
        for (k, l) in shape.into_iter().enumerate() {
            if k > 0 && outer_only {
                break;
            }
            let mut l = crate::cut::above(&l, floor);
            if signed_area2(&l) < 0.0 {
                l.reverse();
            }
            if l.len() >= 3 {
                rings.push((l, k == 0));
            }
        }
    }
    // Left to right, so the ids read across the view.
    rings.sort_by(|a, b| min_x(&a.0).total_cmp(&min_x(&b.0)));
    let mut out = Vec::new();
    let push = |out: &mut Vec<Line>, ring: usize, closed: bool, points: Vec<DVec2>, bottom: bool| {
        let id = pass.saturating_mul(PASS).saturating_add(out.len() as u32);
        out.push(Line { id, ring, closed, points, bottom });
    };
    // A rounded corner's half next to the bottom rises less than its radius.
    let near = grow + 0.05;
    for (ring, (l, outer)) in rings.iter().enumerate() {
        let bottom_y = l.iter().map(|q| q.y).fold(f64::INFINITY, f64::min);
        for points in split(l, floor, c.kerf * 2.0 + 0.5) {
            // Along the underside (left to right round an anticlockwise loop), at its lowest.
            let (Some(a), Some(z)) = (points.first(), points.last()) else { continue };
            // A model's bottom wherever it is; a shape's only where it sits on the floor.
            let resting = outer_only || bottom_y <= floor + near;
            let bottom = *outer && resting && z.x > a.x + 1.0e-6 && points.iter().all(|q| q.y <= bottom_y + near);
            push(&mut out, ring, true, points, bottom);
            if out.len() >= MAX_LINES {
                return out;
            }
        }
    }
    for (k, path) in open.iter().filter(|q| q.len() >= 2).enumerate() {
        for points in split_open(path) {
            push(&mut out, rings.len() + k, false, points, false);
            if out.len() >= MAX_LINES {
                return out;
            }
        }
    }
    out
}

fn min_x(l: &[DVec2]) -> f64 {
    l.iter().map(|q| q.x).fold(f64::INFINITY, f64::min)
}

/// An open path broken where it turns sharply.
fn split_open(l: &[DVec2]) -> Vec<Vec<DVec2>> {
    let mut out = Vec::new();
    let mut current: Vec<DVec2> = l.first().copied().into_iter().collect();
    for w in l.windows(3) {
        let [p, q, r] = w else { continue };
        current.push(*q);
        let (u, v) = (*q - *p, *r - *q);
        if u.perp_dot(v).atan2(u.dot(v)).to_degrees().abs() > CORNER {
            out.push(std::mem::replace(&mut current, vec![*q]));
        }
    }
    current.extend(l.last().copied().filter(|_| l.len() >= 2));
    if current.len() >= 2 {
        out.push(current);
    }
    out
}

/// The lines of a chain picked from line `id`: the run round its loop (or along its path) of
/// lines all along a model's bottom or all not, in order. Lines are (id, ring, closed, bottom).
pub fn chain_at(lines: &[(u32, usize, bool, bool)], id: u32) -> Vec<u32> {
    let Some(&(_, ring, closed, bottom)) = lines.iter().find(|l| l.0 == id) else { return Vec::new() };
    let pass = id / PASS;
    let members: Vec<&(u32, usize, bool, bool)> = lines.iter().filter(|l| l.1 == ring && l.0 / PASS == pass).collect();
    let n = members.len();
    let Some(at) = members.iter().position(|l| l.0 == id) else { return Vec::new() };
    let same = |k: usize| members.get(k).is_some_and(|l| l.3 == bottom);
    if closed && (0..n).all(same) {
        return members.iter().map(|l| l.0).collect();
    }
    // Back to where the run starts, then forward to where it ends.
    let mut start = at;
    for _ in 0..n {
        let prev = if start == 0 { if closed { n - 1 } else { break } } else { start - 1 };
        if !same(prev) {
            break;
        }
        start = prev;
    }
    let mut out = Vec::new();
    let mut k = start;
    for _ in 0..n {
        let Some(l) = members.get(k) else { break };
        if !same(k) {
            break;
        }
        out.push(l.0);
        k += 1;
        if k == n {
            if !closed {
                break;
            }
            k = 0;
        }
        if k == start {
            break;
        }
    }
    out
}

/// Lines' points joined end to end into runs: each line turned to follow on from the last where
/// that joins them, a new run where nothing does.
pub fn join(pieces: &[&[DVec2]]) -> Vec<Vec<DVec2>> {
    let close = |a: DVec2, b: DVec2| a.distance_squared(b) < 1.0e-8;
    let mut runs: Vec<Vec<DVec2>> = Vec::new();
    let mut current: Vec<DVec2> = Vec::new();
    let mut lines_in = 0usize;
    for piece in pieces {
        let (Some(&a), Some(&z)) = (piece.first(), piece.last()) else { continue };
        let end = current.last().copied();
        match end {
            None => current.extend_from_slice(piece),
            Some(e) if close(e, a) => current.extend(piece.iter().skip(1)),
            Some(e) if close(e, z) => current.extend(piece.iter().rev().skip(1)),
            // The first line runs the other way to meet this one.
            Some(_) if lines_in == 1 && current.first().is_some_and(|f| close(*f, a) || close(*f, z)) => {
                current.reverse();
                let e = current.last().copied().unwrap_or(a);
                if close(e, a) {
                    current.extend(piece.iter().skip(1));
                } else {
                    current.extend(piece.iter().rev().skip(1));
                }
            }
            Some(_) => {
                runs.push(std::mem::take(&mut current));
                current.extend_from_slice(piece);
                lines_in = 0;
            }
        }
        lines_in += 1;
    }
    if current.len() >= 2 {
        runs.push(current);
    }
    runs
}

/// A closed loop broken at its corners and where it comes onto or leaves the floor, each piece
/// from one break to the next (sharing its ends with its neighbours), in the loop's order. A
/// corner is where the loop turns more than [`CORNER`] within `window` of it (the kerf offset
/// rounds corners), broken at the middle of the turn.
fn split(l: &[DVec2], floor: f64, window: f64) -> Vec<Vec<DVec2>> {
    let n = l.len();
    let at = |i: usize| l.get(i % n).copied().unwrap_or_default();
    let on_floor = |q: DVec2| q.y <= floor + 1.0e-6;
    let turn: Vec<f64> = (0..n)
        .map(|i| {
            let (p, q, r) = (at(i + n - 1), at(i), at(i + 1));
            let (u, v) = (q - p, r - q);
            u.perp_dot(v).atan2(u.dot(v)).to_degrees()
        })
        .collect();
    let seg = |i: usize| at(i).distance(at(i + 1));
    // Bends: points joined by segments shorter than half the window (a rounded corner). One
    // that turns past a corner within the window breaks at its middle point; a longer one is a
    // curve, broken only where it turns sharply at a point.
    let mut corners: Vec<usize> = (0..n).filter(|&i| turn.get(i).is_some_and(|t| t.abs() > CORNER)).collect();
    if let Some(first) = (0..n).find(|&i| seg(i + n - 1) >= window / 2.0) {
        let mut k = 0;
        while k < n {
            let begin = (first + k) % n;
            let mut run = vec![begin];
            let mut length = 0.0;
            while k + 1 < n && seg((first + k) % n) < window / 2.0 {
                length += seg((first + k) % n);
                k += 1;
                run.push((first + k) % n);
            }
            k += 1;
            let total: f64 = run.iter().filter_map(|&i| turn.get(i)).sum();
            if run.len() > 1 && total.abs() > CORNER && length <= window {
                corners.retain(|c| !run.contains(c));
                corners.extend(run.get(run.len() / 2).copied());
            }
        }
    }
    // Where the loop comes onto or leaves the floor (a segment lies on it when both its ends do);
    // a rounded corner there belongs to the line coming down to the floor.
    let landings: Vec<usize> = (0..n)
        .filter(|&i| {
            let (p, q, r) = (at(i + n - 1), at(i), at(i + 1));
            (on_floor(p) && on_floor(q)) != (on_floor(q) && on_floor(r))
        })
        .collect();
    let near_landing = |i: usize| landings.iter().any(|&k| at(k).distance(at(i)) < window);
    let breaks: Vec<usize> =
        (0..n).filter(|&i| landings.contains(&i) || (corners.contains(&i) && !near_landing(i))).collect();
    if breaks.is_empty() {
        let mut whole = l.to_vec();
        whole.push(at(0));
        return vec![whole];
    }
    let mut out = Vec::new();
    for (k, &from) in breaks.iter().enumerate() {
        let to = breaks.get(k + 1).copied().unwrap_or(breaks.first().copied().unwrap_or(0) + n);
        let to = if to <= from { to + n } else { to };
        out.push((from..=to).map(at).collect());
    }
    out
}

/// A stretch of the wire's path along chosen lines: open (from one end to the other) or round a
/// whole loop, with a lead in before it and out after it.
struct Run {
    points: Vec<DVec2>,
    closed: bool,
    lead_in: Option<DVec2>,
    lead_out: Option<DVec2>,
}

/// The pass's outlines, whole, and the foam they are cut in at angle `b`.
struct Cutter<'a> {
    cx: &'a Context<'a>,
    c: &'a Common,
    b: f64,
    walls: Vec<Vec<DVec2>>,
    range: (f64, f64),
    heights: (f64, f64),
    floor: f64,
    /// Where models' bottoms lie along the floor: a bridge must not run under them.
    bottoms: Vec<(f64, f64)>,
    /// What is part, not waste, in the cut plane: inside the outline's loops (holes left out)
    /// and every model still on the table, seen along the wire.
    parts: Vec<Region>,
}

impl<'a> Cutter<'a> {
    fn new(cx: &'a Context<'a>, c: &'a Common, lines: &[Line], b: f64, ignore: &[String]) -> Self {
        let rings = lines.iter().map(|l| l.ring).max().map_or(0, |r| r + 1);
        let walls: Vec<Vec<DVec2>> = (0..rings)
            .map(|r| {
                let mut v: Vec<DVec2> = Vec::new();
                for l in lines.iter().filter(|l| l.ring == r) {
                    v.extend(l.points.iter().skip(usize::from(!v.is_empty())).copied());
                }
                v
            })
            .collect();
        // The loops' insides (their wire paths are half a kerf out), and the models.
        let grow = if c.compensate { -c.kerf / 2.0 } else { 0.0 };
        let loops: Vec<Vec<DVec2>> = walls
            .iter()
            .enumerate()
            .filter(|(r, w)| w.len() > 3 && lines.iter().any(|l| l.ring == *r && l.closed))
            .map(|(_, w)| w.clone())
            .collect();
        let mut parts = region::offset(&crate::shape::nest(&loops), grow, 0.05);
        let low = cx.stock.bottom() - 1.0;
        for m in cx.models.iter().filter(|m| !cx.ignore.contains(&m.id) && !ignore.contains(&m.id)) {
            let outline = crate::model::clip_above(crate::model::silhouette(&m.mesh, b, false, 0.05), low);
            let rings: Vec<Vec<DVec2>> =
                outline.into_iter().filter(|q| q.closed && q.points.len() >= 3).map(|q| q.points).collect();
            parts.extend(crate::shape::nest(&rings));
        }
        Self {
            cx,
            c,
            b,
            parts,
            walls,
            range: cx.stock.u_range(b),
            heights: (cx.stock.bottom(), cx.stock.top()),
            floor: cx.machine.wire_y0,
            bottoms: lines
                .iter()
                .filter(|l| l.bottom)
                .map(|l| {
                    let xs = l.points.iter().map(|q| q.x);
                    (xs.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max))
                })
                .collect(),
        }
    }

    /// Whether a bridge along the floor from `a` to `z` stays out from under every model.
    fn bridge_clear(&self, a: DVec2, z: DVec2) -> bool {
        let (lo, hi) = (a.x.min(z.x), a.x.max(z.x));
        !self.crosses(a, z) && self.bottoms.iter().all(|(b0, b1)| hi <= b0 + 1.0e-3 || lo >= b1 - 1.0e-3)
    }

    fn at_floor(&self, q: DVec2) -> bool {
        q.y <= self.floor + 1.0e-3
    }

    /// Whether `a`–`z` crosses any outline (touching one at an end does not count) or runs
    /// inside a loop.
    fn crosses(&self, a: DVec2, z: DVec2) -> bool {
        let inside_loop = |q: DVec2| {
            self.walls.iter().filter(|w| w.len() > 3 && w.first() == w.last()).any(|w| {
                // Even-odd rule.
                let mut inside = false;
                for s in w.windows(2) {
                    let (Some(&p), Some(&r)) = (s.first(), s.get(1)) else { continue };
                    if (p.y > q.y) != (r.y > q.y) && q.x < p.x + (q.y - p.y) / (r.y - p.y) * (r.x - p.x) {
                        inside = !inside;
                    }
                }
                inside
            })
        };
        if [0.25, 0.5, 0.75].iter().any(|t| inside_loop(a.lerp(z, *t))) {
            return true;
        }
        self.walls.iter().any(|w| {
            w.windows(2).any(|s| {
                let (Some(&p), Some(&q)) = (s.first(), s.get(1)) else { return false };
                let d1 = (z - a).perp_dot(p - a);
                let d2 = (z - a).perp_dot(q - a);
                let d3 = (q - p).perp_dot(a - p);
                let d4 = (q - p).perp_dot(z - p);
                d1 * d2 < -1.0e-12 && d3 * d4 < -1.0e-12
            })
        })
    }

    fn inside(&self, q: DVec2) -> bool {
        let (u0, u1) = self.range;
        q.x > u0 + 1.0e-6 && q.x < u1 - 1.0e-6 && q.y < self.heights.1 - 1.0e-6
    }

    /// A way from `q` out of the foam that crosses no outline (nor runs under a model on the
    /// floor), as the points after `q`, and the side it leaves by: straight out level or up, the
    /// nearest first; else from the floor along it into the gap beside the model, then up.
    fn way_out(&self, q: DVec2, prefer_right: bool) -> (Vec<DVec2>, &'static str) {
        let ((u0, u1), hi) = (self.range, self.heights.1);
        let cl = self.c.clearance;
        let top = hi.max(q.y) + cl;
        let mut sides = [
            ("left", DVec2::new(u0.min(q.x) - cl, q.y)),
            ("right", DVec2::new(u1.max(q.x) + cl, q.y)),
            ("top", DVec2::new(q.x, top)),
        ];
        sides.sort_by(|a, b| {
            let lean = |s: &str| f64::from(u8::from((s == "right") != prefer_right && s != "top"));
            (a.1.distance(q) + lean(a.0) * 1.0e-3).total_cmp(&(b.1.distance(q) + lean(b.0) * 1.0e-3))
        });
        let level = |o: DVec2| if self.at_floor(q) { self.bridge_clear(q, o) } else { !self.crosses(q, o) };
        let clear = |side: &str, o: DVec2| if side == "top" { !self.crosses(q, o) } else { level(o) };
        if let Some((side, o)) = sides.iter().find(|(side, o)| clear(side, *o)) {
            return (vec![*o], side);
        }
        if self.at_floor(q) {
            // Into the middle of the gap to the next model's bottom along the floor, then up.
            for dir in [1.0, -1.0] {
                let next = self
                    .bottoms
                    .iter()
                    .map(|(b0, b1)| if dir > 0.0 { *b0 } else { *b1 })
                    .filter(|x| (x - q.x) * dir > 1.0e-3)
                    .min_by(|a, b| ((a - q.x) * dir).total_cmp(&((b - q.x) * dir)));
                let x = next.map_or(q.x + dir * cl, |x| (q.x + x) / 2.0);
                let (gap, up) = (DVec2::new(x, q.y), DVec2::new(x, top));
                if self.bridge_clear(q, gap) && !self.crosses(gap, up) {
                    return (vec![gap, up], "top");
                }
            }
        }
        let (side, o) = sides.first().copied().unwrap_or(("left", q));
        (vec![o], side)
    }

    /// Joins open runs that end and start on the floor next to each other with a bridge along it.
    fn bridged(&self, runs: Vec<Run>) -> Vec<Run> {
        let mut out: Vec<Run> = Vec::new();
        for run in runs {
            match out.last_mut() {
                Some(prev)
                    if !prev.closed
                        && !run.closed
                        && prev.lead_out.is_none()
                        && run.lead_in.is_none()
                        && prev.points.last().copied().is_some_and(|q| self.at_floor(q))
                        && run.points.first().copied().is_some_and(|q| self.at_floor(q))
                        && prev
                            .points
                            .last()
                            .zip(run.points.first())
                            .is_some_and(|(a, z)| self.bridge_clear(*a, *z)) =>
                {
                    prev.points.extend(run.points);
                    prev.lead_out = run.lead_out;
                }
                _ => out.push(run),
            }
        }
        out
    }

    /// Cuts the runs in order: into the foam from outside it, along each, and out again; then
    /// works out what stays for the simulation (see [`Cutter::keep_pieces`]).
    fn cut(&self, runs: Vec<Run>, from_right: bool, out: &mut Built) {
        let (cx, c, b) = (self.cx, self.c, self.b);
        let lead = if from_right { "right" } else { "left" };
        let from = out.path.moves.len();
        for run in runs {
            let (Some(a), Some(z)) = (run.points.first().copied(), run.points.last().copied()) else { continue };
            let start = run.lead_in.unwrap_or(a);
            let end = run.lead_out.unwrap_or(z);
            let entry = self.inside(start).then(|| self.way_out(start, from_right));
            let exit = self.inside(end).then(|| self.way_out(end, !from_right));
            match &entry {
                Some((way, side)) => {
                    // In along the way out, backwards.
                    let s = way.last().copied().unwrap_or(start);
                    crate::cut::travel(cx, c, &mut out.path, WirePos::new(s.x, s.y, b), side);
                    for q in way.iter().rev().skip(1).chain(std::iter::once(&start)) {
                        out.path.push(MoveKind::Lead, WirePos::new(q.x, q.y, b), c.feed);
                    }
                }
                None => crate::cut::travel(cx, c, &mut out.path, WirePos::new(start.x, start.y, b), lead),
            }
            if run.lead_in.is_some() {
                out.path.push(MoveKind::Lead, WirePos::new(a.x, a.y, b), c.feed);
            }
            for q in run.points.iter().skip(1) {
                out.path.push(MoveKind::Cut, WirePos::new(q.x, q.y, b), c.feed);
            }
            if let Some(q) = run.lead_out {
                out.path.push(MoveKind::Lead, WirePos::new(q.x, q.y, b), c.feed);
            }
            for q in exit.iter().flat_map(|(way, _)| way) {
                out.path.push(MoveKind::Lead, WirePos::new(q.x, q.y, b), c.feed);
            }
        }
        self.keep_pieces(from, out);
    }

    /// What stays once the pass is done: the wire's cuts (a kerf wide along every feed move from
    /// `from` on) split the foam in the cut plane into pieces; those holding part of a model or of
    /// the outline's own loops stay, each its own body, and the rest is waste and falls away.
    /// With nothing to go by, the pieces resting on the bottom stay.
    fn keep_pieces(&self, from: usize, out: &mut Built) {
        let (c, b) = (self.c, self.b);
        let ((u0, u1), (lo_h, hi_h)) = (self.range, self.heights);
        let mut paths: Vec<Vec<DVec2>> = Vec::new();
        let mut current: Vec<DVec2> = Vec::new();
        let mut at = from.checked_sub(1).and_then(|k| out.path.moves.get(k)).map_or(out.path.start, |m| m.to);
        for m in out.path.moves.iter().skip(from) {
            if matches!(m.kind, MoveKind::Cut | MoveKind::Lead) && (m.to.b - b).abs() < 1.0e-9 {
                if current.is_empty() {
                    current.push(at.uh());
                }
                current.push(m.to.uh());
            } else if !current.is_empty() {
                paths.push(std::mem::take(&mut current));
            }
            at = m.to;
        }
        if current.len() >= 2 {
            paths.push(current);
        }
        if paths.is_empty() {
            return;
        }
        let band = region::stroke(&paths, c.kerf.max(0.05));
        let section = vec![region::rectangle(DVec2::new(u0, lo_h), DVec2::new(u1, hi_h))];
        let pieces = region::boolean(&section, &band, region::Op::Difference);
        // The parts, a little in from their edges so the cut's own edge does not count.
        let shrink = -c.kerf.max(0.5);
        let parts = region::offset(&self.parts, shrink, 0.05);
        let kept: Vec<Region> = if parts.is_empty() {
            pieces.iter().filter(|piece| piece.iter().flatten().any(|q| q.y <= lo_h + 1.0e-6)).cloned().collect()
        } else {
            pieces
                .iter()
                .filter(|piece| {
                    let both = region::boolean(std::slice::from_ref(*piece), &parts, region::Op::Intersect);
                    region::area(&both) > 1.0e-3
                })
                .cloned()
                .collect()
        };
        if kept.len() == pieces.len() && !pieces.is_empty() {
            // Nothing came loose; only the kerf goes.
            out.keeps.push(Keep {
                b,
                regions: pieces,
                outside: false,
                at: out.path.moves.len(),
                sweep: None,
                apart: true,
            });
            return;
        }
        out.keeps.push(Keep { b, regions: kept, outside: false, at: out.path.moves.len(), sweep: None, apart: true });
    }
}

/// Cuts the kept lines of one pass at table angle `b` (an older cut: every line but those left
/// out). Lines whose rings are kept whole go round as loops; the rest are cut as runs from outside
/// the foam and back, kept lines that meet the floor next to each other joined along it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pass(
    cx: &Context<'_>,
    c: &Common,
    lines: &[Line],
    skip: &[u32],
    bottoms: &[u32],
    through: bool,
    bridge: bool,
    b: f64,
    from_right: bool,
    ignore: &[String],
    out: &mut Built,
) -> EngineResult<()> {
    let cutter = Cutter::new(cx, c, lines, b, ignore);
    // A bottom is cut when the cut goes through, or when it was picked to be.
    let kept = |l: &Line| !skip.contains(&l.id) && (through || !l.bottom || bottoms.contains(&l.id));
    let rings = cutter.walls.len();
    let mut loops: Vec<Vec<DVec2>> = Vec::new();
    let mut chains: Vec<Vec<DVec2>> = Vec::new();
    for r in 0..rings {
        let ring: Vec<&Line> = lines.iter().filter(|l| l.ring == r).collect();
        if ring.is_empty() {
            continue;
        }
        if ring.iter().all(|l| kept(l)) && ring.iter().all(|l| l.closed) {
            loops.push(cutter.walls.get(r).cloned().unwrap_or_default());
            continue;
        }
        // Start after a line left out, so the pieces kept join up round the loop.
        let n = ring.len();
        let first_out = ring.iter().position(|l| !kept(l)).unwrap_or(n - 1);
        let mut current: Vec<DVec2> = Vec::new();
        for k in 1..=n {
            let Some(l) = ring.get((first_out + k) % n) else { continue };
            if kept(l) {
                current.extend(l.points.iter().skip(usize::from(!current.is_empty())).copied());
            } else if !current.is_empty() {
                chains.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            chains.push(current);
        }
    }
    if loops.is_empty() && chains.is_empty() {
        return Err(EngineError::InvalidParams("every line of the outline is left out".into()));
    }
    // Each chain left to right, the chains in the order the wire meets them.
    for ch in &mut chains {
        if ch.first().map(|q| q.x) > ch.last().map(|q| q.x) {
            ch.reverse();
        }
    }
    chains.sort_by(|a, b| min_x(a).total_cmp(&min_x(b)));
    if from_right {
        chains.reverse();
        for ch in &mut chains {
            ch.reverse();
        }
    }
    let mut runs: Vec<Run> =
        chains.into_iter().map(|points| Run { points, closed: false, lead_in: None, lead_out: None }).collect();
    if bridge {
        runs = cutter.bridged(runs);
    }
    // Whole loops last, each from its point nearest the lead side.
    loops.sort_by(|a, b| min_x(a).total_cmp(&min_x(b)));
    if from_right {
        loops.reverse();
    }
    for mut l in loops {
        if l.len() > 1 && l.first() == l.last() {
            l.pop();
        }
        let entry = crate::cut::argmax(&l, |q| if from_right { q.x } else { -q.x });
        let n = l.len();
        let mut points: Vec<DVec2> = (0..=n).filter_map(|k| l.get((entry + k) % n).copied()).collect();
        if points.len() < 3 {
            points = l;
        }
        runs.push(Run { points, closed: true, lead_in: None, lead_out: None });
    }
    cutter.cut(runs, from_right, out);
    Ok(())
}

/// Cuts the chosen chains that lie in this pass, in the order chosen, each the way round it is
/// set, with leads of `lead_in` and `lead_out` along its ends (none where an end is on the floor).
#[allow(clippy::too_many_arguments)]
pub(crate) fn chain_pass(
    cx: &Context<'_>,
    c: &Common,
    lines: &[Line],
    chains: &[Chain],
    bridge: bool,
    leads: (f64, f64),
    b: f64,
    ignore: &[String],
    out: &mut Built,
) {
    let cutter = Cutter::new(cx, c, lines, b, ignore);
    let mut runs: Vec<Run> = Vec::new();
    for chain in chains {
        let pieces: Vec<&[DVec2]> = chain
            .lines
            .iter()
            .filter_map(|id| lines.iter().find(|l| l.id == *id))
            .map(|l| l.points.as_slice())
            .collect();
        if pieces.is_empty() {
            continue;
        }
        for mut points in join(&pieces) {
            if chain.reverse {
                points.reverse();
            }
            let n = points.len();
            let closed = n > 3 && points.first().zip(points.last()).is_some_and(|(a, z)| a.distance(*z) < 1.0e-4);
            let (Some(&a), Some(&a1)) = (points.first(), points.get(1)) else { continue };
            let (Some(&z), Some(&z1)) = (points.last(), n.checked_sub(2).and_then(|k| points.get(k))) else {
                continue;
            };
            let lead = |at: DVec2, along: DVec2, len: f64| -> Option<DVec2> {
                let q = at + along.normalize_or_zero() * len;
                (len > 1.0e-9 && (closed || !cutter.at_floor(at)) && q.y >= cutter.floor - 1.0e-9).then_some(q)
            };
            runs.push(Run { lead_in: lead(a, a - a1, leads.0), lead_out: lead(z, z - z1, leads.1), points, closed });
        }
    }
    if bridge {
        runs = cutter.bridged(runs);
    }
    cutter.cut(runs, false, out);
}
