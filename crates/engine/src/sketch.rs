//! Sketches: 2D drawings on one of the three main planes, held to their constraints and
//! dimensions by a solver.
//!
//! A sketch is points, the curves between them (lines, circles, arcs) and constraints on them.
//! Curves that meet share their end point, so a closed outline is a loop of shared points.
//! Constraints are geometric (horizontal, parallel, tangent…) or dimensions (a length, a
//! radius, an angle) with a value. After every change the solver moves the points as little as
//! it can so every constraint holds; a change it cannot satisfy is refused.
//!
//! New geometry snaps to what is there ([`Sketch::infer`]): an end point is shared, a midpoint
//! or a point on a curve becomes a constraint, and a nearly level or upright line is made
//! horizontal or vertical. The same rules serve the mouse and the console.
//!
//! The sketch's closed (and open) outlines make the shape it belongs to, so cuts use it.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

use CosmicCut_io::profile::Path2;
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};

/// Most points a sketch holds (an imported outline is a line between each pair of its points).
pub const MAX_POINTS: usize = 5000;
/// Most curves and constraints.
pub const MAX_ITEMS: usize = 15_000;
/// How far a constraint may be off and still count as met, mm (or radians × 100).
const TOLERANCE: f64 = 1.0e-6;

/// The plane a sketch is drawn on, through the table frame's origin (the turntable's axis on the
/// table top).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    /// X across, Z up: the plane the wire cuts in at B = 0.
    Front,
    /// Y across, Z up.
    Right,
    /// X across, Y up: the table top.
    Top,
}

impl Plane {
    pub const ALL: [Self; 3] = [Self::Front, Self::Right, Self::Top];
    pub const IDS: &'static [&'static str] = &["front", "right", "top"];

    pub fn id(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Right => "right",
            Self::Top => "top",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Front => "Front (XZ)",
            Self::Right => "Right (YZ)",
            Self::Top => "Top (XY)",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// A sketch point in the table frame.
    pub fn to_3d(self, p: DVec2) -> DVec3 {
        match self {
            Self::Front => DVec3::new(p.x, 0.0, p.y),
            Self::Right => DVec3::new(0.0, p.x, p.y),
            Self::Top => DVec3::new(p.x, p.y, 0.0),
        }
    }

    /// A table-frame point in the sketch's coordinates (dropped onto the plane).
    pub fn to_2d(self, p: DVec3) -> DVec2 {
        match self {
            Self::Front => DVec2::new(p.x, p.z),
            Self::Right => DVec2::new(p.y, p.z),
            Self::Top => DVec2::new(p.x, p.y),
        }
    }

    /// The plane's normal, towards the viewer of its view.
    pub fn normal(self) -> DVec3 {
        match self {
            Self::Front => DVec3::NEG_Y,
            Self::Right => DVec3::X,
            Self::Top => DVec3::Z,
        }
    }

    /// The view that looks straight at it.
    pub fn view(self) -> &'static str {
        self.id()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SPoint {
    pub id: u32,
    pub at: DVec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geom {
    Line {
        a: u32,
        b: u32,
    },
    Circle {
        c: u32,
        r: f64,
    },
    /// Counter-clockwise from `a` to `b` about `c`.
    Arc {
        c: u32,
        a: u32,
        b: u32,
    },
    /// A lone point (its own entity so it can be selected and deleted).
    Point {
        p: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entity {
    pub id: u32,
    pub geom: Geom,
    /// Construction geometry guides the drawing but is not part of the outline.
    pub construction: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    // Geometric constraints.
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Equal,
    Midpoint,
    OnCurve,
    Tangent,
    Concentric,
    Fix,
    // Dimensions, with a value.
    Distance,
    HDistance,
    VDistance,
    Radius,
    Diameter,
    Angle,
}

impl Kind {
    pub const GEOMETRIC: &'static [&'static str] = &[
        "horizontal",
        "vertical",
        "parallel",
        "perpendicular",
        "equal",
        "coincident",
        "midpoint",
        "tangent",
        "concentric",
        "fix",
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Parallel => "parallel",
            Self::Perpendicular => "perpendicular",
            Self::Equal => "equal",
            Self::Midpoint => "midpoint",
            Self::OnCurve => "on",
            Self::Tangent => "tangent",
            Self::Concentric => "concentric",
            Self::Fix => "fix",
            Self::Distance => "distance",
            Self::HDistance => "horizontal_distance",
            Self::VDistance => "vertical_distance",
            Self::Radius => "radius",
            Self::Diameter => "diameter",
            Self::Angle => "angle",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        [
            Self::Horizontal,
            Self::Vertical,
            Self::Parallel,
            Self::Perpendicular,
            Self::Equal,
            Self::Midpoint,
            Self::OnCurve,
            Self::Tangent,
            Self::Concentric,
            Self::Fix,
            Self::Distance,
            Self::HDistance,
            Self::VDistance,
            Self::Radius,
            Self::Diameter,
            Self::Angle,
        ]
        .into_iter()
        .find(|k| k.id() == id)
    }

    /// Whether constraint `c` is two points lined up (level or upright): kept as a zero distance
    /// across or up, but a constraint to the user, not a dimension.
    pub fn is_alignment(c: &Constraint) -> bool {
        matches!(c.kind, Self::HDistance | Self::VDistance) && c.value2 > 0.5
    }

    pub fn is_dimension(self) -> bool {
        matches!(self, Self::Distance | Self::HDistance | Self::VDistance | Self::Radius | Self::Diameter | Self::Angle)
    }

    /// A short mark drawn beside what it constrains.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Horizontal => "H",
            Self::Vertical => "V",
            Self::Parallel => "∥",
            Self::Perpendicular => "⊥",
            Self::Equal => "=",
            Self::Midpoint => "M",
            Self::OnCurve => "◦",
            Self::Tangent => "T",
            Self::Concentric => "◎",
            Self::Fix => "⚓",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub id: u32,
    pub kind: Kind,
    /// Point ids for Fix, Midpoint (point, line), OnCurve (point, curve), point distances;
    /// entity ids for the rest.
    pub refs: Vec<u32>,
    /// A dimension's value (mm, or degrees for an angle); Fix's x.
    pub value: f64,
    /// Fix's y; for a tangent arc pair, 1 when inside (radii subtract).
    pub value2: f64,
}

/// What a point snaps to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Snap {
    /// Nothing: where it was put.
    Free,
    /// An existing point, which is shared.
    Point(u32),
    /// The middle of a line.
    Midpoint(u32),
    /// Somewhere on a line, circle or arc.
    On(u32),
    /// Level with a point (same height: the first), and/or straight above or below one (the
    /// second). The origin's are the plane's axes.
    Align(Option<u32>, Option<u32>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sketch {
    pub plane: Plane,
    pub points: Vec<SPoint>,
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
    /// Where dimensions' sizes were dragged to (sketch coordinates), by dimension id; the others
    /// sit where they are put by default.
    pub labels: Vec<(u32, DVec2)>,
    next: u32,
}

/// The id of the origin point, held at (0, 0).
pub const ORIGIN: u32 = 1;

/// An item id with this bit set stands for the midpoint of the line with the rest of the id
/// (`m12` in commands). Constraining or dimensioning it gives the solver a point held there.
pub const MIDPOINT: u32 = 1 << 31;

impl Sketch {
    /// An empty sketch on `plane`, with its origin point fixed at (0, 0).
    pub fn new(plane: Plane) -> Self {
        let mut s = Self {
            plane,
            points: Vec::new(),
            entities: Vec::new(),
            constraints: Vec::new(),
            labels: Vec::new(),
            next: ORIGIN,
        };
        let o = s.add_point(DVec2::ZERO);
        let id = s.fresh();
        s.constraints.push(Constraint { id, kind: Kind::Fix, refs: vec![o], value: 0.0, value2: 0.0 });
        s
    }

    fn fresh(&mut self) -> u32 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }

    pub fn point(&self, id: u32) -> Option<DVec2> {
        self.points.iter().find(|p| p.id == id).map(|p| p.at)
    }

    pub fn entity(&self, id: u32) -> Option<&Entity> {
        self.entities.iter().find(|e| e.id == id)
    }

    pub fn constraint(&self, id: u32) -> Option<&Constraint> {
        self.constraints.iter().find(|c| c.id == id)
    }

    fn add_point(&mut self, at: DVec2) -> u32 {
        let id = self.fresh();
        self.points.push(SPoint { id, at });
        id
    }

    fn check_room(&self, points: usize, items: usize) -> EngineResult<()> {
        if self.points.len() + points > MAX_POINTS || self.entities.len() + self.constraints.len() + items > MAX_ITEMS {
            return Err(EngineError::InvalidParams(format!(
                "a sketch holds at most {MAX_POINTS} points and {MAX_ITEMS} curves and constraints"
            )));
        }
        Ok(())
    }

    /// A circle's radius or an arc's (from its start point).
    pub fn radius(&self, e: &Entity) -> Option<f64> {
        match e.geom {
            Geom::Circle { r, .. } => Some(r),
            Geom::Arc { c, a, .. } => Some(self.point(c)?.distance(self.point(a)?)),
            _ => None,
        }
    }

    /// A line's end points.
    pub fn line(&self, id: u32) -> Option<(DVec2, DVec2)> {
        match self.entity(id)?.geom {
            Geom::Line { a, b } => Some((self.point(a)?, self.point(b)?)),
            _ => None,
        }
    }

    // ---- Snapping -----------------------------------------------------------------------

    /// What `at` snaps to within `tol` (mm), points first, then midpoints, then curves; and
    /// where that puts it. `skip` points are not snapped to (the point being moved).
    pub fn infer(&self, at: DVec2, tol: f64, skip: &[u32]) -> (Snap, DVec2) {
        let tol = if tol.is_finite() && tol > 0.0 { tol } else { 0.0 };
        let mut best: Option<(f64, u32, DVec2)> = None;
        for p in self.points.iter().filter(|p| !skip.contains(&p.id)) {
            let d = p.at.distance(at);
            if d <= tol && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, p.id, p.at));
            }
        }
        if let Some((_, id, p)) = best {
            return (Snap::Point(id), p);
        }
        for e in &self.entities {
            if let Geom::Line { a, b } = e.geom
                && !skip.contains(&a)
                && !skip.contains(&b)
                && let (Some(pa), Some(pb)) = (self.point(a), self.point(b))
            {
                let mid = (pa + pb) * 0.5;
                if mid.distance(at) <= tol {
                    return (Snap::Midpoint(e.id), mid);
                }
            }
        }
        let mut best: Option<(f64, u32, DVec2)> = None;
        for e in &self.entities {
            let touches = |id: u32| skip.contains(&id);
            let near = match e.geom {
                Geom::Line { a, b } if !touches(a) && !touches(b) => {
                    let (Some(pa), Some(pb)) = (self.point(a), self.point(b)) else { continue };
                    closest_on_segment(pa, pb, at)
                }
                Geom::Circle { c, r } if !touches(c) => {
                    let Some(pc) = self.point(c) else { continue };
                    let d = at - pc;
                    if d.length() < 1.0e-12 {
                        continue;
                    }
                    pc + d.normalize() * r
                }
                Geom::Arc { c, a, b } if !touches(c) && !touches(a) && !touches(b) => {
                    let (Some(pc), Some(pa), Some(pb)) = (self.point(c), self.point(a), self.point(b)) else {
                        continue;
                    };
                    let d = at - pc;
                    if d.length() < 1.0e-12 || !in_arc(pc, pa, pb, at) {
                        continue;
                    }
                    pc + d.normalize() * pc.distance(pa)
                }
                _ => continue,
            };
            let d = near.distance(at);
            if d <= tol && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, e.id, near));
            }
        }
        if let Some((_, id, p)) = best {
            return (Snap::On(id), p);
        }
        // Lined up with a point: level with it, or straight above or below it.
        let mut level: Option<(f64, u32, f64)> = None;
        let mut upright: Option<(f64, u32, f64)> = None;
        for p in self.points.iter().filter(|p| !skip.contains(&p.id)) {
            let (dy, dx) = ((p.at.y - at.y).abs(), (p.at.x - at.x).abs());
            if dy <= tol && level.is_none_or(|(d, _, _)| dy < d) {
                level = Some((dy, p.id, p.at.y));
            }
            if dx <= tol && upright.is_none_or(|(d, _, _)| dx < d) {
                upright = Some((dx, p.id, p.at.x));
            }
        }
        match (level, upright) {
            (None, None) => (Snap::Free, at),
            (l, u) => (
                Snap::Align(l.map(|(_, id, _)| id), u.map(|(_, id, _)| id)),
                DVec2::new(u.map_or(at.x, |(_, _, x)| x), l.map_or(at.y, |(_, _, y)| y)),
            ),
        }
    }

    /// The point for a snapped position: an existing one, or a new one with the constraint the
    /// snap asks for.
    fn place(&mut self, snap: Snap, at: DVec2) -> u32 {
        match snap {
            Snap::Point(id) if self.point(id).is_some() => id,
            Snap::Midpoint(line) => {
                let p = self.add_point(at);
                let id = self.fresh();
                self.constraints.push(Constraint {
                    id,
                    kind: Kind::Midpoint,
                    refs: vec![p, line],
                    value: 0.0,
                    value2: 0.0,
                });
                p
            }
            Snap::Align(level, upright) => {
                let p = self.add_point(at);
                // Held lined up: no distance up (level), or across (upright), drawn as H or V.
                for (with, kind) in [(level, Kind::VDistance), (upright, Kind::HDistance)] {
                    if let Some(q) = with.filter(|q| self.point(*q).is_some()) {
                        let id = self.fresh();
                        self.constraints.push(Constraint { id, kind, refs: vec![q, p], value: 0.0, value2: 1.0 });
                    }
                }
                p
            }
            Snap::On(curve) => {
                let p = self.add_point(at);
                let id = self.fresh();
                self.constraints.push(Constraint {
                    id,
                    kind: Kind::OnCurve,
                    refs: vec![p, curve],
                    value: 0.0,
                    value2: 0.0,
                });
                p
            }
            _ => self.add_point(at),
        }
    }

    /// Whether a line from `a` to `b` is close enough to level, or upright, to be made so:
    /// within `tol` (mm) of it over its length, and within 3°.
    pub fn level_or_upright(a: DVec2, b: DVec2, tol: f64) -> Option<Kind> {
        let d = b - a;
        let len = d.length();
        if len < 1.0e-9 {
            return None;
        }
        let near = |off: f64| off.abs() <= tol.max(0.0) || (off.abs() / len) < (3.0f64).to_radians().sin();
        if near(d.y) {
            Some(Kind::Horizontal)
        } else if near(d.x) {
            Some(Kind::Vertical)
        } else {
            None
        }
    }

    // ---- Adding geometry ----------------------------------------------------------------

    /// A line from `a` to `b`, each snapped within `tol`; made horizontal or vertical when it is
    /// nearly so. Returns the line's id.
    pub fn add_line(&mut self, a: DVec2, b: DVec2, tol: f64) -> EngineResult<u32> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<u32> {
            self.check_room(2, 4)?;
            let (sa, pa) = self.infer(a, tol, &[]);
            // The end does not line up with the start: a level or upright line says that itself.
            let start: Vec<u32> = match sa {
                Snap::Point(id) => vec![id],
                _ => Vec::new(),
            };
            let (sb, pb) = self.infer(b, tol, &start);
            if pa.distance(pb) < 1.0e-6 {
                return Err(EngineError::InvalidParams("the line has no length".into()));
            }
            let ia = self.place(sa, pa);
            if sb == Snap::Point(ia) {
                return Err(EngineError::InvalidParams("the line has no length".into()));
            }
            let ib = self.place(sb, pb);
            let line = self.fresh();
            self.entities.push(Entity { id: line, geom: Geom::Line { a: ia, b: ib }, construction: false });
            if let Some(kind) = Self::level_or_upright(pa, pb, tol) {
                let id = self.fresh();
                self.constraints.push(Constraint { id, kind, refs: vec![line], value: 0.0, value2: 0.0 });
            }
            self.solve()?;
            Ok(line)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// A rectangle from corner `a` to the opposite corner `b` (or centred on `a` with `b` a corner
    /// when `centred`): four lines, two horizontal and two vertical. Returns the lines.
    pub fn add_rectangle(&mut self, a: DVec2, b: DVec2, centred: bool, tol: f64) -> EngineResult<Vec<u32>> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<Vec<u32>> {
            self.check_room(5, 12)?;
            let (lo, hi) = if centred { (a - (b - a).abs(), a + (b - a).abs()) } else { (a.min(b), a.max(b)) };
            if (hi.x - lo.x) < 1.0e-6 || (hi.y - lo.y) < 1.0e-6 {
                return Err(EngineError::InvalidParams("the rectangle has no area".into()));
            }
            let corners = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
            let ids: Vec<u32> = corners
                .iter()
                .map(|&c| {
                    let (snap, at) = self.infer(c, tol, &[]);
                    // Only the corners the user put down snap to points; the others stay where they are.
                    let picked = c == a || c == b;
                    if picked { self.place(snap, at) } else { self.add_point(c) }
                })
                .collect();
            let mut lines = Vec::new();
            for k in 0..4 {
                let (Some(&p), Some(&q)) = (ids.get(k), ids.get((k + 1) % 4)) else { continue };
                let line = self.fresh();
                self.entities.push(Entity { id: line, geom: Geom::Line { a: p, b: q }, construction: false });
                let kind = if k % 2 == 0 { Kind::Horizontal } else { Kind::Vertical };
                let id = self.fresh();
                self.constraints.push(Constraint { id, kind, refs: vec![line], value: 0.0, value2: 0.0 });
                lines.push(line);
            }
            if centred {
                // The centre stays the middle: a construction diagonal with the centre at its midpoint.
                let (snap, at) = self.infer(a, tol, &[]);
                let centre = self.place(snap, at);
                if let (Some(&p), Some(&q)) = (ids.first(), ids.get(2)) {
                    let diag = self.fresh();
                    self.entities.push(Entity { id: diag, geom: Geom::Line { a: p, b: q }, construction: true });
                    let id = self.fresh();
                    self.constraints.push(Constraint {
                        id,
                        kind: Kind::Midpoint,
                        refs: vec![centre, diag],
                        value: 0.0,
                        value2: 0.0,
                    });
                }
            }
            self.solve()?;
            Ok(lines)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// A circle about `centre` through `edge`.
    pub fn add_circle(&mut self, centre: DVec2, radius: f64, tol: f64) -> EngineResult<u32> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<u32> {
            self.check_room(1, 2)?;
            if !(radius.is_finite() && radius > 1.0e-6) {
                return Err(EngineError::InvalidParams("the circle needs a radius".into()));
            }
            let (snap, at) = self.infer(centre, tol, &[]);
            let c = self.place(snap, at);
            let id = self.fresh();
            self.entities.push(Entity { id, geom: Geom::Circle { c, r: radius }, construction: false });
            self.solve()?;
            Ok(id)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// An arc from `start` through `through` to `end`.
    pub fn add_arc(&mut self, start: DVec2, through: DVec2, end: DVec2, tol: f64) -> EngineResult<u32> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<u32> {
            self.check_room(3, 3)?;
            let (s1, p1) = self.infer(start, tol, &[]);
            let (s2, p2) = self.infer(end, tol, &[]);
            let centre = circumcentre(p1, through, p2)
                .ok_or_else(|| EngineError::InvalidParams("the arc's three points are in a line".into()))?;
            // Counter-clockwise from start to end when the middle point is to the right of the chord
            // from start to end; otherwise it goes the other way round, from end to start.
            let ccw = (p2 - p1).perp_dot(through - p1) < 0.0;
            let (sa, pa, sb, pb) = if ccw { (s1, p1, s2, p2) } else { (s2, p2, s1, p1) };
            let a = self.place(sa, pa);
            let b = self.place(sb, pb);
            if a == b {
                return Err(EngineError::InvalidParams("the arc starts where it ends".into()));
            }
            let c = self.add_point(centre);
            let id = self.fresh();
            self.entities.push(Entity { id, geom: Geom::Arc { c, a, b }, construction: false });
            self.solve()?;
            Ok(id)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// A regular polygon of `sides` about `centre` with a corner at `corner`: equal sides on a
    /// construction circle.
    pub fn add_polygon(&mut self, centre: DVec2, corner: DVec2, sides: usize, tol: f64) -> EngineResult<Vec<u32>> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<Vec<u32>> {
            if !(3..=64).contains(&sides) {
                return Err(EngineError::InvalidParams("a polygon has 3 to 64 sides".into()));
            }
            self.check_room(sides + 1, 3 * sides + 2)?;
            let r = corner.distance(centre);
            if r < 1.0e-6 {
                return Err(EngineError::InvalidParams("the polygon has no size".into()));
            }
            let (snap, at) = self.infer(centre, tol, &[]);
            let c = self.place(snap, at);
            let ring = self.fresh();
            self.entities.push(Entity { id: ring, geom: Geom::Circle { c, r }, construction: true });
            let start = (corner - centre).to_angle();
            let ids: Vec<u32> = (0..sides)
                .map(|k| {
                    let t = start + TAU * k as f64 / sides as f64;
                    let p = self.add_point(at + DVec2::from_angle(t) * r);
                    let id = self.fresh();
                    self.constraints.push(Constraint {
                        id,
                        kind: Kind::OnCurve,
                        refs: vec![p, ring],
                        value: 0.0,
                        value2: 0.0,
                    });
                    p
                })
                .collect();
            let mut lines = Vec::new();
            for k in 0..sides {
                let (Some(&p), Some(&q)) = (ids.get(k), ids.get((k + 1) % sides)) else { continue };
                let line = self.fresh();
                self.entities.push(Entity { id: line, geom: Geom::Line { a: p, b: q }, construction: false });
                if let Some(&first) = lines.first() {
                    let id = self.fresh();
                    self.constraints.push(Constraint {
                        id,
                        kind: Kind::Equal,
                        refs: vec![first, line],
                        value: 0.0,
                        value2: 0.0,
                    });
                }
                lines.push(line);
            }
            self.solve()?;
            Ok(lines)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// A lone point.
    pub fn add_point_entity(&mut self, at: DVec2, tol: f64) -> EngineResult<u32> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<u32> {
            self.check_room(1, 2)?;
            let (snap, at) = self.infer(at, tol, &[]);
            let p = self.place(snap, at);
            let id = self.fresh();
            self.entities.push(Entity { id, geom: Geom::Point { p }, construction: false });
            self.solve()?;
            Ok(id)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    // ---- Constraints and dimensions ---------------------------------------------------------

    /// Points of the items: a point entity's point, a line's ends; ids that are points as they are.
    fn as_points(&self, items: &[u32]) -> Vec<u32> {
        let mut out = Vec::new();
        for &i in items {
            if self.point(i).is_some() {
                out.push(i);
            } else if let Some(e) = self.entity(i) {
                match e.geom {
                    Geom::Point { p } => out.push(p),
                    Geom::Line { a, b } => {
                        out.push(a);
                        out.push(b);
                    }
                    Geom::Circle { c, .. } => out.push(c),
                    Geom::Arc { c, a, b } => {
                        out.push(c);
                        out.push(a);
                        out.push(b);
                    }
                }
            }
        }
        out
    }

    fn kind_of(&self, id: u32) -> Option<&'static str> {
        if self.point(id).is_some() {
            return Some("point");
        }
        Some(match self.entity(id)?.geom {
            Geom::Line { .. } => "line",
            Geom::Circle { .. } | Geom::Arc { .. } => "curve",
            Geom::Point { .. } => "point",
        })
    }

    /// The point a point-like item stands for.
    fn point_of(&self, id: u32) -> Option<u32> {
        if self.point(id).is_some() {
            return Some(id);
        }
        match self.entity(id)?.geom {
            Geom::Point { p } => Some(p),
            _ => None,
        }
    }

    /// Adds geometric constraint `kind` (one of [`Kind::GEOMETRIC`]) to `items`. Returns the
    /// constraints added (none for coincident, which merges the points).
    pub fn constrain(&mut self, kind: &str, items: &[u32]) -> EngineResult<Vec<u32>> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<Vec<u32>> {
            self.check_room(0, items.len().max(1))?;
            let resolved = self.resolve(items)?;
            let items: &[u32] = &resolved;
            let bad = |what: &str| Err(EngineError::InvalidParams(format!("{kind} needs {what}")));
            let kinds: Vec<&str> = items.iter().filter_map(|&i| self.kind_of(i)).collect();
            if kinds.len() != items.len() {
                return Err(EngineError::InvalidParams("an item is not in the sketch".into()));
            }
            let mut added = Vec::new();
            let mut push = |s: &mut Self, kind: Kind, refs: Vec<u32>, value: f64, value2: f64| {
                let id = s.fresh();
                s.constraints.push(Constraint { id, kind, refs, value, value2 });
                added.push(id);
            };
            match kind {
                "horizontal" | "vertical" => {
                    let k = if kind == "horizontal" { Kind::Horizontal } else { Kind::Vertical };
                    let lines: Vec<u32> = items.iter().copied().filter(|&i| self.kind_of(i) == Some("line")).collect();
                    if !lines.is_empty() {
                        for l in lines {
                            push(self, k, vec![l], 0.0, 0.0);
                        }
                    } else {
                        // Two points level (or upright): a zero distance across them.
                        let pts = self.as_points(items);
                        let (Some(&p), Some(&q)) = (pts.first(), pts.get(1)) else {
                            return bad("lines, or two points");
                        };
                        let dim = if kind == "horizontal" { Kind::VDistance } else { Kind::HDistance };
                        // value2 marks it an alignment (drawn as a constraint, not a dimension).
                        push(self, dim, vec![p, q], 0.0, 1.0);
                    }
                }
                "parallel" | "perpendicular" | "equal" => {
                    let (Some(&a), Some(&b)) = (items.first(), items.get(1)) else { return bad("two items") };
                    let lines = kinds.iter().all(|k| *k == "line");
                    let curves = kinds.iter().all(|k| *k == "curve");
                    if !(lines || (kind == "equal" && curves)) {
                        return bad(if kind == "equal" { "two lines, or two circles or arcs" } else { "two lines" });
                    }
                    let k = match kind {
                        "parallel" => Kind::Parallel,
                        "perpendicular" => Kind::Perpendicular,
                        _ => Kind::Equal,
                    };
                    let _ = b;
                    for &other in items.iter().skip(1) {
                        push(self, k, vec![a, other], 0.0, 0.0);
                    }
                }
                "coincident" => {
                    let pts: Vec<u32> = items.iter().filter_map(|&i| self.point_of(i)).collect();
                    match (pts.as_slice(), items) {
                        ([p, q, ..], _) => self.merge(*p, *q)?,
                        ([p], [_, _]) => {
                            // A point and a curve: the point on it.
                            let curve = items.iter().copied().find(|&i| self.point_of(i).is_none()).unwrap_or_default();
                            push(self, Kind::OnCurve, vec![*p, curve], 0.0, 0.0);
                        }
                        _ => return bad("two points, or a point and a curve"),
                    }
                }
                "midpoint" => {
                    let p = items.iter().find_map(|&i| self.point_of(i));
                    let l = items.iter().copied().find(|&i| self.kind_of(i) == Some("line"));
                    let (Some(p), Some(l)) = (p, l) else { return bad("a point and a line") };
                    push(self, Kind::Midpoint, vec![p, l], 0.0, 0.0);
                }
                "tangent" => {
                    let (Some(&a), Some(&b)) = (items.first(), items.get(1)) else { return bad("two items") };
                    let (line, curve) = match (self.kind_of(a), self.kind_of(b)) {
                        (Some("line"), Some("curve")) => (Some(a), b),
                        (Some("curve"), Some("line")) => (Some(b), a),
                        (Some("curve"), Some("curve")) => (None, b),
                        _ => return bad("a line and a curve, or two curves"),
                    };
                    match line {
                        Some(l) => push(self, Kind::Tangent, vec![l, curve], 0.0, 0.0),
                        None => {
                            // Inside or outside, whichever they are nearer now.
                            let (Some(ea), Some(eb)) = (self.entity(a).copied(), self.entity(b).copied()) else {
                                return bad("two curves");
                            };
                            let (Some(ra), Some(rb)) = (self.radius(&ea), self.radius(&eb)) else {
                                return bad("two curves");
                            };
                            let d = self.centre(&ea).zip(self.centre(&eb)).map_or(0.0, |(p, q)| p.distance(q));
                            let inside = ((d - (ra - rb).abs()).abs() < (d - (ra + rb)).abs()) as u8 as f64;
                            push(self, Kind::Tangent, vec![a, b], 0.0, inside);
                        }
                    }
                }
                "concentric" => {
                    let centres: Vec<u32> = items
                        .iter()
                        .filter_map(|&i| self.entity(i).and_then(|e| self.centre_id(e)).or(self.point_of(i)))
                        .collect();
                    let (Some(&p), Some(&q)) = (centres.first(), centres.get(1)) else {
                        return bad("two circles or arcs");
                    };
                    self.merge(p, q)?;
                }
                "fix" => {
                    let pts = self.as_points(items);
                    if pts.is_empty() {
                        return bad("points or curves");
                    }
                    for p in pts {
                        let at = self.point(p).unwrap_or_default();
                        push(self, Kind::Fix, vec![p], at.x, at.y);
                    }
                }
                _ => return Err(EngineError::InvalidParams(format!("unknown constraint `{kind}`"))),
            }
            self.solve()?;
            Ok(added)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    fn centre_id(&self, e: &Entity) -> Option<u32> {
        match e.geom {
            Geom::Circle { c, .. } | Geom::Arc { c, .. } => Some(c),
            _ => None,
        }
    }

    fn centre(&self, e: &Entity) -> Option<DVec2> {
        self.point(self.centre_id(e)?)
    }

    /// Makes `q` the same point as `p`: everything on `q` moves to `p`.
    fn merge(&mut self, p: u32, q: u32) -> EngineResult<()> {
        if p == q {
            return Ok(());
        }
        let (keep, gone) = if q == ORIGIN { (q, p) } else { (p, q) };
        let swap = |id: &mut u32| {
            if *id == gone {
                *id = keep;
            }
        };
        for e in &mut self.entities {
            match &mut e.geom {
                Geom::Line { a, b } | Geom::Arc { a, b, .. } => {
                    swap(a);
                    swap(b);
                }
                _ => {}
            }
            match &mut e.geom {
                Geom::Circle { c, .. } | Geom::Arc { c, .. } => swap(c),
                Geom::Point { p } => swap(p),
                _ => {}
            }
        }
        for c in &mut self.constraints {
            if !matches!(c.kind, Kind::Parallel | Kind::Perpendicular | Kind::Equal | Kind::Tangent) {
                if matches!(c.kind, Kind::Midpoint | Kind::OnCurve) {
                    if let Some(first) = c.refs.first_mut() {
                        swap(first);
                    }
                } else if matches!(c.kind, Kind::Fix | Kind::Distance | Kind::HDistance | Kind::VDistance) {
                    c.refs.iter_mut().for_each(swap);
                }
            }
        }
        self.points.retain(|pt| pt.id != gone);
        let degenerate = self.entities.iter().any(|e| matches!(e.geom, Geom::Line { a, b } if a == b));
        if degenerate {
            return Err(EngineError::InvalidParams("that would shrink a line to nothing".into()));
        }
        // A distance between a point and itself means nothing now.
        self.constraints.retain(|c| !(c.kind.is_dimension() && c.refs.len() == 2 && c.refs.first() == c.refs.get(1)));
        Ok(())
    }

    /// The dimension that `items` call for, with its current value: a line's length, a circle's
    /// diameter, an arc's radius, the distance between two points (or across, or up, with
    /// `along`), the angle between two lines, a point's distance from a line.
    pub fn measure(&self, items: &[u32], along: Option<&str>) -> EngineResult<(Kind, Vec<u32>, f64)> {
        let kinds: Vec<&str> = items.iter().filter_map(|&i| self.kind_of(i)).collect();
        let dist = |p: u32, q: u32| -> EngineResult<(Kind, Vec<u32>, f64)> {
            let (Some(a), Some(b)) = (self.point(p), self.point(q)) else {
                return Err(EngineError::InvalidParams("a point is not in the sketch".into()));
            };
            Ok(match along {
                Some("horizontal") => (Kind::HDistance, vec![p, q], (b.x - a.x).abs()),
                Some("vertical") => (Kind::VDistance, vec![p, q], (b.y - a.y).abs()),
                _ => (Kind::Distance, vec![p, q], a.distance(b)),
            })
        };
        match (items, kinds.as_slice()) {
            ([l], ["line"]) => {
                let pts = self.as_points(&[*l]);
                let (Some(&a), Some(&b)) = (pts.first(), pts.get(1)) else {
                    return Err(EngineError::InvalidParams("the line has no ends".into()));
                };
                dist(a, b)
            }
            ([c], ["curve"]) => {
                let e = self.entity(*c).ok_or_else(|| EngineError::InvalidParams("no such curve".into()))?;
                let r = self.radius(e).unwrap_or_default();
                Ok(match e.geom {
                    Geom::Circle { .. } if along != Some("radius") => (Kind::Diameter, vec![*c], 2.0 * r),
                    _ => (Kind::Radius, vec![*c], r),
                })
            }
            ([a, b], ["point", "point"]) => dist(self.point_of(*a).unwrap_or(*a), self.point_of(*b).unwrap_or(*b)),
            ([a, b], ["line", "line"]) => {
                let (Some((a0, a1)), Some((b0, b1))) = (self.line(*a), self.line(*b)) else {
                    return Err(EngineError::InvalidParams("no such line".into()));
                };
                let (da, db) = (a1 - a0, b1 - b0);
                if da.perp_dot(db).abs() < 1.0e-9 * da.length() * db.length() {
                    // Parallel lines: the gap between them, from one end of the second.
                    let pts = self.as_points(&[*b]);
                    let p = pts.first().copied().unwrap_or_default();
                    return Ok((Kind::Distance, vec![p, *a], point_line(b0, a0, a1).abs()));
                }
                let (ra, rb) = angle_rays(a0, a1, b0, b1);
                Ok((Kind::Angle, vec![*a, *b], ra.angle_to(rb).to_degrees()))
            }
            ([p, l], ["point", "line"]) | ([l, p], ["line", "point"]) => {
                let pid = self.point_of(*p).unwrap_or(*p);
                let (Some(at), Some((a0, a1))) = (self.point(pid), self.line(*l)) else {
                    return Err(EngineError::InvalidParams("no such point or line".into()));
                };
                Ok((Kind::Distance, vec![pid, *l], point_line(at, a0, a1).abs()))
            }
            _ => Err(EngineError::InvalidParams(
                "dimension a line, a circle or arc, two points, two lines, or a point and a line".into(),
            )),
        }
    }

    /// Adds the dimension `items` call for, set to `value` (its current value if None).
    /// Returns its id.
    pub fn dimension(&mut self, items: &[u32], along: Option<&str>, value: Option<f64>) -> EngineResult<u32> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<u32> {
            self.check_room(0, 1)?;
            let resolved = self.resolve(items)?;
            let items: &[u32] = &resolved;
            let (kind, refs, now) = self.measure(items, along)?;
            let value = value.unwrap_or(now);
            check_value(kind, value)?;
            // An angle is measured from where the lines meet out along each (which way along each
            // line, kept in value2), so it is the angle between them as drawn, not its outside.
            let value2 = match (kind, refs.first().and_then(|&a| self.line(a)), refs.get(1).and_then(|&b| self.line(b)))
            {
                (Kind::Angle, Some((a0, a1)), Some((b0, b1))) => {
                    let (ra, rb) = angle_rays(a0, a1, b0, b1);
                    f64::from(u8::from(ra.dot(a1 - a0) < 0.0) | (u8::from(rb.dot(b1 - b0) < 0.0) << 1))
                }
                _ => 0.0,
            };
            let id = self.fresh();
            self.constraints.push(Constraint { id, kind, refs, value, value2 });
            self.solve()?;
            Ok(id)
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// Sets dimension `id` to `value`.
    pub fn set_dimension(&mut self, id: u32, value: f64) -> EngineResult<()> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<()> {
            let c = self
                .constraints
                .iter_mut()
                .find(|c| c.id == id && c.kind.is_dimension())
                .ok_or_else(|| EngineError::InvalidParams("there is no such dimension".into()))?;
            check_value(c.kind, value)?;
            // An angle keeps the way round it was measured; the size typed is how far.
            c.value = if c.kind == Kind::Angle && c.value < 0.0 {
                -value.abs()
            } else if c.kind == Kind::Angle {
                value.abs()
            } else {
                value
            };
            self.solve()
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// Where dimension `id`'s size is shown, if it was dragged.
    pub fn label(&self, id: u32) -> Option<DVec2> {
        self.labels.iter().find(|(l, _)| *l == id).map(|(_, at)| *at)
    }

    /// Puts dimension `id`'s size at `at` (sketch coordinates); its lines follow.
    pub fn move_label(&mut self, id: u32, at: DVec2) -> EngineResult<()> {
        if !self.constraint(id).is_some_and(|c| c.kind.is_dimension()) {
            return Err(EngineError::InvalidParams("there is no such dimension".into()));
        }
        self.labels.retain(|(l, _)| *l != id);
        self.labels.push((id, at));
        Ok(())
    }

    /// Moves `items` (all of the sketch when empty) by `delta`, as far as their constraints let
    /// them: dimensions to the origin or fixed points hold what they hold.
    pub fn move_items(&mut self, items: &[u32], delta: DVec2) -> EngineResult<()> {
        let pts: Vec<u32> = if items.is_empty() {
            self.points.iter().map(|p| p.id).filter(|&p| p != ORIGIN).collect()
        } else {
            self.as_points(items).into_iter().filter(|&p| p != ORIGIN).collect()
        };
        if pts.is_empty() || !delta.is_finite() {
            return Err(EngineError::InvalidParams("nothing to move".into()));
        }
        let start = self.clone();
        for p in &mut self.points {
            if pts.contains(&p.id) {
                p.at += delta;
            }
        }
        // Fixed points that moved with it stay fixed where they went; labels go along.
        for c in &mut self.constraints {
            if c.kind == Kind::Fix && c.refs.first().is_some_and(|r| pts.contains(r)) {
                c.value += delta.x;
                c.value2 += delta.y;
            }
        }
        if items.is_empty() {
            for (_, at) in &mut self.labels {
                *at += delta;
            }
        }
        let held: Vec<Constraint> = pts
            .iter()
            .filter_map(|&p| {
                let at = self.point(p)?;
                Some(Constraint { id: 0, kind: Kind::Fix, refs: vec![p], value: at.x, value2: at.y })
            })
            .collect();
        if self.solve_with(&held).is_ok() || self.solve().is_ok() {
            return Ok(());
        }
        *self = start;
        Err(EngineError::InvalidParams("the constraints hold it there".into()))
    }

    /// Removes items: curves (and the points only they used), constraints and dimensions.
    pub fn delete(&mut self, items: &[u32]) -> EngineResult<()> {
        // All or nothing: a change that fails leaves the sketch as it was.
        let before = self.clone();
        let out = (|| -> EngineResult<()> {
            let mut gone_entities: Vec<u32> = Vec::new();
            for &i in items {
                if self.entity(i).is_some() {
                    gone_entities.push(i);
                } else if let Some(c) = self.constraint(i) {
                    if c.kind == Kind::Fix && c.refs.first() == Some(&ORIGIN) {
                        return Err(EngineError::InvalidParams("the origin stays fixed".into()));
                    }
                } else if self.point(i).is_some() {
                    if i == ORIGIN {
                        return Err(EngineError::InvalidParams("the origin cannot be deleted".into()));
                    }
                    // A point goes with every curve on it.
                    for e in &self.entities {
                        let on = match e.geom {
                            Geom::Line { a, b } => a == i || b == i,
                            Geom::Circle { c, .. } => c == i,
                            Geom::Arc { c, a, b } => c == i || a == i || b == i,
                            Geom::Point { p } => p == i,
                        };
                        if on {
                            gone_entities.push(e.id);
                        }
                    }
                }
            }
            self.entities.retain(|e| !gone_entities.contains(&e.id));
            self.constraints.retain(|c| !items.contains(&c.id));
            // Constraints on what is gone go too.
            let entity_ids: Vec<u32> = self.entities.iter().map(|e| e.id).collect();
            let used: Vec<u32> = self
                .entities
                .iter()
                .flat_map(|e| match e.geom {
                    Geom::Line { a, b } => vec![a, b],
                    Geom::Circle { c, .. } => vec![c],
                    Geom::Arc { c, a, b } => vec![c, a, b],
                    Geom::Point { p } => vec![p],
                })
                .chain([ORIGIN])
                .collect();
            self.points.retain(|p| used.contains(&p.id));
            let point_ids: Vec<u32> = self.points.iter().map(|p| p.id).collect();
            self.constraints.retain(|c| c.refs.iter().all(|r| entity_ids.contains(r) || point_ids.contains(r)));
            let dims: Vec<u32> = self.constraints.iter().map(|c| c.id).collect();
            self.labels.retain(|(l, _)| dims.contains(l));
            self.solve()
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// The line or arc that curve `new` carries on from: one sharing an end point with it.
    pub fn previous(&self, new: u32) -> Option<(u32, u32)> {
        let ends = |e: &Entity| match e.geom {
            Geom::Line { a, b } | Geom::Arc { a, b, .. } => Some([a, b]),
            _ => None,
        };
        let mine = ends(self.entity(new)?)?;
        self.entities.iter().filter(|e| e.id != new).find_map(|e| {
            let theirs = ends(e)?;
            mine.iter().find(|p| theirs.contains(p)).map(|&p| (e.id, p))
        })
    }

    /// Makes curve `new` tangent to the one it carries on from.
    pub fn tangent_to_previous(&mut self, new: u32) -> EngineResult<()> {
        let (prev, _) =
            self.previous(new).ok_or_else(|| EngineError::InvalidParams("there is no line to carry on from".into()))?;
        self.constrain("tangent", &[prev, new]).map(|_| ())
    }

    /// Holds line `new` at the angle it makes with the line it carries on from (a chamfer),
    /// measured from the way that line was going to the way the new one goes.
    pub fn chamfer_angle(&mut self, new: u32) -> EngineResult<()> {
        let before = self.clone();
        let out = (|| -> EngineResult<()> {
            let (prev, shared) = self
                .previous(new)
                .ok_or_else(|| EngineError::InvalidParams("there is no line to carry on from".into()))?;
            let (Some(Geom::Line { b: pb, .. }), Some(Geom::Line { a: na, .. })) =
                (self.entity(prev).map(|e| e.geom), self.entity(new).map(|e| e.geom))
            else {
                return Err(EngineError::InvalidParams("a chamfer carries on from a line".into()));
            };
            let ((a0, a1), (b0, b1)) = (
                self.line(prev).ok_or_else(|| EngineError::InvalidParams("no line".into()))?,
                self.line(new).ok_or_else(|| EngineError::InvalidParams("no line".into()))?,
            );
            // Into the corner along the first, out of it along the second.
            let fa = if pb == shared { 1.0 } else { -1.0 };
            let fb = if na == shared { 1.0 } else { -1.0 };
            let value = ((a1 - a0) * fa).angle_to((b1 - b0) * fb).to_degrees();
            let value2 = f64::from(u8::from(fa < 0.0) | (u8::from(fb < 0.0) << 1));
            self.check_room(0, 1)?;
            let id = self.fresh();
            self.constraints.push(Constraint { id, kind: Kind::Angle, refs: vec![prev, new], value, value2 });
            self.solve()
        })();
        if out.is_err() {
            *self = before;
        }
        out
    }

    /// Turns curves into construction geometry, or back.
    pub fn toggle_construction(&mut self, items: &[u32]) -> EngineResult<()> {
        let mut any = false;
        for e in self.entities.iter_mut().filter(|e| items.contains(&e.id)) {
            e.construction = !e.construction;
            any = true;
        }
        if any { Ok(()) } else { Err(EngineError::InvalidParams("select lines, circles or arcs".into())) }
    }

    /// Moves point `id` to `to` (as near as the constraints let it), or a curve by moving its
    /// points by `to` minus `from`.
    pub fn drag(&mut self, id: u32, from: DVec2, to: DVec2) -> EngineResult<()> {
        let pts: Vec<u32> = if self.point(id).is_some() { vec![id] } else { self.as_points(&[id]) };
        if pts.is_empty() {
            return Err(EngineError::InvalidParams("nothing to move".into()));
        }
        let delta = to - from;
        let start = self.clone();
        for p in &mut self.points {
            if pts.contains(&p.id) && p.id != ORIGIN {
                p.at = if pts.len() == 1 { to } else { p.at + delta };
            }
        }
        // Hold what was moved where it was put while the rest follows; if that cannot be, let the
        // moved points give way too.
        let held: Vec<Constraint> = pts
            .iter()
            .filter_map(|&p| {
                let at = self.point(p)?;
                Some(Constraint { id: 0, kind: Kind::Fix, refs: vec![p], value: at.x, value2: at.y })
            })
            .collect();
        if self.solve_with(&held).is_ok() {
            return Ok(());
        }
        let mut free = start.clone();
        for p in &mut free.points {
            if pts.contains(&p.id) && p.id != ORIGIN {
                p.at = if pts.len() == 1 { to } else { p.at + delta };
            }
        }
        if free.solve().is_ok() {
            *self = free;
            return Ok(());
        }
        *self = start;
        Err(EngineError::InvalidParams("the constraints hold it there".into()))
    }

    // ---- Solving --------------------------------------------------------------------------

    /// Moves the points (and radii) so every constraint holds, as little as it can.
    pub fn solve(&mut self) -> EngineResult<()> {
        self.solve_with(&[])
    }

    fn solve_with(&mut self, extra: &[Constraint]) -> EngineResult<()> {
        let mut vars = Vars::of(self);
        let all: Vec<&Constraint> = self.constraints.iter().chain(extra).collect();
        let arcs: Vec<(u32, u32, u32)> = self
            .entities
            .iter()
            .filter_map(|e| match e.geom {
                Geom::Arc { c, a, b } => Some((c, a, b)),
                _ => None,
            })
            .collect();
        let residuals = |v: &Vars| -> Vec<(Vec<usize>, Vec<f64>)> {
            let mut out: Vec<(Vec<usize>, Vec<f64>)> = all.iter().filter_map(|c| residual(self, v, c)).collect();
            for &(c, a, b) in &arcs {
                // An arc's ends are the same distance from its centre.
                if let (Some(pc), Some(pa), Some(pb)) = (v.p(c), v.p(a), v.p(b)) {
                    out.push((v.deps(&[c, a, b], &[]), vec![pc.distance(pa) - pc.distance(pb)]));
                }
            }
            out
        };
        let cost = |r: &[(Vec<usize>, Vec<f64>)]| r.iter().flat_map(|(_, v)| v.iter()).map(|x| x * x).sum::<f64>();
        let mut r = residuals(&vars);
        let mut c0 = cost(&r);
        let mut lambda = 1.0e-3;
        for _ in 0..120 {
            if c0 < TOLERANCE * TOLERANCE || vars.x.is_empty() {
                break;
            }
            // Derivatives per constraint by perturbing its own variables.
            let mut row_list: Vec<Row> = Vec::new();
            for c in all.iter().copied() {
                if let Some(row) = jacobian_rows(self, &vars, |v| residual(self, v, c)) {
                    row_list.push(row);
                }
            }
            for &(c, aa, b) in &arcs {
                let f = |v: &Vars| {
                    let (pc, pa, pb) = (v.p(c)?, v.p(aa)?, v.p(b)?);
                    Some((v.deps(&[c, aa, b], &[]), vec![pc.distance(pa) - pc.distance(pb)]))
                };
                if let Some(row) = jacobian_rows(self, &vars, f) {
                    row_list.push(row);
                }
            }
            // Normal equations over just the variables the constraints touch (an imported
            // outline may have thousands of points, few of them constrained), row by row from a
            // finite-difference Jacobian.
            let mut active: Vec<usize> = row_list.iter().flat_map(|(deps, _, _)| deps.iter().copied()).collect();
            active.sort_unstable();
            active.dedup();
            let slot_of: HashMap<usize, usize> = active.iter().enumerate().map(|(k, &j)| (j, k)).collect();
            let n = active.len();
            if n == 0 {
                break;
            }
            let mut a = vec![0.0; n * n];
            let mut g = vec![0.0; n];
            for (deps, values, grads) in &row_list {
                let deps: Vec<usize> = deps.iter().map(|j| slot_of.get(j).copied().unwrap_or(usize::MAX)).collect();
                for (k, rv) in values.iter().enumerate() {
                    let Some(grad) = grads.get(k) else { continue };
                    for (ia, &ja) in deps.iter().enumerate() {
                        let ga = grad.get(ia).copied().unwrap_or(0.0);
                        if ga == 0.0 {
                            continue;
                        }
                        if let Some(slot) = g.get_mut(ja) {
                            *slot += ga * rv;
                        }
                        for (ib, &jb) in deps.iter().enumerate() {
                            let gb = grad.get(ib).copied().unwrap_or(0.0);
                            if let Some(slot) = a.get_mut(ja * n + jb) {
                                *slot += ga * gb;
                            }
                        }
                    }
                }
            }
            let mut improved = false;
            for _ in 0..12 {
                let mut m = a.clone();
                for j in 0..n {
                    if let Some(d) = m.get_mut(j * n + j) {
                        *d += lambda * (1.0 + *d);
                    }
                }
                let rhs: Vec<f64> = g.iter().map(|x| -x).collect();
                let Some(dx) = solve_dense(m, rhs, n) else {
                    lambda *= 10.0;
                    continue;
                };
                let mut trial = vars.clone();
                for (&j, d) in active.iter().zip(&dx) {
                    if let Some(x) = trial.x.get_mut(j) {
                        *x += d;
                    }
                }
                let rt = residuals(&trial);
                let ct = cost(&rt);
                if ct.is_finite() && ct < c0 {
                    vars = trial;
                    r = rt;
                    c0 = ct;
                    lambda = (lambda * 0.3).max(1.0e-12);
                    improved = true;
                    break;
                }
                lambda *= 8.0;
            }
            if !improved {
                break;
            }
        }
        let worst = r.iter().flat_map(|(_, v)| v.iter()).fold(0.0f64, |m, x| m.max(x.abs()));
        if worst.is_nan() || worst > 1.0e-4 {
            return Err(EngineError::InvalidParams(
                "the sketch cannot meet all its constraints that way (over-constrained, or a dimension it cannot reach)"
                    .into(),
            ));
        }
        // A solution that shrinks a line or a circle to nothing is no solution.
        let mut solved = self.clone();
        vars.write(&mut solved);
        let collapsed = solved.entities.iter().any(|e| match e.geom {
            Geom::Line { a, b } => solved.point(a).zip(solved.point(b)).is_none_or(|(p, q)| p.distance(q) < 1.0e-6),
            Geom::Circle { r, .. } => r < 1.0e-6,
            Geom::Arc { c, a, b } => {
                let (Some(pc), Some(pa), Some(pb)) = (solved.point(c), solved.point(a), solved.point(b)) else {
                    return true;
                };
                pc.distance(pa) < 1.0e-6 || pa.distance(pb) < 1.0e-6
            }
            Geom::Point { .. } => false,
        });
        if collapsed {
            return Err(EngineError::InvalidParams(
                "that would shrink a line or a circle to nothing (the constraints conflict)".into(),
            ));
        }
        // Fixed points sit exactly where they are held.
        for c in solved.constraints.iter().filter(|c| c.kind == Kind::Fix).cloned().collect::<Vec<_>>() {
            if let Some(p) = c.refs.first().and_then(|id| solved.points.iter_mut().find(|p| p.id == *id)) {
                p.at = DVec2::new(c.value, c.value2);
            }
        }
        *self = solved;
        Ok(())
    }

    // ---- Outlines -------------------------------------------------------------------------

    /// The outlines the sketch draws: curves joined end to end into loops (closed) and chains
    /// (open); circles are loops of their own. Construction geometry is left out.
    pub fn paths(&self) -> Vec<Path2> {
        self.paths_without(&[])
    }

    /// The outlines with the curves `skip` names left out (a cut that leaves some lines alone).
    pub fn paths_without(&self, skip: &[u32]) -> Vec<Path2> {
        let mut out = Vec::new();
        // Each joining curve as (from, to, points along it, excluding `from`).
        let mut edges: Vec<(u32, u32, Vec<DVec2>)> = Vec::new();
        for e in self.entities.iter().filter(|e| !e.construction && !skip.contains(&e.id)) {
            match e.geom {
                Geom::Line { a, b } => {
                    if let Some(pb) = self.point(b) {
                        edges.push((a, b, vec![pb]));
                    }
                }
                Geom::Arc { c, a, b } => {
                    let (Some(pc), Some(pa), Some(pb)) = (self.point(c), self.point(a), self.point(b)) else {
                        continue;
                    };
                    edges.push((a, b, arc_points(pc, pa, pb).into_iter().skip(1).collect()));
                }
                Geom::Circle { c, r } => {
                    let Some(pc) = self.point(c) else { continue };
                    let n = segments(r, TAU);
                    let points = (0..n).map(|k| pc + DVec2::from_angle(TAU * k as f64 / n as f64) * r).collect();
                    out.push(Path2 { points, closed: true });
                }
                Geom::Point { .. } => {}
            }
        }
        let mut used = vec![false; edges.len()];
        let degree = |id: u32| edges.iter().filter(|(a, b, _)| *a == id || *b == id).count();
        let ends: Vec<u32> = edges.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
        // Open chains from free ends first, then whatever leaves a junction (three or more curves
        // meeting), then plain loops. Every walk stops at a junction, so a line ending on a
        // corner of a rectangle leaves the rectangle a closed loop of its own.
        let mut starts: Vec<u32> = ends.iter().copied().filter(|&p| degree(p) == 1).collect();
        starts.extend(ends.iter().copied().filter(|&p| degree(p) >= 3));
        starts.extend(ends.iter().copied());
        for start in starts {
            while let Some(first) = (0..edges.len()).find(|&i| {
                !used.get(i).copied().unwrap_or(true)
                    && edges.get(i).is_some_and(|(a, b, _)| *a == start || *b == start)
            }) {
                let Some(p0) = self.point(start) else { break };
                let mut points = vec![p0];
                let mut at = start;
                let mut next = Some(first);
                while let Some(i) = next {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    let Some((a, b, along)) = edges.get(i) else { break };
                    if *a == at {
                        points.extend(along.iter().copied());
                        at = *b;
                    } else {
                        // Walked backwards: the points from the far end, then back to `a`.
                        let mut back: Vec<DVec2> = along.iter().rev().skip(1).copied().collect();
                        if let Some(pa) = self.point(*a) {
                            back.push(pa);
                        }
                        points.extend(back);
                        at = *a;
                    }
                    if at == start || degree(at) != 2 {
                        break;
                    }
                    next = (0..edges.len()).find(|&j| {
                        !used.get(j).copied().unwrap_or(true)
                            && edges.get(j).is_some_and(|(a, b, _)| *a == at || *b == at)
                    });
                }
                let closed = at == start && points.len() > 2;
                if closed {
                    points.pop();
                }
                out.push(Path2 { points, closed });
            }
        }
        out
    }

    /// Each curve of the outline (not construction) by id, as points along it.
    pub fn curves(&self) -> Vec<(u32, Vec<DVec2>)> {
        self.entities
            .iter()
            .filter(|e| !e.construction)
            .filter_map(|e| {
                let pts = match e.geom {
                    Geom::Line { a, b } => vec![self.point(a)?, self.point(b)?],
                    Geom::Arc { c, a, b } => arc_points(self.point(c)?, self.point(a)?, self.point(b)?),
                    Geom::Circle { c, r } => {
                        let pc = self.point(c)?;
                        let n = segments(r, TAU);
                        (0..=n).map(|k| pc + DVec2::from_angle(TAU * k as f64 / n as f64) * r).collect()
                    }
                    Geom::Point { .. } => return None,
                };
                Some((e.id, pts))
            })
            .collect()
    }

    /// A sketch on `plane` drawing `paths` as lines end to end (each closed one a loop), so
    /// constraints and dimensions can be put on an imported outline. None if it has more points
    /// than a sketch holds.
    pub fn from_paths(plane: Plane, paths: &[Path2]) -> Option<Self> {
        let total: usize = paths.iter().map(|p| p.points.len()).sum();
        if total + 1 > MAX_POINTS || total * 2 + 2 > MAX_ITEMS {
            return None;
        }
        let mut s = Self::new(plane);
        for path in paths {
            let mut pts: Vec<DVec2> = Vec::with_capacity(path.points.len());
            for &p in &path.points {
                if p.is_finite() && pts.last().is_none_or(|q: &DVec2| q.distance(p) > 1.0e-6) {
                    pts.push(p);
                }
            }
            if path.closed && pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) < 1.0e-6)
            {
                pts.pop();
            }
            if pts.len() < 2 {
                continue;
            }
            let ids: Vec<u32> = pts.iter().map(|&p| s.add_point(p)).collect();
            let closed = path.closed && ids.len() > 2;
            let count = if closed { ids.len() } else { ids.len() - 1 };
            for k in 0..count {
                let (Some(&a), Some(&b)) = (ids.get(k), ids.get((k + 1) % ids.len())) else { continue };
                let id = s.fresh();
                s.entities.push(Entity { id, geom: Geom::Line { a, b }, construction: false });
            }
        }
        Some(s)
    }

    // ---- Files ----------------------------------------------------------------------------

    pub fn to_json(&self) -> Value {
        json!({
            "plane": self.plane.id(),
            "points": self.points.iter().map(|p| json!([p.id, p.at.x, p.at.y])).collect::<Vec<_>>(),
            "entities": self.entities.iter().map(|e| {
                let (kind, refs, r) = match e.geom {
                    Geom::Line { a, b } => ("line", vec![a, b], 0.0),
                    Geom::Circle { c, r } => ("circle", vec![c], r),
                    Geom::Arc { c, a, b } => ("arc", vec![c, a, b], 0.0),
                    Geom::Point { p } => ("point", vec![p], 0.0),
                };
                json!({ "id": e.id, "kind": kind, "refs": refs, "r": r, "construction": e.construction })
            }).collect::<Vec<_>>(),
            "labels": self.labels.iter().map(|(id, at)| json!([id, at.x, at.y])).collect::<Vec<_>>(),
            "constraints": self.constraints.iter().map(|c| json!({
                "id": c.id, "kind": c.kind.id(), "refs": c.refs, "value": c.value, "value2": c.value2,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(v: &Value) -> EngineResult<Self> {
        let bad = || EngineError::InvalidParams("the sketch in the file is damaged".into());
        let plane = v.get("plane").and_then(Value::as_str).and_then(Plane::from_id).ok_or_else(bad)?;
        let mut s = Self {
            plane,
            points: Vec::new(),
            entities: Vec::new(),
            constraints: Vec::new(),
            labels: Vec::new(),
            next: ORIGIN,
        };
        let id = |x: &Value| x.as_u64().and_then(|n| u32::try_from(n).ok());
        let num = |x: Option<&Value>| x.and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() < 1.0e9);
        for p in v.get("points").and_then(Value::as_array).into_iter().flatten().take(MAX_POINTS) {
            let a = p.as_array().ok_or_else(bad)?;
            let pid = a.first().and_then(id).ok_or_else(bad)?;
            s.points.push(SPoint {
                id: pid,
                at: DVec2::new(num(a.get(1)).ok_or_else(bad)?, num(a.get(2)).ok_or_else(bad)?),
            });
        }
        let refs = |x: &Value| -> Vec<u32> {
            x.get("refs")
                .and_then(Value::as_array)
                .map(|a| a.iter().take(8).filter_map(id).collect())
                .unwrap_or_default()
        };
        for e in v.get("entities").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS) {
            let eid = e.get("id").and_then(id).ok_or_else(bad)?;
            let r = refs(e);
            let at = |k: usize| r.get(k).copied().ok_or_else(bad);
            let geom = match e.get("kind").and_then(Value::as_str) {
                Some("line") => Geom::Line { a: at(0)?, b: at(1)? },
                Some("circle") => Geom::Circle { c: at(0)?, r: num(e.get("r")).filter(|r| *r > 0.0).ok_or_else(bad)? },
                Some("arc") => Geom::Arc { c: at(0)?, a: at(1)?, b: at(2)? },
                Some("point") => Geom::Point { p: at(0)? },
                _ => return Err(bad()),
            };
            let construction = e.get("construction").and_then(Value::as_bool).unwrap_or(false);
            s.entities.push(Entity { id: eid, geom, construction });
        }
        for c in v.get("constraints").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS) {
            let kind = c.get("kind").and_then(Value::as_str).and_then(Kind::from_id).ok_or_else(bad)?;
            s.constraints.push(Constraint {
                id: c.get("id").and_then(id).ok_or_else(bad)?,
                kind,
                refs: refs(c),
                value: num(c.get("value")).unwrap_or(0.0),
                value2: num(c.get("value2")).unwrap_or(0.0),
            });
        }
        for l in v.get("labels").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS) {
            let a = l.as_array().ok_or_else(bad)?;
            if let (Some(lid), Some(x), Some(y)) = (a.first().and_then(id), num(a.get(1)), num(a.get(2))) {
                s.labels.push((lid, DVec2::new(x, y)));
            }
        }
        // Everything refers to what is there.
        let point_ids: Vec<u32> = s.points.iter().map(|p| p.id).collect();
        for e in &s.entities {
            let ok = match e.geom {
                Geom::Line { a, b } => point_ids.contains(&a) && point_ids.contains(&b),
                Geom::Circle { c, .. } => point_ids.contains(&c),
                Geom::Arc { c, a, b } => [c, a, b].iter().all(|p| point_ids.contains(p)),
                Geom::Point { p } => point_ids.contains(&p),
            };
            if !ok {
                return Err(bad());
            }
        }
        s.next = s
            .points
            .iter()
            .map(|p| p.id)
            .chain(s.entities.iter().map(|e| e.id))
            .chain(s.constraints.iter().map(|c| c.id))
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        if s.point(ORIGIN).is_none() {
            s.points.push(SPoint { id: ORIGIN, at: DVec2::ZERO });
        }
        Ok(s)
    }

    /// Item ids as `p3` (point), `e5` (curve) and `c7` (constraint), for commands.
    pub fn item_name(&self, id: u32) -> String {
        if id & MIDPOINT != 0 {
            format!("m{}", id & !MIDPOINT)
        } else if self.point(id).is_some() {
            format!("p{id}")
        } else if self.entity(id).is_some() {
            format!("e{id}")
        } else {
            format!("c{id}")
        }
    }

    /// Reads an item name (`p3`, `e5`, `c7` or just `5`).
    pub fn parse_item(name: &str) -> Option<u32> {
        if let Some(line) = name.strip_prefix('m') {
            return line.parse::<u32>().ok().filter(|l| l & MIDPOINT == 0).map(|l| l | MIDPOINT);
        }
        name.trim_start_matches(['p', 'e', 'c']).parse().ok()
    }

    /// Where a line's midpoint is, for a [`MIDPOINT`] item.
    pub fn midpoint(&self, item: u32) -> Option<DVec2> {
        if item & MIDPOINT == 0 {
            return None;
        }
        let (a, b) = self.line(item & !MIDPOINT)?;
        Some((a + b) * 0.5)
    }

    /// The items with each line midpoint ([`MIDPOINT`]) made a real point: one already held at
    /// that midpoint, or a new construction point held there.
    pub fn resolve(&mut self, items: &[u32]) -> EngineResult<Vec<u32>> {
        let mut out = Vec::with_capacity(items.len());
        for &i in items {
            if i & MIDPOINT == 0 {
                out.push(i);
                continue;
            }
            let line = i & !MIDPOINT;
            let at = self.midpoint(i).ok_or_else(|| EngineError::InvalidParams("that line is gone".into()))?;
            let held = self
                .constraints
                .iter()
                .find(|c| c.kind == Kind::Midpoint && c.refs.get(1) == Some(&line))
                .and_then(|c| c.refs.first().copied());
            let p = match held {
                Some(p) => p,
                None => {
                    self.check_room(1, 2)?;
                    let p = self.add_point(at);
                    let e = self.fresh();
                    self.entities.push(Entity { id: e, geom: Geom::Point { p }, construction: true });
                    let c = self.fresh();
                    self.constraints.push(Constraint {
                        id: c,
                        kind: Kind::Midpoint,
                        refs: vec![p, line],
                        value: 0.0,
                        value2: 0.0,
                    });
                    p
                }
            };
            out.push(p);
        }
        Ok(out)
    }

    /// How many more numbers would fix every point (0 when fully constrained), roughly: the
    /// variables less the independent constraints.
    pub fn freedom(&self) -> usize {
        let vars = Vars::of(self);
        let n = vars.x.len();
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut add = |row: (Vec<usize>, Vec<f64>, Vec<Vec<f64>>)| {
            for grad in row.2 {
                let mut full = vec![0.0; n];
                for (k, &j) in row.0.iter().enumerate() {
                    if let (Some(slot), Some(g)) = (full.get_mut(j), grad.get(k)) {
                        *slot = *g;
                    }
                }
                rows.push(full);
            }
        };
        for c in &self.constraints {
            if let Some(row) = jacobian_rows(self, &vars, |v| residual(self, v, c)) {
                add(row);
            }
        }
        for e in &self.entities {
            if let Geom::Arc { c, a, b } = e.geom {
                let f = |v: &Vars| {
                    let (pc, pa, pb) = (v.p(c)?, v.p(a)?, v.p(b)?);
                    Some((v.deps(&[c, a, b], &[]), vec![pc.distance(pa) - pc.distance(pb)]))
                };
                if let Some(row) = jacobian_rows(self, &vars, f) {
                    add(row);
                }
            }
        }
        n.saturating_sub(rank(rows, n))
    }
}

fn check_value(kind: Kind, value: f64) -> EngineResult<()> {
    let ok = match kind {
        Kind::Angle => value.is_finite() && value.abs() <= 360.0,
        _ => (0.0..=1.0e6).contains(&value),
    };
    if ok { Ok(()) } else { Err(EngineError::InvalidParams(format!("{value} is not a size this can have"))) }
}

/// The variables: every point's x and y, every circle's radius.
#[derive(Clone)]
struct Vars {
    x: Vec<f64>,
    points: HashMap<u32, usize>,
    radii: HashMap<u32, usize>,
}

impl Vars {
    fn of(s: &Sketch) -> Self {
        let mut x = Vec::new();
        let mut points = HashMap::new();
        let mut radii = HashMap::new();
        for p in &s.points {
            points.insert(p.id, x.len());
            x.push(p.at.x);
            x.push(p.at.y);
        }
        for e in &s.entities {
            if let Geom::Circle { r, .. } = e.geom {
                radii.insert(e.id, x.len());
                x.push(r);
            }
        }
        Self { x, points, radii }
    }

    fn p(&self, id: u32) -> Option<DVec2> {
        let i = *self.points.get(&id)?;
        Some(DVec2::new(*self.x.get(i)?, *self.x.get(i + 1)?))
    }

    fn r(&self, circle: u32) -> Option<f64> {
        self.x.get(*self.radii.get(&circle)?).copied()
    }

    /// The variables of these points and circles.
    fn deps(&self, points: &[u32], circles: &[u32]) -> Vec<usize> {
        let mut out = Vec::new();
        for p in points {
            if let Some(&i) = self.points.get(p)
                && !out.contains(&i)
            {
                out.push(i);
                out.push(i + 1);
            }
        }
        for c in circles {
            if let Some(&i) = self.radii.get(c) {
                out.push(i);
            }
        }
        out
    }

    fn write(&self, s: &mut Sketch) {
        for p in &mut s.points {
            if let Some(at) = self.p(p.id) {
                p.at = at;
            }
        }
        for e in &mut s.entities {
            if let Geom::Circle { r, .. } = &mut e.geom
                && let Some(v) = self.r(e.id)
            {
                *r = v.abs();
            }
        }
    }
}

/// A curve as the solver sees it: its points and, for a circle, its radius variable.
enum Curve {
    Line(DVec2, DVec2),
    Circle(DVec2, f64),
}

fn curve(s: &Sketch, v: &Vars, id: u32) -> Option<(Curve, Vec<u32>, Vec<u32>)> {
    match s.entity(id)?.geom {
        Geom::Line { a, b } => Some((Curve::Line(v.p(a)?, v.p(b)?), vec![a, b], vec![])),
        Geom::Circle { c, .. } => Some((Curve::Circle(v.p(c)?, v.r(id)?), vec![c], vec![id])),
        Geom::Arc { c, a, .. } => {
            let (pc, pa) = (v.p(c)?, v.p(a)?);
            Some((Curve::Circle(pc, pc.distance(pa)), vec![c, a], vec![]))
        }
        Geom::Point { .. } => None,
    }
}

/// How far constraint `c` is from holding, and the variables that changes with.
fn residual(s: &Sketch, v: &Vars, c: &Constraint) -> Option<(Vec<usize>, Vec<f64>)> {
    let r0 = |i: usize| c.refs.get(i).copied();
    let line = |id: u32| match curve(s, v, id)? {
        (Curve::Line(a, b), pts, _) => Some((a, b, pts)),
        _ => None,
    };
    Some(match c.kind {
        Kind::Horizontal | Kind::Vertical => {
            let (a, b, pts) = line(r0(0)?)?;
            let d = if c.kind == Kind::Horizontal { b.y - a.y } else { b.x - a.x };
            (v.deps(&pts, &[]), vec![d])
        }
        Kind::Parallel | Kind::Perpendicular => {
            let (a0, a1, mut pts) = line(r0(0)?)?;
            let (b0, b1, more) = line(r0(1)?)?;
            pts.extend(more);
            let (da, db) = (a1 - a0, b1 - b0);
            let la = da.length().max(1.0e-9);
            let lb = db.length().max(1.0e-9);
            let f = if c.kind == Kind::Parallel { da.perp_dot(db) } else { da.dot(db) };
            // Scaled to millimetres at the size of the lines.
            (v.deps(&pts, &[]), vec![f / la / lb * (la + lb) * 0.5])
        }
        Kind::Equal => {
            let (ca, pa, ra) = curve(s, v, r0(0)?)?;
            let (cb, pb, rb) = curve(s, v, r0(1)?)?;
            let size = |c: &Curve| match c {
                Curve::Line(a, b) => a.distance(*b),
                Curve::Circle(_, r) => *r,
            };
            let pts: Vec<u32> = pa.into_iter().chain(pb).collect();
            let rads: Vec<u32> = ra.into_iter().chain(rb).collect();
            (v.deps(&pts, &rads), vec![size(&ca) - size(&cb)])
        }
        Kind::Midpoint => {
            let p = r0(0)?;
            let (a, b, mut pts) = line(r0(1)?)?;
            let m = (a + b) * 0.5 - v.p(p)?;
            pts.push(p);
            (v.deps(&pts, &[]), vec![m.x, m.y])
        }
        Kind::OnCurve => {
            let p = r0(0)?;
            let at = v.p(p)?;
            let (cv, mut pts, rads) = curve(s, v, r0(1)?)?;
            pts.push(p);
            let d = match cv {
                Curve::Line(a, b) => point_line(at, a, b),
                Curve::Circle(c, r) => at.distance(c) - r,
            };
            (v.deps(&pts, &rads), vec![d])
        }
        Kind::Tangent => {
            let (ca, pa, ra) = curve(s, v, r0(0)?)?;
            let (cb, pb, rb) = curve(s, v, r0(1)?)?;
            let pts: Vec<u32> = pa.into_iter().chain(pb).collect();
            let rads: Vec<u32> = ra.into_iter().chain(rb).collect();
            let d = match (ca, cb) {
                (Curve::Line(a, b), Curve::Circle(c, r)) | (Curve::Circle(c, r), Curve::Line(a, b)) => {
                    point_line(c, a, b).abs() - r
                }
                (Curve::Circle(c1, r1), Curve::Circle(c2, r2)) => {
                    let d = c1.distance(c2);
                    if c.value2 > 0.5 { d - (r1 - r2).abs() } else { d - (r1 + r2) }
                }
                _ => return None,
            };
            (v.deps(&pts, &rads), vec![d])
        }
        Kind::Concentric => {
            let (ca, pa, _) = curve(s, v, r0(0)?)?;
            let (cb, pb, _) = curve(s, v, r0(1)?)?;
            let (Curve::Circle(c1, _), Curve::Circle(c2, _)) = (ca, cb) else { return None };
            let pts: Vec<u32> = pa.into_iter().chain(pb).collect();
            (v.deps(&pts, &[]), vec![c1.x - c2.x, c1.y - c2.y])
        }
        Kind::Fix => {
            let p = r0(0)?;
            let at = v.p(p)?;
            (v.deps(&[p], &[]), vec![at.x - c.value, at.y - c.value2])
        }
        Kind::Distance | Kind::HDistance | Kind::VDistance => {
            let p = r0(0)?;
            let q = r0(1)?;
            let at = v.p(p)?;
            if let Some((a, b, mut pts)) = line(q) {
                // A point's distance from a line.
                pts.push(p);
                return Some((v.deps(&pts, &[]), vec![point_line(at, a, b).abs() - c.value]));
            }
            let to = v.p(q)?;
            // Lined up (an alignment) is the plain difference, smooth through zero; a size
            // across or up is how far, either way.
            let aligned = Kind::is_alignment(c);
            let d = match c.kind {
                Kind::HDistance if aligned => to.x - at.x,
                Kind::VDistance if aligned => to.y - at.y,
                Kind::HDistance => (to.x - at.x).abs(),
                Kind::VDistance => (to.y - at.y).abs(),
                _ => at.distance(to),
            };
            (v.deps(&[p, q], &[]), vec![d - c.value])
        }
        Kind::Radius | Kind::Diameter => {
            let (cv, pts, rads) = curve(s, v, r0(0)?)?;
            let Curve::Circle(_, r) = cv else { return None };
            let size = if c.kind == Kind::Diameter { 2.0 * r } else { r };
            (v.deps(&pts, &rads), vec![size - c.value])
        }
        Kind::Angle => {
            let (a0, a1, mut pts) = line(r0(0)?)?;
            let (b0, b1, more) = line(r0(1)?)?;
            pts.extend(more);
            let (fa, fb) = angle_flips(c.value2);
            let angle = ((a1 - a0) * fa).angle_to((b1 - b0) * fb);
            let mut d = angle - c.value.to_radians();
            while d > PI {
                d -= TAU;
            }
            while d < -PI {
                d += TAU;
            }
            (v.deps(&pts, &[]), vec![d * 100.0])
        }
    })
}

/// A residual's variables, values, and the values' derivatives by each variable.
type Row = (Vec<usize>, Vec<f64>, Vec<Vec<f64>>);

/// A residual's values and their derivatives by each of its variables (central differences).
fn jacobian_rows(_s: &Sketch, vars: &Vars, f: impl Fn(&Vars) -> Option<(Vec<usize>, Vec<f64>)>) -> Option<Row> {
    let (deps, values) = f(vars)?;
    let mut grads = vec![vec![0.0; deps.len()]; values.len()];
    let mut probe = vars.clone();
    for (k, &j) in deps.iter().enumerate() {
        let xj = *vars.x.get(j)?;
        let h = 1.0e-6 * xj.abs().max(1.0);
        *probe.x.get_mut(j)? = xj + h;
        let plus = f(&probe).map(|(_, v)| v);
        *probe.x.get_mut(j)? = xj - h;
        let minus = f(&probe).map(|(_, v)| v);
        *probe.x.get_mut(j)? = xj;
        if let (Some(p), Some(m)) = (plus, minus) {
            for (row, (vp, vm)) in grads.iter_mut().zip(p.iter().zip(&m)) {
                if let Some(slot) = row.get_mut(k) {
                    *slot = (vp - vm) / (2.0 * h);
                }
            }
        }
    }
    Some((deps, values, grads))
}

/// Solves the n × n system `m x = rhs` by Gaussian elimination with partial pivoting.
fn solve_dense(mut m: Vec<f64>, mut rhs: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let pivot = (col..n).max_by(|&a, &b| {
            m.get(a * n + col).map_or(0.0, |v| v.abs()).total_cmp(&m.get(b * n + col).map_or(0.0, |v| v.abs()))
        })?;
        let pv = *m.get(pivot * n + col)?;
        if pv.abs() < 1.0e-14 {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                m.swap(pivot * n + k, col * n + k);
            }
            rhs.swap(pivot, col);
        }
        for row in (col + 1)..n {
            let f = *m.get(row * n + col)? / pv;
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                let v = *m.get(col * n + k)?;
                *m.get_mut(row * n + k)? -= f * v;
            }
            let v = *rhs.get(col)?;
            *rhs.get_mut(row)? -= f * v;
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let mut sum = *rhs.get(row)?;
        for k in (row + 1)..n {
            sum -= m.get(row * n + k)? * x.get(k)?;
        }
        *x.get_mut(row)? = sum / m.get(row * n + row)?;
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// The rank of `rows` (each `n` long).
fn rank(mut rows: Vec<Vec<f64>>, n: usize) -> usize {
    let mut rank = 0;
    for col in 0..n {
        let Some(pivot) = (rank..rows.len()).max_by(|&a, &b| {
            let v = |r: usize| rows.get(r).and_then(|row| row.get(col)).map_or(0.0, |v| v.abs());
            v(a).total_cmp(&v(b))
        }) else {
            break;
        };
        let pv = rows.get(pivot).and_then(|r| r.get(col)).copied().unwrap_or(0.0);
        if pv.abs() < 1.0e-9 {
            continue;
        }
        rows.swap(pivot, rank);
        let Some(top) = rows.get(rank).cloned() else { break };
        for (i, row) in rows.iter_mut().enumerate() {
            if i == rank {
                continue;
            }
            let f = row.get(col).copied().unwrap_or(0.0) / pv;
            if f != 0.0 {
                for (x, t) in row.iter_mut().zip(&top) {
                    *x -= f * t;
                }
            }
        }
        rank += 1;
    }
    rank
}

/// The directions an angle between two lines is measured along: from where they meet (or would)
/// out towards each line's far end.
pub fn angle_rays(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> (DVec2, DVec2) {
    let (da, db) = (a1 - a0, b1 - b0);
    let den = da.perp_dot(db);
    if den.abs() < 1.0e-12 {
        return (da, db);
    }
    let vertex = a0 + da * ((b0 - a0).perp_dot(db) / den);
    let away = |p: DVec2, q: DVec2, d: DVec2| if p.distance(vertex) > q.distance(vertex) { -d } else { d };
    (away(a0, a1, da), away(b0, b1, db))
}

/// The signs an angle dimension's stored `value2` puts on its two lines' directions.
pub fn angle_flips(value2: f64) -> (f64, f64) {
    let bits = value2.round() as i64;
    (if bits & 1 != 0 { -1.0 } else { 1.0 }, if bits & 2 != 0 { -1.0 } else { 1.0 })
}

/// Signed distance of `p` from the line through `a` and `b`.
pub fn point_line(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    let len = d.length();
    if len < 1.0e-12 { p.distance(a) } else { d.perp_dot(p - a) / len }
}

pub fn closest_on_segment(a: DVec2, b: DVec2, p: DVec2) -> DVec2 {
    let d = b - a;
    let len2 = d.length_squared();
    if len2 < 1.0e-18 {
        return a;
    }
    a + d * ((p - a).dot(d) / len2).clamp(0.0, 1.0)
}

/// Whether `p` lies in the angle swept counter-clockwise from `a` to `b` about `c`.
pub fn in_arc(c: DVec2, a: DVec2, b: DVec2, p: DVec2) -> bool {
    let start = (a - c).to_angle();
    let sweep = ((b - c).to_angle() - start).rem_euclid(TAU);
    ((p - c).to_angle() - start).rem_euclid(TAU) <= sweep
}

/// The centre of the circle through three points; None when they are in a line.
pub fn circumcentre(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1.0e-9 {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let p = DVec2::new(ux, uy);
    p.is_finite().then_some(p)
}

/// Segments for an arc of `sweep` radians and radius `r`: about one every 3°, more for big ones.
fn segments(r: f64, sweep: f64) -> usize {
    let by_angle = (sweep / 3.0f64.to_radians()).ceil();
    let by_size = (sweep * r / 2.0).ceil();
    by_angle.max(by_size).clamp(4.0, 720.0) as usize
}

/// Points along the arc counter-clockwise from `a` to `b` about `c`, both ends included.
pub fn arc_points(c: DVec2, a: DVec2, b: DVec2) -> Vec<DVec2> {
    let r = c.distance(a);
    let start = (a - c).to_angle();
    let mut sweep = ((b - c).to_angle() - start).rem_euclid(TAU);
    if sweep < 1.0e-9 {
        sweep = TAU;
    }
    let n = segments(r, sweep);
    (0..=n).map(|k| c + DVec2::from_angle(start + sweep * k as f64 / n as f64) * r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve_of(s: &Sketch, id: u32) -> Vec<DVec2> {
        match s.entity(id).map(|e| e.geom) {
            Some(Geom::Arc { c, a, b }) => match (s.point(c), s.point(a), s.point(b)) {
                (Some(c), Some(a), Some(b)) => arc_points(c, a, b),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1.0e-4
    }

    #[test]
    fn a_rectangle_is_four_lines_level_and_upright_and_closed() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let lines = s.add_rectangle(DVec2::new(10.0, 10.0), DVec2::new(110.0, 60.0), false, 1.0)?;
        assert_eq!(lines.len(), 4);
        let paths = s.paths();
        assert_eq!(paths.len(), 1);
        assert!(paths.first().is_some_and(|p| p.closed && p.points.len() == 4));
        // Width and height dimensions drive it.
        let bottom = lines.first().copied().unwrap_or_default();
        let w = s.dimension(&[bottom], None, Some(200.0))?;
        let (a, b) = s.line(bottom).unwrap_or_default();
        assert!(near(a.distance(b), 200.0));
        assert!(near(a.y, b.y), "still level");
        s.set_dimension(w, 150.0)?;
        let (a, b) = s.line(bottom).unwrap_or_default();
        assert!(near(a.distance(b), 150.0));
        Ok(())
    }

    #[test]
    fn lines_snap_to_ends_midpoints_and_level() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let first = s.add_line(DVec2::new(0.4, 0.3), DVec2::new(100.0, 1.2), 2.0)?;
        // It starts on the origin (shared) and was made level.
        let (a, b) = s.line(first).unwrap_or_default();
        assert_eq!(a, DVec2::ZERO);
        assert!(near(a.y, b.y));
        assert!(s.constraints.iter().any(|c| c.kind == Kind::Horizontal));
        // The next line starts at its end: the same point.
        let second = s.add_line(b + DVec2::new(0.5, 0.5), DVec2::new(101.0, 80.0), 2.0)?;
        let (
            Some(Entity { geom: Geom::Line { b: end, .. }, .. }),
            Some(Entity { geom: Geom::Line { a: start, .. }, .. }),
        ) = (s.entity(first).copied(), s.entity(second).copied())
        else {
            return Err(EngineError::Internal("lines".into()));
        };
        assert_eq!(end, start);
        assert!(s.constraints.iter().any(|c| c.kind == Kind::Vertical));
        // One from the first line's middle snaps to it.
        s.add_line(DVec2::new(50.5, 0.0), DVec2::new(50.0, -40.0), 2.0)?;
        assert!(s.constraints.iter().any(|c| c.kind == Kind::Midpoint));
        Ok(())
    }

    #[test]
    fn a_line_ending_on_a_corner_leaves_the_rectangle_closed() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        s.add_rectangle(DVec2::new(-50.0, 0.0), DVec2::new(50.0, 80.0), false, 0.5)?;
        s.add_line(DVec2::new(-50.0, 80.0), DVec2::new(0.0, 120.0), 0.5)?;
        let paths = s.paths();
        assert_eq!(paths.iter().filter(|p| p.closed && p.points.len() == 4).count(), 1, "{paths:?}");
        assert_eq!(paths.iter().filter(|p| !p.closed && p.points.len() == 2).count(), 1);
        Ok(())
    }

    #[test]
    fn perpendicular_lines_and_angle_dimensions_solve() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let base = s.add_line(DVec2::ZERO, DVec2::new(100.0, 0.0), 0.5)?;
        let side = s.add_line(DVec2::new(100.0, 0.0), DVec2::new(150.0, 80.0), 0.5)?;
        s.constrain("perpendicular", &[base, side])?;
        let ((a0, a1), (b0, b1)) = (s.line(base).unwrap_or_default(), s.line(side).unwrap_or_default());
        assert!((a1 - a0).dot(b1 - b0).abs() < 1.0e-3, "square");
        let slope = s.add_line(DVec2::new(1.0, 1.0), DVec2::new(80.0, 60.0), 2.0)?;
        let (kind, _, now) = s.measure(&[base, slope], None)?;
        assert_eq!(kind, Kind::Angle);
        assert!((now - 36.87).abs() < 0.5, "{now}");
        // Two lines meeting at a corner: the angle between them there, not its outside.
        let mut t = Sketch::new(Plane::Front);
        let floor = t.add_line(DVec2::new(-200.0, 50.0), DVec2::new(100.0, 50.0), 0.5)?;
        let lean = t.add_line(DVec2::new(100.0, 50.0), DVec2::new(-50.0, 300.0), 0.5)?;
        let a = t.dimension(&[floor, lean], None, None)?;
        assert!(t.constraint(a).is_some_and(|c| (c.value.abs() - 59.036).abs() < 0.01), "{:?}", t.constraint(a));
        t.set_dimension(a, 45.0)?;
        let ((f0, f1), (l0, l1)) = (t.line(floor).unwrap_or_default(), t.line(lean).unwrap_or_default());
        let (ra, rb) = angle_rays(f0, f1, l0, l1);
        assert!((ra.angle_to(rb).to_degrees().abs() - 45.0).abs() < 1.0e-3);
        let d = s.dimension(&[base, slope], None, None)?;
        s.set_dimension(d, 45.0)?;
        let (c0, c1) = s.line(slope).unwrap_or_default();
        let (a0, a1) = s.line(base).unwrap_or_default();
        assert!(((a1 - a0).angle_to(c1 - c0).to_degrees() - 45.0).abs() < 1.0e-3);
        Ok(())
    }

    #[test]
    fn sketches_move_and_dimensions_keep_where_they_were_put() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let lines = s.add_rectangle(DVec2::new(10.0, 10.0), DVec2::new(110.0, 60.0), false, 0.5)?;
        let w = s.dimension(&lines[..1], None, None)?;
        s.move_items(&[], DVec2::new(30.0, 5.0))?;
        let (a, b) = s.line(lines[0]).unwrap_or_default();
        assert!(near(a.x.min(b.x), 40.0) && near(a.y, 15.0), "moved: {a:?}");
        assert!(near(a.distance(b), 100.0), "same size");
        assert_eq!(s.point(ORIGIN), Some(DVec2::ZERO), "the origin stays");
        s.move_label(w, DVec2::new(60.0, -20.0))?;
        let back = Sketch::from_json(&s.to_json())?;
        assert_eq!(back.label(w), Some(DVec2::new(60.0, -20.0)));
        // A dimension to the origin holds the sketch there.
        let corner = s.points.iter().map(|p| p.id).find(|&p| p != ORIGIN).unwrap_or_default();
        s.dimension(&[ORIGIN, corner], Some("horizontal"), None)?;
        let before = s.point(corner);
        s.move_items(&[], DVec2::new(50.0, 0.0))?;
        assert!(s.point(corner).zip(before).is_some_and(|(p, q)| near(p.x, q.x)), "held across");
        Ok(())
    }

    #[test]
    fn an_angle_typed_keeps_its_way_round() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let base = s.add_line(DVec2::new(10.0, 10.0), DVec2::new(110.0, 10.0), 0.5)?;
        let down = s.add_line(DVec2::new(10.0, 50.0), DVec2::new(90.0, -10.0), 0.5)?;
        let d = s.dimension(&[base, down], None, None)?;
        assert!(s.constraint(d).is_some_and(|c| c.value < 0.0), "measured clockwise");
        s.set_dimension(d, 30.0)?;
        let (a0, a1) = s.line(base).unwrap_or_default();
        let (b0, b1) = s.line(down).unwrap_or_default();
        assert!(near((a1 - a0).angle_to(b1 - b0).to_degrees(), -30.0), "still going down");
        Ok(())
    }

    #[test]
    fn the_origin_and_midpoints_take_constraints_and_dimensions() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let l = s.add_line(DVec2::new(40.0, 30.0), DVec2::new(140.0, 80.0), 0.5)?;
        let mid = Sketch::parse_item(&format!("m{l}")).unwrap_or_default();
        assert_eq!(s.item_name(mid), format!("m{l}"));
        // The line's middle straight above the origin, then 50 mm above it.
        s.constrain("vertical", &[ORIGIN, mid])?;
        s.dimension(&[ORIGIN, mid], Some("vertical"), Some(50.0))?;
        let (a, b) = s.line(l).unwrap_or_default();
        let m = (a + b) * 0.5;
        assert!(near(m.x, 0.0) && near(m.y, 50.0), "{m:?}");
        // The same midpoint is used again, not a second point.
        let points = s.points.len();
        s.constrain("horizontal", &[mid, ORIGIN]).ok();
        assert_eq!(s.points.len(), points);
        // An end point of another line onto the origin.
        let other = s.add_line(DVec2::new(-60.0, -40.0), DVec2::new(-10.0, -40.0), 0.5)?;
        let Some(Geom::Line { a: start, .. }) = s.entity(other).map(|e| e.geom) else {
            return Err(EngineError::Internal("line".into()));
        };
        s.constrain("coincident", &[start, ORIGIN])?;
        assert_eq!(s.line(other).map(|(a, _)| a), Some(DVec2::ZERO));
        Ok(())
    }

    #[test]
    fn points_line_up_with_others_and_lines_carry_on_as_arcs_and_chamfers() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        // A point near the vertical axis snaps onto it, and is held there.
        let up = s.add_line(DVec2::new(0.4, 120.0), DVec2::new(80.0, 160.0), 1.0)?;
        let (a, _) = s.line(up).unwrap_or_default();
        assert!(near(a.x, 0.0), "{a:?}");
        assert!(s.constraints.iter().any(|c| Kind::is_alignment(c) && c.refs.first() == Some(&ORIGIN)));
        // Level with that line's end.
        let flat = s.add_line(DVec2::new(200.0, 30.0), DVec2::new(260.0, 160.6), 1.0)?;
        let (_, b) = s.line(flat).unwrap_or_default();
        assert!(near(b.y, 160.0), "{b:?}");
        // A chamfer: 45° off the line it carries on from.
        let base = s.add_line(DVec2::new(-300.0, -100.0), DVec2::new(-100.0, -100.0), 0.5)?;
        let ch = s.add_line(DVec2::new(-100.0, -100.0), DVec2::new(-60.0, -60.0), 0.5)?;
        s.chamfer_angle(ch)?;
        let c = s.constraints.iter().rev().find(|c| c.kind == Kind::Angle).cloned();
        assert!(c.is_some_and(|c| near(c.value.abs(), 45.0)));
        // A tangent arc carrying on from the chamfer's end.
        let arc = s.add_arc(DVec2::new(-60.0, -60.0), DVec2::new(-40.0, -20.0), DVec2::new(-60.0, 20.0), 0.5)?;
        s.tangent_to_previous(arc)?;
        assert_eq!(s.previous(arc).map(|(e, _)| e), Some(ch));
        let _ = base;
        Ok(())
    }

    #[test]
    fn the_gap_between_parallel_lines_can_be_set() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let low = s.add_line(DVec2::new(-150.0, 100.0), DVec2::new(100.0, 100.0), 0.5)?;
        let high = s.add_line(DVec2::new(-100.0, 250.0), DVec2::new(60.0, 250.0), 0.5)?;
        let gap = s.dimension(&[low, high], None, None)?;
        assert!(s.constraint(gap).is_some_and(|c| c.kind == Kind::Distance && near(c.value, 150.0)));
        s.set_dimension(gap, 80.0)?;
        let ((a, _), (b, c)) = (s.line(low).unwrap_or_default(), s.line(high).unwrap_or_default());
        assert!(near((b.y - a.y).abs(), 80.0) && near(b.y, c.y), "{a:?} {b:?} {c:?}");
        Ok(())
    }

    #[test]
    fn an_imported_outline_becomes_lines_that_take_dimensions() -> EngineResult<()> {
        // A big imported loop: hundreds of points, constrained in one place only.
        let n = 600;
        let ring: Vec<DVec2> = (0..n).map(|k| DVec2::from_angle(TAU * k as f64 / n as f64) * 100.0).collect();
        let square =
            vec![DVec2::new(200.0, 0.0), DVec2::new(260.0, 0.0), DVec2::new(260.0, 60.0), DVec2::new(200.0, 60.0)];
        let paths = vec![Path2 { points: ring, closed: true }, Path2 { points: square, closed: true }];
        let mut s = Sketch::from_paths(Plane::Front, &paths).ok_or_else(|| EngineError::Internal("too big".into()))?;
        assert_eq!(s.entities.len(), n + 4);
        assert_eq!(s.paths().iter().filter(|p| p.closed).count(), 2);
        // The square's bottom side dimensioned to 80 mm; the solver touches only its corners.
        let bottom = s
            .entities
            .iter()
            .find(|e| s.line(e.id).is_some_and(|(a, b)| near(a.y, 0.0) && near(b.y, 0.0) && a.x >= 200.0))
            .map(|e| e.id)
            .unwrap_or_default();
        let started = std::time::Instant::now();
        s.dimension(&[bottom], None, Some(80.0))?;
        assert!(started.elapsed().as_secs_f64() < 2.0, "quick even with many points");
        let (a, b) = s.line(bottom).unwrap_or_default();
        assert!(near(a.distance(b), 80.0));
        Ok(())
    }

    #[test]
    fn conflicting_constraints_are_refused() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let l = s.add_line(DVec2::new(10.0, 10.0), DVec2::new(60.0, 10.0), 0.5)?;
        assert!(s.constraints.iter().any(|c| c.kind == Kind::Horizontal));
        let before = s.clone();
        assert!(s.constrain("vertical", &[l]).is_err());
        assert_eq!(s, before, "a refused change leaves nothing behind");
        Ok(())
    }

    #[test]
    fn circles_arcs_and_tangents_solve() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Top);
        let c = s.add_circle(DVec2::new(50.0, 50.0), 20.0, 0.5)?;
        let d = s.dimension(&[c], None, None)?;
        s.set_dimension(d, 30.0)?;
        assert!(s.entity(c).and_then(|e| s.radius(e)).is_some_and(|r| near(r, 15.0)));
        let l = s.add_line(DVec2::new(0.0, 80.0), DVec2::new(100.0, 75.0), 0.1)?;
        s.constrain("tangent", &[l, c])?;
        let (a, b) = s.line(l).unwrap_or_default();
        let centre = s.entity(c).and_then(|e| s.centre(e)).unwrap_or_default();
        assert!(near(point_line(centre, a, b).abs(), 15.0), "the line touches the circle");
        let arc = s.add_arc(DVec2::new(200.0, 0.0), DVec2::new(250.0, 50.0), DVec2::new(300.0, 0.0), 0.1)?;
        let e = s.entity(arc).copied().unwrap_or(Entity { id: 0, geom: Geom::Point { p: 0 }, construction: false });
        assert!(s.radius(&e).is_some_and(|r| near(r, 50.0)));
        // It goes through the middle point (over the top), not round the other side.
        let pts = curve_of(&s, arc);
        assert!(pts.iter().any(|p| p.distance(DVec2::new(250.0, 50.0)) < 1.0), "{pts:?}");
        assert!(pts.iter().all(|p| p.y > -1.0e-6));
        assert!(s.paths().iter().any(|p| !p.closed && p.points.len() > 4));
        Ok(())
    }

    #[test]
    fn sketches_round_trip_through_json() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Right);
        s.add_rectangle(DVec2::new(-20.0, 0.0), DVec2::new(20.0, 50.0), false, 0.5)?;
        s.add_polygon(DVec2::new(0.0, 100.0), DVec2::new(0.0, 130.0), 6, 0.5)?;
        let back = Sketch::from_json(&s.to_json())?;
        assert_eq!(back, s);
        Ok(())
    }

    #[test]
    fn a_fully_dimensioned_rectangle_has_no_freedom_left() -> EngineResult<()> {
        let mut s = Sketch::new(Plane::Front);
        let lines = s.add_rectangle(DVec2::ZERO, DVec2::new(100.0, 50.0), false, 0.5)?;
        assert!(s.freedom() > 0);
        s.dimension(&lines[..1], None, None)?;
        s.dimension(&lines[1..2], None, None)?;
        assert_eq!(s.freedom(), 0, "corner on the origin, width and height set");
        Ok(())
    }
}
