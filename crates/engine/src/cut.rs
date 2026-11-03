//! Cuts: what the wire does to the foam, and the wire paths and kept regions that come of it.
//!
//! - **Profile**: the wire follows a shape through the foam with the table at one angle (or
//!   at several, indexed), entering from the side or the top and leaving the way it came.
//! - **Two-view**: a front shape, then the table turns a quarter and a side shape; the part is
//!   what both leave.
//! - **Lathe**: a half profile (radius against height) cut at many table angles, each pass down
//!   the profile and out at the bottom, turning the table above the foam between passes, so the
//!   part comes out a faceted solid of revolution.
//! - **Slab**: level passes across the block, slicing it into sheets.
//!
//! Each cut gives its wire path and, for the simulation, the regions it keeps: the part is what
//! is left of the stock inside every one of them.

use CosmicCut_geom::offset::signed_area2;
use CosmicCut_geom::region::{self, Shape as Region};
use CosmicCut_io::profile::Path2;
use glam::DVec2;
use serde_json::{Map, Value, json};

use crate::error::{EngineError, EngineResult};
use crate::machine::Machine;
use crate::params;
use crate::shape::{Shape, nest};
use crate::stock::Stock;
use crate::units::DisplayUnits;
use crate::wire::{MoveKind, WirePath, WirePos};

/// Flattening and offset tolerance, mm.
const TOL: f64 = 0.01;
/// Far enough to stand in for "beyond the stock", mm.
const FAR: f64 = 1.0e6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutKind {
    Profile,
    TwoView,
    Lathe,
    Slab,
    /// A profile of a 3D model at every rotation.
    Model,
    /// The wire along selected faces of a model, X, Y and B moving together.
    Swarf,
}

impl CutKind {
    pub const ALL: [Self; 6] = [Self::Model, Self::Swarf, Self::Profile, Self::TwoView, Self::Lathe, Self::Slab];

    pub fn id(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::TwoView => "twoview",
            Self::Lathe => "lathe",
            Self::Slab => "slab",
            Self::Model => "model",
            Self::Swarf => "swarf",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    /// The command that makes this kind of cut.
    pub fn command(self) -> &'static str {
        match self {
            Self::Profile => "cut.profile",
            Self::TwoView => "cut.twoview",
            Self::Lathe => "cut.lathe",
            Self::Slab => "cut.slab",
            Self::Model => "cut.model",
            Self::Swarf => "cut.swarf",
        }
    }

    pub fn from_command(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.command() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Profile => "Profile",
            Self::TwoView => "Two-view",
            Self::Lathe => "Lathe",
            Self::Slab => "Slabs",
            Self::Model => "From model",
            Self::Swarf => "Swarf",
        }
    }
}

/// A region a cut keeps, in the cut plane at table angle `b`: inside the outlines (or outside
/// them, with `outside`), extruded along the wire. A swarf pass keeps one side of the surface
/// its wire swept instead (`sweep`).
#[derive(Clone, Debug, PartialEq)]
pub struct Keep {
    pub b: f64,
    pub regions: Vec<Region>,
    pub outside: bool,
    /// The cut's move after which the waste falls away.
    pub at: usize,
    pub sweep: Option<Sweep>,
}

/// The surface a swarf pass swept: at each height the wire's line, and the part on one side of
/// it. Below the lowest line the wire left level, so everything there stays.
#[derive(Clone, Debug, PartialEq)]
pub struct Sweep {
    /// The face's line (`u` across at table angle `b`) at each height, from the bottom up.
    pub lines: Vec<WirePos>,
    /// The waste is on the +u side of the line (1) or the −u side (−1).
    pub waste: f64,
}

impl Sweep {
    /// How far `p` (table frame) is inside what stays: positive kept, negative waste.
    pub fn inside(&self, p: glam::DVec3) -> f64 {
        let Some(first) = self.lines.first() else { return 1.0e3 };
        if p.z < first.h {
            return 1.0e3;
        }
        let k = self.lines.partition_point(|l| l.h <= p.z);
        let (u, b) = match (k.checked_sub(1).and_then(|i| self.lines.get(i)), self.lines.get(k)) {
            (Some(a), Some(c)) if c.h - a.h > 1.0e-12 => {
                let t = (p.z - a.h) / (c.h - a.h);
                (a.u + (c.u - a.u) * t, a.b + (c.b - a.b) * t)
            }
            (Some(a), _) | (None, Some(a)) => (a.u, a.b),
            (None, None) => return 1.0e3,
        };
        (u - p.dot(crate::wire::u_axis(b))) * self.waste
    }
}

/// One cut in the job.
#[derive(Clone, Debug)]
pub struct Cut {
    pub id: String,
    pub name: String,
    pub kind: CutKind,
    /// The operation it belongs to.
    pub op: String,
    /// The parameters it was made with (its command's, without `name`).
    pub params: Value,
    pub visible: bool,
    pub path: WirePath,
    pub keeps: Vec<Keep>,
    pub warnings: Vec<String>,
    /// The cut that frees the part from the foam under it, run at the very end of the program.
    pub release: Option<WirePath>,
    /// Why it could not be made the last time it was rebuilt.
    pub error: Option<String>,
}

impl Cut {
    pub fn summary_json(&self, machine: &Machine) -> Value {
        let (cut, air, secs) = self.path.stats(machine.rapid, machine.b_rate);
        json!({
            "id": self.id,
            "name": self.name,
            "kind": self.kind.id(),
            "visible": self.visible,
            "params": self.params,
            "moves": self.path.moves.len(),
            "cut_length": cut,
            "air_length": air,
            "seconds": secs,
            "warnings": self.warnings,
            "error": self.error,
        })
    }

    pub fn properties(&self, machine: &Machine, units: DisplayUnits) -> Vec<(String, String)> {
        let (cut, air, secs) = self.path.stats(machine.rapid, machine.b_rate);
        let mut out = vec![
            ("Kind".into(), self.kind.label().into()),
            ("Moves".into(), self.path.moves.len().to_string()),
            ("Cut length".into(), units.fmt_len(cut)),
            ("Air length".into(), units.fmt_len(air)),
            ("Time".into(), fmt_time(secs)),
        ];
        if let Some(Value::Object(p)) = Some(&self.params) {
            for (k, v) in p {
                let shown = match v {
                    // A swarf cut's faces: how many, not every number.
                    Value::Array(a) if k == "faces" => a.len().to_string(),
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| format!("{f:.4}")),
                    other => other.to_string(),
                };
                out.push((k.replace('_', " "), shown));
            }
        }
        if let Some(e) = &self.error {
            out.push(("Error".into(), e.clone()));
        }
        for w in &self.warnings {
            out.push(("Warning".into(), w.clone()));
        }
        out
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}h {:02}m {:02}s", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

/// What a cut needs from the job.
pub struct Context<'a> {
    pub machine: &'a Machine,
    pub stock: &'a Stock,
    pub shapes: &'a [Shape],
    pub models: &'a [crate::model::Model],
    /// Models taken off the table before the cut's operation: the wire may pass through them.
    pub ignore: &'a [String],
}

impl Context<'_> {
    fn shape(&self, p: &Value, key: &str) -> EngineResult<&Shape> {
        let id = params::req_str(p, key)?;
        self.shapes
            .iter()
            .find(|s| s.id == id || s.name == id)
            .ok_or_else(|| EngineError::InvalidParams(format!("there is no shape `{id}`")))
    }
}

/// What a cut comes to.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub path: WirePath,
    pub keeps: Vec<Keep>,
    pub warnings: Vec<String>,
    /// The cut that frees the part from the foam under it, run at the end of the program.
    pub release: Option<WirePath>,
}

pub fn build(cx: &Context<'_>, kind: CutKind, p: &Value) -> EngineResult<Built> {
    let built = match kind {
        CutKind::Profile => profile(cx, p),
        CutKind::TwoView => two_view(cx, p),
        CutKind::Lathe => lathe(cx, p),
        CutKind::Slab => slab(cx, p),
        CutKind::Model => model_cut(cx, p),
        CutKind::Swarf => crate::swarf::swarf(cx, p),
    }?;
    // The wire must not touch the models this cut is not making.
    let own: Vec<String> = match kind {
        CutKind::Model => cut_models(cx, p)?.iter().map(|m| m.id.clone()).collect(),
        // A swarf cut keeps its own parts' wire clear of each other itself.
        CutKind::Swarf => crate::swarf::parts_of(cx, p)?.iter().map(|(m, _)| m.id.clone()).collect(),
        _ => Vec::new(),
    };
    let kerf = params::opt_f64(p, "kerf", 0.0, 50.0)?.unwrap_or(cx.machine.kerf);
    // Models the cut or its operation says are off the table by then do not count.
    let ignored: Vec<&str> = p
        .get("ignore")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let others: Vec<&crate::model::Model> = cx
        .models
        .iter()
        .filter(|m| !own.contains(&m.id) && !ignored.contains(&m.id.as_str()) && !cx.ignore.contains(&m.id))
        .collect();
    crate::interference::check(&others, &built, kerf / 2.0)?;
    Ok(built)
}

/// The models a cut from models makes: those its `models` list names, else the one `model`
/// names, else every model in the job.
pub fn cut_models<'a>(cx: &Context<'a>, p: &Value) -> EngineResult<Vec<&'a crate::model::Model>> {
    let find = |id: &str| {
        cx.models
            .iter()
            .find(|m| m.id == id || m.name == id)
            .ok_or_else(|| EngineError::InvalidParams(format!("there is no model `{id}`")))
    };
    let listed: Vec<&str> = p
        .get("models")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let out: Vec<&crate::model::Model> = if !listed.is_empty() {
        listed.into_iter().map(find).collect::<EngineResult<_>>()?
    } else if let Some(id) = params::opt_str(p, "model")? {
        vec![find(id)?]
    } else {
        cx.models.iter().collect()
    };
    if out.is_empty() {
        return Err(EngineError::InvalidParams("import a model first".into()));
    }
    Ok(out)
}

/// The models as one mesh, so the wire follows the outline of them all together.
fn merged(models: &[&crate::model::Model]) -> EngineResult<std::sync::Arc<CosmicCut_geom::Mesh>> {
    if let [one] = models {
        return Ok(std::sync::Arc::clone(&one.mesh));
    }
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for m in models {
        let base = positions.len() as u32;
        positions.extend_from_slice(m.mesh.positions());
        triangles.extend(m.mesh.triangles().iter().map(|t| t.map(|i| i + base)));
    }
    CosmicCut_geom::Mesh::new(positions, triangles)
        .map(std::sync::Arc::new)
        .map_err(|e| EngineError::InvalidParams(format!("the models cannot be cut together: {e}")))
}

pub const SPACINGS: &[&str] = &["divide", "degrees"];

/// The angles a model is profiled at: half a turn divided into `divisions`, or every `degrees`
/// (the outline seen at B and at B + 180° is the same, mirrored, so half a turn is enough).
pub fn model_angles(p: &Value) -> EngineResult<Vec<f64>> {
    let step = match choice(p, "spacing", SPACINGS)? {
        "degrees" => params::opt_f64(p, "degrees", 0.5, 180.0)?.unwrap_or(22.5),
        _ => 180.0 / params::opt_f64(p, "divisions", 1.0, 180.0)?.unwrap_or(8.0).round().max(1.0),
    };
    let n = ((180.0 - 1.0e-6) / step).floor() as usize + 1;
    Ok((0..n.min(360)).map(|k| step * k as f64).collect())
}

/// The lowest the wire's centre can go, plus half a kerf: outlines below it stay joined to the
/// foam underneath.
pub fn wire_floor(cx: &Context<'_>, kerf: f64) -> f64 {
    cx.machine.wire_y0.max(cx.stock.bottom()) + kerf / 2.0
}

/// A profile of the model at every rotation: at each angle the wire follows the model's outline
/// seen along it, the part held on the foam under it until the part-off at the end.
fn model_cut(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = common(cx, p)?;
    let chosen = cut_models(cx, p)?;
    let model = merged(&chosen)?;
    let angles = model_angles(p)?;
    needs_b(cx, &angles)?;
    let alternate = params::opt_bool(p, "alternate")?.unwrap_or(true);
    let low = wire_floor(cx, c.kerf);
    let mut out = Built { path: WirePath::new(park(cx.machine, cx.stock, 0.0)), ..Built::default() };
    let bottom = f64::from(model.bounds().min.z);
    if bottom < low - 1.0e-6 {
        out.warnings.push(format!(
            "the bottom {:.1} mm of the model is lower than the wire reaches; it stays joined to the foam under it \
             (lift the model or put the stock on a riser to cut it free)",
            low - bottom
        ));
    }
    for (k, b) in angles.iter().enumerate() {
        let paths = crate::model::clip_above(crate::model::silhouette(&model, *b, false, 0.02), low);
        if paths.is_empty() {
            return Err(EngineError::InvalidParams("the model has no outline above what the wire reaches".into()));
        }
        let (lead, direction) = pass_way("left", "ccw", alternate && k % 2 == 1);
        profile_pass(cx, &c, &paths, *b, lead, false, direction, Some(0.0), None, None, &mut out)?;
    }
    if !wants_release(p)? {
        out.release = None;
    }
    finish(cx, &c, &mut out);
    check_fits(cx, &mut out);
    Ok(out)
}

/// Where the shape's anchor goes in the cut plane.
pub const ANCHORS: &[&str] = &["centre", "bottom_centre", "bottom_left", "origin", "absolute"];
pub const LEADS: &[&str] = &["left", "right", "top"];
pub const KEEPS: &[&str] = &["inside", "outside"];
pub const DIRECTIONS: &[&str] = &["ccw", "cw"];
pub const BOTTOMS: &[&str] = &["attached", "through"];

/// Shared settings of cuts.
pub(crate) struct Common {
    pub(crate) kerf: f64,
    pub(crate) compensate: bool,
    pub(crate) feed: f64,
    pub(crate) clearance: f64,
}

pub(crate) fn common(cx: &Context<'_>, p: &Value) -> EngineResult<Common> {
    let m = cx.machine;
    Ok(Common {
        kerf: params::opt_f64(p, "kerf", 0.0, 50.0)?.unwrap_or(m.kerf),
        compensate: params::opt_bool(p, "compensate")?.unwrap_or(true),
        feed: params::opt_f64(p, "feed", 1.0, 1.0e6)?.unwrap_or(m.default_feed).min(m.max_feed),
        clearance: params::opt_f64(p, "clearance", 0.0, 10_000.0)?.unwrap_or(m.clearance),
    })
}

/// The shape scaled, turned and mirrored per `p`, then moved so its anchor lands where `p`
/// (or, by default, the stock's middle at angle `b`) says. Returns the paths in (u, h).
fn place(cx: &Context<'_>, shape: &Shape, p: &Value, b: f64, prefix: &str) -> EngineResult<Vec<Path2>> {
    Ok(placing(cx, shape, p, b, prefix)?.paths(&shape.paths))
}

/// How a shape's points land in the cut plane: scaled, turned and mirrored about its origin, then
/// moved so its anchor sits where `p` says. Worked out from the whole shape, so leaving some of
/// its lines out does not move the rest.
struct Placing {
    scale: f64,
    turn: (f64, f64),
    mirror: bool,
    shift: DVec2,
}

impl Placing {
    fn point(&self, q: DVec2) -> DVec2 {
        let q = if self.mirror { DVec2::new(-q.x, q.y) } else { q } * self.scale;
        let (s, c) = self.turn;
        DVec2::new(q.x * c - q.y * s, q.x * s + q.y * c) + self.shift
    }

    fn paths(&self, paths: &[Path2]) -> Vec<Path2> {
        paths
            .iter()
            .map(|q| Path2 { closed: q.closed, points: q.points.iter().map(|pt| self.point(*pt)).collect() })
            .collect()
    }
}

fn placing(cx: &Context<'_>, shape: &Shape, p: &Value, b: f64, prefix: &str) -> EngineResult<Placing> {
    let key = |k: &str| format!("{prefix}{k}");
    let scale = params::opt_f64(p, &key("scale"), 1.0e-6, 1.0e6)?.or(params::opt_f64(p, "scale", 1.0e-6, 1.0e6)?);
    let rotate = params::opt_f64(p, &key("rotate"), -3600.0, 3600.0)?.unwrap_or(0.0);
    let mirror = params::opt_bool(p, &key("mirror"))?.unwrap_or(false);
    let paths = shape.transformed(scale.unwrap_or(1.0), rotate, mirror);
    let (lo, hi) = bounds(&paths);
    let anchor = params::opt_str(p, &key("anchor"))?.or(params::opt_str(p, "anchor")?).unwrap_or("centre");
    let (u0, u1) = cx.stock.u_range(b);
    let (h0, h1) = (cx.stock.bottom(), cx.stock.top());
    let (from, default_to) = match anchor {
        "centre" => ((lo + hi) * 0.5, DVec2::new((u0 + u1) / 2.0, (h0 + h1) / 2.0)),
        "bottom_centre" => (DVec2::new((lo.x + hi.x) / 2.0, lo.y), DVec2::new((u0 + u1) / 2.0, h0)),
        "bottom_left" => (lo, DVec2::new(u0, h0)),
        "origin" => (DVec2::ZERO, DVec2::new((u0 + u1) / 2.0, h0)),
        // The shape is already in the cut plane (outlines taken from a model).
        "absolute" => (DVec2::ZERO, DVec2::ZERO),
        other => return Err(EngineError::InvalidParams(format!("`anchor` must be one of {ANCHORS:?}, not {other}"))),
    };
    // A view's own place, else the place both views share, else the stock's.
    let at = |k: &str, default: f64| -> EngineResult<f64> {
        Ok(params::opt_f64(p, &key(k), -1.0e6, 1.0e6)?.or(params::opt_f64(p, k, -1.0e6, 1.0e6)?).unwrap_or(default))
    };
    let to = DVec2::new(at("x", default_to.x)?, at("y", default_to.y)?);
    Ok(Placing { scale: scale.unwrap_or(1.0), turn: rotate.to_radians().sin_cos(), mirror, shift: to - from })
}

/// The curves (by id) of a profile cut's shape, where they lie in the cut plane at its first
/// angle, and that angle: for picking the lines it leaves out. Empty if the shape is not a sketch.
pub fn profile_lines(cx: &Context<'_>, p: &Value) -> EngineResult<(f64, Curves)> {
    let shape = cx.shape(p, "shape")?;
    let b = angle(p, "angle", 0.0)?;
    let at = placing(cx, shape, p, b, "")?;
    let curves = shape.sketch.as_ref().map(|s| s.curves()).unwrap_or_default();
    Ok((b, curves.into_iter().map(|(id, pts)| (id, pts.into_iter().map(|q| at.point(q)).collect())).collect()))
}

/// Curves of a sketch by id, each as points along it.
pub type Curves = Vec<(u32, Vec<DVec2>)>;

/// The curve ids a profile cut leaves out.
fn skipped(p: &Value) -> Vec<u32> {
    p.get("skip")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_u64).filter_map(|v| u32::try_from(v).ok()).collect())
        .unwrap_or_default()
}

fn bounds(paths: &[Path2]) -> (DVec2, DVec2) {
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    for q in paths.iter().flat_map(|p| &p.points) {
        lo = lo.min(*q);
        hi = hi.max(*q);
    }
    (lo, hi)
}

/// One profile pass at table angle `b`: every outline of `paths`, holes first, each entered from
/// `lead` and left the same way, starting from (and ending at) the wire's waiting spot.
#[allow(clippy::too_many_arguments)]
fn profile_pass(
    cx: &Context<'_>,
    c: &Common,
    paths: &[Path2],
    b: f64,
    lead: &str,
    keep_outside: bool,
    direction: &str,
    attach: Option<f64>,
    start: Option<DVec2>,
    middle: Option<DVec2>,
    out: &mut Built,
) -> EngineResult<()> {
    let loops: Vec<Vec<DVec2>> =
        paths.iter().filter(|q| q.closed && q.points.len() >= 3).map(|q| q.points.clone()).collect();
    let open: Vec<&Path2> = paths.iter().filter(|q| !q.closed || q.points.len() < 3).collect();
    if loops.is_empty() && open.is_empty() {
        return Err(EngineError::InvalidParams("the shape has no outline to cut".into()));
    }
    let regions = nest(&loops);
    // The wire runs half a kerf off the part, on the waste side.
    let wire_offset = if c.compensate { c.kerf / 2.0 } else { 0.0 };
    let grow = if keep_outside { -wire_offset } else { wire_offset };
    let wire_regions = region::offset(&regions, grow, TOL);
    if !loops.is_empty() && wire_regions.is_empty() {
        return Err(EngineError::InvalidParams("the kerf compensation leaves nothing of the shape".into()));
    }
    // The part edge is half a kerf in from the wire.
    let part = region::offset(&wire_regions, -grow.signum() * c.kerf / 2.0, TOL);
    // What the closed outlines keep (open ones keep their own, below).
    let keep_index = (!loops.is_empty()).then(|| {
        out.keeps.push(Keep {
            b,
            regions: if c.compensate { regions } else { part },
            outside: keep_outside,
            at: 0,
            sweep: None,
        });
        out.keeps.len() - 1
    });

    let (u0, u1) = cx.stock.u_range(b);
    let (_, h1) = (cx.stock.bottom(), cx.stock.top());
    // A part left attached keeps the foam under it, below where the wire comes in level.
    let attach = attach.filter(|_| !keep_outside);
    let mut lowest_level: Option<f64> = None;
    let mut releases: Vec<WirePath> = Vec::new();
    // Holes before the outlines around them, so the part stays put while its insides are cut.
    let mut ordered: Vec<(usize, Vec<DVec2>)> = Vec::new();
    // The wire goes no lower than it can reach (machine Y0); below that it runs level.
    let floor = cx.machine.wire_y0;
    let mut sunk = 0.0_f64;
    for shape in &wire_regions {
        for (depth, l) in shape.iter().enumerate() {
            sunk = sunk.max(l.iter().map(|q| floor - q.y).fold(0.0, f64::max));
            ordered.push((usize::from(depth == 0), above(l, floor)));
        }
    }
    if sunk > c.kerf + 1.0e-6 {
        out.warnings.push(format!(
            "the wire reaches no lower than Y0, so it runs level there, {sunk:.1} mm above the bottom of the outline"
        ));
    }
    let lead_key = |l: &Vec<DVec2>| match lead {
        "right" => -l.iter().map(|q| q.x).fold(f64::NEG_INFINITY, f64::max),
        "top" => -l.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max),
        _ => l.iter().map(|q| q.x).fold(f64::INFINITY, f64::min),
    };
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(lead_key(&a.1).total_cmp(&lead_key(&b.1))));
    let mut waits = 0usize;
    for (depth, mut l) in ordered {
        // An outline left attached: in level at its bottom, over its top, out level the other side.
        if depth == 1
            && let Some(up) = attach
        {
            let lo = l.iter().map(|q| q.y).fold(f64::INFINITY, f64::min);
            let hi = l.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max);
            let level = (lo + up).min(hi - 1.0e-3) + 1.0e-6;
            if let Some((left, right, mut upper, mut lower)) = split_at_level(&l, level) {
                // A start point picks the side it comes in from.
                let middle = l.iter().map(|q| q.x).sum::<f64>() / l.len().max(1) as f64;
                let from_right = start.map_or(lead == "right", |st| st.x > middle);
                if from_right {
                    upper.reverse();
                    lower.reverse();
                }
                let (first, last) = if from_right { (right, left) } else { (left, right) };
                let outside = |p: DVec2, right_side: bool| {
                    if right_side {
                        DVec2::new(u1.max(p.x) + c.clearance, level)
                    } else {
                        DVec2::new(u0.min(p.x) - c.clearance, level)
                    }
                };
                let (s, e) = (outside(first, from_right), outside(last, !from_right));
                travel(cx, c, &mut out.path, WirePos::new(s.x, s.y, b), "left");
                out.path.push(MoveKind::Lead, WirePos::new(first.x, first.y, b), c.feed);
                for q in upper.iter().skip(1) {
                    out.path.push(MoveKind::Cut, WirePos::new(q.x, q.y, b), c.feed);
                }
                out.path.push(MoveKind::Lead, WirePos::new(e.x, e.y, b), c.feed);
                // The release follows the bottom through the same two kerfs.
                let mut rel = WirePath::new(WirePos::new(s.x, s.y, b));
                rel.push(MoveKind::Lead, WirePos::new(first.x, first.y, b), c.feed);
                for q in lower.iter().skip(1) {
                    rel.push(MoveKind::Cut, WirePos::new(q.x, q.y, b), c.feed);
                }
                rel.push(MoveKind::Lead, WirePos::new(e.x, e.y, b), c.feed);
                releases.push(rel);
                lowest_level = Some(lowest_level.map_or(level, |v: f64| v.min(level)));
                waits += 1;
                continue;
            }
        }
        let ccw = signed_area2(&l) > 0.0;
        if ccw != (direction != "cw") {
            l.reverse();
        }
        // The entry: the loop's point nearest the side it is entered from.
        // A start point: in at the outline's point nearest it, from the nearest side that gets
        // there without crossing the part.
        let (entry, lead) = match start.filter(|_| depth == 1) {
            Some(st) => {
                let entry = argmax(&l, |q| -q.distance(st));
                let e = l.get(entry).copied().unwrap_or_default();
                let mut sides = [
                    ("left", DVec2::new(u0.min(e.x) - c.clearance, e.y)),
                    ("right", DVec2::new(u1.max(e.x) + c.clearance, e.y)),
                    ("top", DVec2::new(e.x, h1.max(e.y) + c.clearance)),
                ];
                sides.sort_by(|a, b| a.1.distance(e).total_cmp(&b.1.distance(e)));
                let clear = sides.iter().find(|(_, out)| !crosses_outline(&l, entry, e, *out));
                (entry, clear.or(sides.first()).map_or(lead, |(side, _)| *side))
            }
            None => (
                match lead {
                    "right" => argmax(&l, |q| q.x),
                    "top" => argmax(&l, |q| q.y),
                    _ => argmax(&l, |q| -q.x),
                },
                lead,
            ),
        };
        let e = l.get(entry).copied().unwrap_or_default();
        let s = match lead {
            "right" => DVec2::new(u1.max(e.x) + c.clearance, e.y),
            "top" => DVec2::new(e.x, h1.max(e.y) + c.clearance),
            _ => DVec2::new(u0.min(e.x) - c.clearance, e.y),
        };
        travel(cx, c, &mut out.path, WirePos::new(s.x, s.y, b), lead);
        out.path.push(MoveKind::Lead, WirePos::new(e.x, e.y, b), c.feed);
        for k in 1..=l.len() {
            let q = l.get((entry + k) % l.len()).copied().unwrap_or(e);
            out.path.push(MoveKind::Cut, WirePos::new(q.x, q.y, b), c.feed);
        }
        out.path.push(MoveKind::Lead, WirePos::new(s.x, s.y, b), c.feed);
        waits += 1;
    }
    // Open outlines (or outlines with lines left out): cut along them as they are, from the end
    // nearest the start point (else nearest the wire). An end inside the foam is reached level
    // from the nearest side (or down from the top) that does not cross the outline, and left the
    // same way.
    let (lo_h, hi_h) = (cx.stock.bottom(), h1);
    let inside = |q: DVec2| q.x > u0 + 1.0e-6 && q.x < u1 - 1.0e-6 && q.y < hi_h - 1.0e-6 && q.y > lo_h - 1.0e-6;
    let way_out = |pts: &[DVec2], at: usize| -> Option<(DVec2, &'static str)> {
        let q = *pts.get(at)?;
        let mut sides = [
            ("left", DVec2::new(u0.min(q.x) - c.clearance, q.y)),
            ("right", DVec2::new(u1.max(q.x) + c.clearance, q.y)),
            ("top", DVec2::new(q.x, hi_h.max(q.y) + c.clearance)),
        ];
        sides.sort_by(|a, b| a.1.distance(q).total_cmp(&b.1.distance(q)));
        let clear = sides.iter().find(|(_, out)| !crosses_chain(pts, at, q, *out));
        clear.or(sides.first()).map(|(side, out)| (*out, *side))
    };
    for q in open {
        let mut pts = q.points.clone();
        let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) else { continue };
        let at = out.path.end();
        let from = start.unwrap_or(at.uh());
        if last.distance_squared(from) < first.distance_squared(from) {
            pts.reverse();
        }
        let n = pts.len();
        let (Some(a), Some(z)) = (pts.first().copied(), pts.last().copied()) else { continue };
        let entry = if inside(a) { way_out(&pts, 0) } else { None };
        let exit = if inside(z) { way_out(&pts, n - 1) } else { None };
        match entry {
            Some((s, side)) => {
                travel(cx, c, &mut out.path, WirePos::new(s.x, s.y, b), side);
                out.path.push(MoveKind::Lead, WirePos::new(a.x, a.y, b), c.feed);
            }
            None => travel(cx, c, &mut out.path, WirePos::new(a.x, a.y, b), lead),
        }
        for pt in pts.iter().skip(1) {
            out.path.push(MoveKind::Cut, WirePos::new(pt.x, pt.y, b), c.feed);
        }
        if let Some((e, _)) = exit {
            out.path.push(MoveKind::Lead, WirePos::new(e.x, e.y, b), c.feed);
        }
        // What it cuts away: the side of the outline (closed round the outside of the stock) that
        // does not hold the part.
        let s = entry.map_or(a, |(s, _)| s);
        let e = exit.map_or(z, |(e, _)| e);
        let mut chain = vec![s];
        chain.extend(pts.iter().copied());
        chain.push(e);
        let margin = c.clearance * 2.0 + 10.0;
        let frame = (DVec2::new(u0 - margin, lo_h - margin), DVec2::new(u1 + margin, hi_h + margin));
        if let Some(region) = middle.and_then(|inner| enclosed(&chain, frame, inner)) {
            out.keeps.push(Keep { b, regions: vec![region], outside: false, at: out.path.moves.len(), sweep: None });
        } else {
            out.warnings
                .push("an open outline that does not run from outside the foam back to it cuts nothing free".into());
        }
        waits += 1;
    }
    if waits == 0 {
        return Err(EngineError::InvalidParams("nothing to cut".into()));
    }
    let done = out.path.moves.len();
    if let Some(k) = keep_index.and_then(|i| out.keeps.get_mut(i)) {
        k.at = done;
        // The foam under the attached part stays, below the level the wire came in at.
        if let Some(level) = lowest_level {
            let under = vec![region::rectangle(DVec2::new(-FAR, -FAR), DVec2::new(FAR, level))];
            k.regions = region::boolean(&k.regions, &under, region::Op::Union);
        }
    }
    // The last pass that holds the part is the one that frees it.
    if let Some(first) = releases.first() {
        let mut all = WirePath::new(first.start);
        for rel in &releases {
            travel(cx, c, &mut all, rel.start, "left");
            all.extend(rel);
        }
        out.release = Some(all);
    }
    Ok(())
}

/// A closed loop with everything below `floor` brought up to it: where an edge crosses the floor
/// the loop meets it there, and runs along it until it comes back up.
fn above(l: &[DVec2], floor: f64) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(l.len() + 4);
    for (a, b) in l.iter().zip(l.iter().cycle().skip(1)) {
        out.push(DVec2::new(a.x, a.y.max(floor)));
        if (a.y - floor) * (b.y - floor) < 0.0 {
            out.push(a.lerp(*b, (floor - a.y) / (b.y - a.y)));
        }
    }
    out.dedup_by(|a, b| a.distance_squared(*b) < 1.0e-18);
    // Points along the floor between two others on it add nothing.
    let on = |q: &DVec2| (q.y - floor).abs() < 1.0e-9;
    let n = out.len();
    let keep: Vec<bool> = (0..n)
        .map(|i| {
            let prev = out.get((i + n - 1) % n);
            let next = out.get((i + 1) % n);
            !(out.get(i).is_some_and(on) && prev.is_some_and(on) && next.is_some_and(on))
        })
        .collect();
    let mut kept: Vec<DVec2> = out.into_iter().zip(keep).filter_map(|(q, k)| k.then_some(q)).collect();
    if kept.len() < 3 {
        kept = l.to_vec();
    }
    kept
}

/// Splits a closed loop where it crosses the level `h` farthest left and farthest right: those
/// two points, the part of the loop over its top and the part under it, both from left to right.
fn split_at_level(l: &[DVec2], h: f64) -> Option<(DVec2, DVec2, Vec<DVec2>, Vec<DVec2>)> {
    let n = l.len();
    let mut crossings: Vec<(DVec2, usize)> = Vec::new();
    for i in 0..n {
        let (a, b) = (*l.get(i)?, *l.get((i + 1) % n)?);
        if (a.y >= h) != (b.y >= h) {
            let t = (h - a.y) / (b.y - a.y);
            crossings.push((a + (b - a) * t, i));
        }
    }
    let left = *crossings.iter().min_by(|p, q| p.0.x.total_cmp(&q.0.x))?;
    let right = *crossings.iter().max_by(|p, q| p.0.x.total_cmp(&q.0.x))?;
    if left.1 == right.1 {
        return None;
    }
    // From one crossing forward round the loop to the other.
    let walk = |from: (DVec2, usize), to: (DVec2, usize)| -> Vec<DVec2> {
        let mut v = vec![from.0];
        let mut i = (from.1 + 1) % n;
        for _ in 0..n {
            v.push(l.get(i).copied().unwrap_or_default());
            if i == to.1 {
                break;
            }
            i = (i + 1) % n;
        }
        v.push(to.0);
        v
    };
    let a = walk(left, right);
    let mut b = walk(right, left);
    b.reverse();
    let top = |v: &[DVec2]| v.iter().map(|q| q.y).fold(f64::NEG_INFINITY, f64::max);
    Some(if top(&a) >= top(&b) { (left.0, right.0, a, b) } else { (left.0, right.0, b, a) })
}

/// How a profile or two-view cut treats the part's bottom: Some(height it stays joined up to)
/// when attached, None when cut right through.
fn bottom(p: &Value) -> EngineResult<Option<f64>> {
    Ok(match choice(p, "bottom", BOTTOMS)? {
        "through" => None,
        _ => Some(params::opt_f64(p, "attach", 0.0, 10_000.0)?.unwrap_or(0.0)),
    })
}

/// The side a pass enters from and the way round it goes: the other side and the other way
/// when `flip` (alternating passes), so each pass starts where the last one finished.
fn pass_way<'a>(lead: &'a str, direction: &'a str, flip: bool) -> (&'a str, &'a str) {
    if !flip {
        return (lead, direction);
    }
    let lead = match lead {
        "left" => "right",
        "right" => "left",
        other => other,
    };
    (lead, if direction == "cw" { "ccw" } else { "cw" })
}

fn wants_release(p: &Value) -> EngineResult<bool> {
    Ok(params::opt_bool(p, "release")?.unwrap_or(true))
}

/// Whether the segment from `a` (the outline's point `at`) to `b` crosses the outline anywhere
/// but at that point.
fn crosses_outline(l: &[DVec2], at: usize, a: DVec2, b: DVec2) -> bool {
    let n = l.len();
    (0..n).any(|i| {
        let j = (i + 1) % n;
        if i == at || j == at {
            return false;
        }
        let (Some(&p), Some(&q)) = (l.get(i), l.get(j)) else { return false };
        let d1 = (b - a).perp_dot(p - a);
        let d2 = (b - a).perp_dot(q - a);
        let d3 = (q - p).perp_dot(a - p);
        let d4 = (q - p).perp_dot(b - p);
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    })
}

/// Whether `a`–`b` (leaving the chain's point `at`) crosses the chain anywhere else.
fn crosses_chain(pts: &[DVec2], at: usize, a: DVec2, b: DVec2) -> bool {
    pts.windows(2).enumerate().any(|(i, w)| {
        if i == at || i + 1 == at {
            return false;
        }
        let (Some(&p), Some(&q)) = (w.first(), w.get(1)) else { return false };
        let d1 = (b - a).perp_dot(p - a);
        let d2 = (b - a).perp_dot(q - a);
        let d3 = (q - p).perp_dot(a - p);
        let d4 = (q - p).perp_dot(b - p);
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    })
}

/// The region `chain` (from outside the stock, through it, back outside) closes off on the side
/// holding `inner`: the chain, then round the edge of `frame` one way or the other. None if the
/// ends are not out on the frame's sides or neither way holds `inner`.
fn enclosed(chain: &[DVec2], frame: (DVec2, DVec2), inner: DVec2) -> Option<Region> {
    let (lo, hi) = frame;
    let (s, e) = (*chain.first()?, *chain.last()?);
    // Out to the frame from an end: straight away from the stock on its side.
    let to_frame = |p: DVec2| -> Option<DVec2> {
        let mid = (lo + hi) / 2.0;
        let inside_x = p.x > lo.x && p.x < hi.x;
        if p.y >= hi.y - (hi.y - lo.y) * 0.25 && inside_x && p.y > mid.y {
            Some(DVec2::new(p.x, hi.y))
        } else if p.x <= mid.x {
            Some(DVec2::new(lo.x, p.y))
        } else {
            Some(DVec2::new(hi.x, p.y))
        }
    };
    let (fs, fe) = (to_frame(s)?, to_frame(e)?);
    // Distance round the frame's edge, anticlockwise from its bottom-left corner.
    let (w, h) = (hi.x - lo.x, hi.y - lo.y);
    let around = |p: DVec2| -> f64 {
        if (p.y - lo.y).abs() < 1.0e-9 {
            p.x - lo.x
        } else if (p.x - hi.x).abs() < 1.0e-9 {
            w + (p.y - lo.y)
        } else if (p.y - hi.y).abs() < 1.0e-9 {
            w + h + (hi.x - p.x)
        } else {
            2.0 * w + h + (hi.y - p.y)
        }
    };
    let corners =
        [(w, DVec2::new(hi.x, lo.y)), (w + h, hi), (2.0 * w + h, DVec2::new(lo.x, hi.y)), (2.0 * (w + h), lo)];
    let perimeter = 2.0 * (w + h);
    // From the end's frame point back to the start's, one way round or the other.
    let walk = |forward: bool| -> Vec<DVec2> {
        let (t0, t1) = (around(fe), around(fs));
        let span = if forward { (t1 - t0).rem_euclid(perimeter) } else { (t0 - t1).rem_euclid(perimeter) };
        let mut along: Vec<(f64, DVec2)> = corners
            .iter()
            .flat_map(|&(t, c)| [(t, c), (t + perimeter, c)])
            .filter_map(|(t, c)| {
                let d = if forward { t - t0 } else { t0 - t };
                let d = d.rem_euclid(perimeter);
                (d > 1.0e-9 && d < span - 1.0e-9).then_some((d, c))
            })
            .collect();
        along.sort_by(|a, b| a.0.total_cmp(&b.0));
        along.dedup_by(|a, b| a.1.distance(b.1) < 1.0e-9);
        let mut out = chain.to_vec();
        out.push(fe);
        out.extend(along.into_iter().map(|(_, c)| c));
        out.push(fs);
        out
    };
    [true, false].into_iter().map(walk).find(|ring| region::contains(&[vec![ring.clone()]], inner)).and_then(|ring| {
        region::union_polygons(&[ring]).into_iter().find(|r| region::contains(std::slice::from_ref(r), inner))
    })
}

fn argmax(pts: &[DVec2], f: impl Fn(DVec2) -> f64) -> usize {
    let mut best = (0usize, f64::NEG_INFINITY);
    for (i, q) in pts.iter().enumerate() {
        let v = f(*q);
        if v > best.1 + 1.0e-9 {
            best = (i, v);
        }
    }
    best.0
}

/// The way from `from` to `to` that stays beside the foam, when both are clear of the stock on
/// the same side: straight there, or, when the table turns, out to `reach` (past the stock's
/// turning circle), the turn there, and straight on. None when they are on different sides or
/// either is over or inside the stock; then the wire has to go over the top.
pub fn beside(stock: &Stock, from: WirePos, to: WirePos, reach: f64) -> Option<Vec<(MoveKind, WirePos)>> {
    let (u0, u1) = stock.u_range(from.b);
    let (v0, v1) = stock.u_range(to.b);
    let left = from.u <= u0 - 1.0e-6 && to.u <= v0 - 1.0e-6;
    let right = from.u >= u1 + 1.0e-6 && to.u >= v1 + 1.0e-6;
    if !(left || right) {
        return None;
    }
    let mut out = Vec::new();
    if (from.b - to.b).abs() > 1.0e-9 {
        let out_u = if left { from.u.min(-reach) } else { from.u.max(reach) };
        if (out_u - from.u).abs() > 1.0e-9 {
            out.push((MoveKind::Rapid, WirePos::new(out_u, from.h, from.b)));
        }
        out.push((MoveKind::Rotate, WirePos::new(out_u, from.h, to.b)));
    }
    out.push((MoveKind::Rapid, to));
    Some(out)
}

/// Moves the wire from where the path ends to `to` without going through foam. Beside the stock
/// on the side `to` is entered from, it stays there (turning the table past the stock's reach);
/// otherwise it goes out to the side clear of the stock's reach, up over it, turning the table
/// there if the angle changes, and down the side `to` is entered from.
pub(crate) fn travel(cx: &Context<'_>, c: &Common, path: &mut WirePath, to: WirePos, lead: &str) {
    let from = path.end();
    if from.uh().distance_squared(to.uh()) < 1.0e-12 && (from.b - to.b).abs() < 1.0e-9 {
        return;
    }
    let reach = cx.stock.reach() + c.clearance;
    if lead != "top"
        && let Some(moves) = beside(cx.stock, from, to, reach)
    {
        for (kind, at) in moves {
            path.push(kind, at, 0.0);
        }
        return;
    }
    let safe = cx.stock.top() + c.clearance;
    let (u0, u1) = cx.stock.u_range(from.b);
    let clear_of = |p: &WirePos, u0: f64, u1: f64| p.u <= u0 - 1.0e-6 || p.u >= u1 + 1.0e-6 || p.h >= safe - 1.0e-6;
    // Same angle and both clear on the same side: straight across.
    if (from.b - to.b).abs() < 1.0e-9 && from.h >= safe - 1.0e-6 && to.h >= safe - 1.0e-6 {
        path.push(MoveKind::Rapid, to, 0.0);
        return;
    }
    let side = |p: &WirePos, u0: f64, u1: f64| if p.u <= (u0 + u1) / 2.0 { -1.0 } else { 1.0 };
    // Up: out to the side first (unless already above), then up to the safe height.
    if from.h < safe - 1.0e-6 {
        if clear_of(&from, u0, u1) {
            let s = side(&from, u0, u1);
            let out_u = if s < 0.0 { from.u.min(-reach) } else { from.u.max(reach) };
            path.push(MoveKind::Rapid, WirePos::new(out_u, from.h, from.b), 0.0);
            path.push(MoveKind::Rapid, WirePos::new(out_u, safe, from.b), 0.0);
        } else {
            // Inside the stock's shadow: only straight up is clear (in its own kerf).
            path.push(MoveKind::Rapid, WirePos::new(from.u, safe, from.b), 0.0);
        }
    }
    let up = path.end();
    if (up.b - to.b).abs() > 1.0e-9 {
        // The table turns with the wire above the foam (it is not beside it on that side).
        path.push(MoveKind::Rotate, WirePos::new(up.u, up.h, to.b), 0.0);
    }
    let (v0, v1) = cx.stock.u_range(to.b);
    if to.h >= safe - 1.0e-6 || lead == "top" || !(to.u <= v0 - 1.0e-6 || to.u >= v1 + 1.0e-6) {
        path.push(MoveKind::Rapid, WirePos::new(to.u, safe.max(to.h), to.b), 0.0);
        path.push(MoveKind::Rapid, to, 0.0);
        return;
    }
    let s = side(&to, v0, v1);
    let down_u = if s < 0.0 { to.u.min(-reach) } else { to.u.max(reach) };
    path.push(MoveKind::Rapid, WirePos::new(down_u, safe, to.b), 0.0);
    path.push(MoveKind::Rapid, WirePos::new(down_u, to.h, to.b), 0.0);
    path.push(MoveKind::Rapid, to, 0.0);
}

/// Where every cut starts and ends: left of the stock's reach, above it, the table at `b`.
pub fn park(machine: &Machine, stock: &Stock, b: f64) -> WirePos {
    WirePos::new(-(stock.reach() + machine.clearance), stock.top() + machine.clearance, b)
}

/// Ends a cut out of the foam: where the last pass left the wire when that is beside the stock
/// (the next cut, or the way home, goes on from there), else parked above it on the left.
pub(crate) fn finish(cx: &Context<'_>, c: &Common, out: &mut Built) {
    let end = out.path.end();
    let (u0, u1) = cx.stock.u_range(end.b);
    if end.u <= u0 - 1.0e-6 || end.u >= u1 + 1.0e-6 {
        return;
    }
    let b = end.b;
    let park = WirePos::new(-(cx.stock.reach() + c.clearance), cx.stock.top() + c.clearance, b);
    travel(cx, c, &mut out.path, park, "left");
}

fn angle(p: &Value, key: &str, default: f64) -> EngineResult<f64> {
    Ok(params::opt_f64(p, key, -36_000.0, 36_000.0)?.unwrap_or(default))
}

fn choice<'a>(p: &'a Value, key: &str, choices: &[&'a str]) -> EngineResult<&'a str> {
    let first = choices.first().copied().unwrap_or_default();
    match params::opt_str(p, key)? {
        None => Ok(first),
        Some(v) if choices.contains(&v) => Ok(v),
        Some(v) => Err(EngineError::InvalidParams(format!("`{key}` must be one of {choices:?}, not {v}"))),
    }
}

pub(crate) fn needs_b(cx: &Context<'_>, angles: &[f64]) -> EngineResult<()> {
    if !cx.machine.b_axis && angles.iter().any(|a| a.abs() > 1.0e-9) {
        return Err(EngineError::InvalidParams("the machine has no turntable; cut at 0°".into()));
    }
    Ok(())
}

fn profile(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = common(cx, p)?;
    let shape = cx.shape(p, "shape")?;
    let b0 = angle(p, "angle", 0.0)?;
    let copies = params::opt_f64(p, "copies", 1.0, 360.0)?.unwrap_or(1.0) as usize;
    let step = angle(p, "step", if copies > 1 { 360.0 / copies as f64 } else { 0.0 })?;
    let lead = choice(p, "lead", LEADS)?;
    let keep_outside = choice(p, "keep", KEEPS)? == "outside";
    let direction = choice(p, "direction", DIRECTIONS)?;
    let angles: Vec<f64> = (0..copies.max(1)).map(|k| b0 + step * k as f64).collect();
    let attach = bottom(p)?;
    let alternate = params::opt_bool(p, "alternate")?.unwrap_or(true);
    let start = match (params::opt_f64(p, "start_x", -1.0e5, 1.0e5)?, params::opt_f64(p, "start_y", -1.0e5, 1.0e5)?) {
        (Some(x), Some(y)) => Some(DVec2::new(x, y)),
        _ => None,
    };
    needs_b(cx, &angles)?;
    let mut out = Built { path: WirePath::new(park(cx.machine, cx.stock, b0)), ..Built::default() };
    let skip = skipped(p);
    for (k, b) in angles.into_iter().enumerate() {
        let full = place(cx, shape, p, b, "")?;
        // Lines left out: the rest of the outline, where it was. The middle of the whole shape
        // says which side of what is left the part is on.
        let paths = match shape.sketch.as_deref().filter(|_| !skip.is_empty()) {
            Some(sketch) => placing(cx, shape, p, b, "")?.paths(&sketch.paths_without(&skip)),
            None => full.clone(),
        };
        let pts: Vec<DVec2> = full.iter().filter(|q| q.closed).flat_map(|q| q.points.iter().copied()).collect();
        let part = (!pts.is_empty()).then(|| pts.iter().copied().sum::<DVec2>() / pts.len() as f64);
        let (lead, direction) = pass_way(lead, direction, alternate && k % 2 == 1);
        profile_pass(cx, &c, &paths, b, lead, keep_outside, direction, attach, start, part, &mut out)?;
    }
    if !wants_release(p)? {
        out.release = None;
    }
    finish(cx, &c, &mut out);
    check_fits(cx, &mut out);
    Ok(out)
}

fn two_view(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = common(cx, p)?;
    let front = cx.shape(p, "front")?;
    let side = cx.shape(p, "side")?;
    let b0 = angle(p, "angle", 0.0)?;
    let lead = choice(p, "lead", LEADS)?;
    let direction = choice(p, "direction", DIRECTIONS)?;
    needs_b(cx, &[b0, b0 + 90.0])?;
    let mut out = Built { path: WirePath::new(park(cx.machine, cx.stock, b0)), ..Built::default() };
    let front_paths = place(cx, front, p, b0, "front_")?;
    let attach = bottom(p)?;
    let alternate = params::opt_bool(p, "alternate")?.unwrap_or(true);
    profile_pass(cx, &c, &front_paths, b0, lead, false, direction, attach, None, None, &mut out)?;
    let side_paths = place(cx, side, p, b0 + 90.0, "side_")?;
    let (lead, direction) = pass_way(lead, direction, alternate);
    profile_pass(cx, &c, &side_paths, b0 + 90.0, lead, false, direction, attach, None, None, &mut out)?;
    if !wants_release(p)? {
        out.release = None;
    }
    finish(cx, &c, &mut out);
    check_fits(cx, &mut out);
    Ok(out)
}

/// The half profile of a lathe shape: (radius, height) points from the top down, radius
/// measured from the shape's axis.
fn half_profile(shape: &[Path2], axis_x: f64) -> EngineResult<Vec<DVec2>> {
    let main = shape
        .iter()
        .max_by(|a, b| a.length().total_cmp(&b.length()))
        .ok_or_else(|| EngineError::InvalidParams("the shape has no outline".into()))?;
    let pts = &main.points;
    let mut half: Vec<DVec2> = if main.closed {
        // The side of the loop between its top and bottom that lies farther out.
        let top = argmax(pts, |q| q.y);
        let bottom = argmax(pts, |q| -q.y);
        let n = pts.len();
        let walk = |from: usize, to: usize| -> Vec<DVec2> {
            let mut v = Vec::new();
            let mut i = from;
            for _ in 0..=n {
                v.push(pts.get(i).copied().unwrap_or_default());
                if i == to {
                    break;
                }
                i = (i + 1) % n;
            }
            v
        };
        let a = walk(top, bottom);
        let mut b = walk(bottom, top);
        b.reverse();
        let mean = |v: &[DVec2]| v.iter().map(|q| q.x).sum::<f64>() / v.len().max(1) as f64;
        if mean(&a) >= mean(&b) { a } else { b }
    } else {
        let mut v = pts.clone();
        if v.first().map(|q| q.y) < v.last().map(|q| q.y) {
            v.reverse();
        }
        v
    };
    for q in &mut half {
        *q = DVec2::new((q.x - axis_x).max(0.0), q.y);
    }
    half.dedup_by(|a, b| a.distance_squared(*b) < 1.0e-12);
    if half.len() < 2 {
        return Err(EngineError::InvalidParams("the lathe profile needs at least two points".into()));
    }
    // Heights must not climb going down the profile.
    if half.windows(2).any(|w| w[1].y > w[0].y + 1.0e-6) {
        return Err(EngineError::InvalidParams(
            "a lathe profile must run steadily downwards (no overhangs the wire cannot reach)".into(),
        ));
    }
    Ok(half)
}

pub const LATHE_AXES: &[&str] = &["shape_centre", "shape_origin"];

fn lathe(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = common(cx, p)?;
    if !cx.machine.b_axis {
        return Err(EngineError::InvalidParams("lathe cuts need a turntable".into()));
    }
    let shape = cx.shape(p, "shape")?;
    let scale = params::opt_f64(p, "scale", 1.0e-6, 1.0e6)?.unwrap_or(1.0);
    let paths = shape.transformed(scale, 0.0, false);
    let (lo, hi) = bounds(&paths);
    let axis_x = match choice(p, "axis", LATHE_AXES)? {
        "shape_origin" => 0.0,
        _ => (lo.x + hi.x) / 2.0,
    };
    let lift = params::opt_f64(p, "y", -1.0e6, 1.0e6)?.unwrap_or(cx.stock.bottom()) - lo.y;
    let mut half = half_profile(&paths, axis_x)?;
    for q in &mut half {
        q.y += lift;
    }
    let step = params::opt_f64(p, "step", 0.1, 180.0)?.unwrap_or(5.0);
    let start = angle(p, "angle", 0.0)?;
    let sweep = params::opt_f64(p, "sweep", 1.0, 360.0)?.unwrap_or(360.0);
    let passes = ((sweep / step).round() as usize).clamp(1, 3600);
    let wire_offset = if c.compensate { c.kerf / 2.0 } else { 0.0 };
    let top = cx.stock.top() + c.clearance;
    let reach = cx.stock.reach() + c.clearance;
    let mut warnings = Vec::new();
    let (hb, ht) = (half.last().map_or(0.0, |q| q.y), half.first().map_or(0.0, |q| q.y));
    if hb < cx.stock.bottom() - 1.0e-6 {
        warnings.push("the profile goes below the stock's bottom".into());
    }
    let mut out = Built { path: WirePath::new(park(cx.machine, cx.stock, start)), warnings, ..Built::default() };
    let r_top = half.first().map_or(0.0, |q| q.x) + wire_offset;
    for k in 0..passes {
        let b = start + sweep * k as f64 / passes as f64;
        // Down from above the foam onto the top of the profile, down it, out at the bottom.
        travel(cx, &c, &mut out.path, WirePos::new(r_top, top, b), "top");
        // What the pass cuts away: everything beyond the part's edge (half a kerf in from the
        // wire), above the profile's bottom.
        let edge = wire_offset - c.kerf / 2.0;
        let mut cap = vec![DVec2::new(r_top + edge - wire_offset, FAR)];
        for q in &half {
            let w = WirePos::new(q.x + wire_offset, q.y, b);
            out.path.push(MoveKind::Cut, w, c.feed);
            cap.push(DVec2::new(q.x + edge, q.y));
        }
        out.path.push(MoveKind::Lead, WirePos::new(reach, hb, b), c.feed);
        cap.push(DVec2::new(FAR, hb));
        cap.push(DVec2::new(FAR, FAR));
        out.keeps.push(Keep {
            b,
            regions: vec![vec![close(cap)]],
            outside: true,
            at: out.path.moves.len(),
            sweep: None,
        });
        // Back up the outside to above the foam.
        out.path.push(MoveKind::Rapid, WirePos::new(reach, top, b), 0.0);
    }
    if ht > cx.stock.top() + 1.0e-6 {
        out.warnings.push("the profile is taller than the stock".into());
    }
    // The turned part stands on the foam under its foot until a level cut at the end frees it.
    if wants_release(p)? {
        let b = out.path.end().b;
        let mut rel = WirePath::new(WirePos::new(-reach, hb, b));
        rel.push(MoveKind::Cut, WirePos::new(reach, hb, b), c.feed);
        out.release = Some(rel);
    }
    finish(cx, &c, &mut out);
    check_fits(cx, &mut out);
    Ok(out)
}

/// A loop as a region contour (counter-clockwise).
fn close(mut l: Vec<DVec2>) -> Vec<DVec2> {
    if signed_area2(&l) < 0.0 {
        l.reverse();
    }
    l
}

fn slab(cx: &Context<'_>, p: &Value) -> EngineResult<Built> {
    let c = common(cx, p)?;
    let thickness = params::opt_f64(p, "thickness", 0.1, 1.0e5)?
        .ok_or_else(|| EngineError::InvalidParams("`thickness` is required".into()))?;
    let skim = params::opt_f64(p, "skim", 0.0, 1.0e5)?.unwrap_or(0.0);
    let most = params::opt_f64(p, "count", 1.0, 10_000.0)?.map(|n| n as usize);
    let b = angle(p, "angle", 0.0)?;
    let alternate = params::opt_bool(p, "alternate")?.unwrap_or(true);
    let from_bottom = choice(p, "order", &["top_down", "bottom_up"])? == "bottom_up";
    needs_b(cx, &[b])?;
    let (top, bottom) = (cx.stock.top(), cx.stock.bottom());
    // Wire centres: the skim, then one slab plus a kerf below each.
    let mut heights = Vec::new();
    if skim > 0.0 {
        heights.push(top - skim);
    }
    let first = if skim > 0.0 { top - skim } else { top + c.kerf / 2.0 };
    let mut k = 1;
    loop {
        let h = first - k as f64 * (thickness + c.kerf);
        if h - c.kerf / 2.0 < bottom - 1.0e-6 || most.is_some_and(|n| k > n) || heights.len() >= 10_000 {
            break;
        }
        heights.push(h);
        k += 1;
    }
    if heights.is_empty() {
        return Err(EngineError::InvalidParams("the stock is thinner than one slab".into()));
    }
    if from_bottom {
        heights.reverse();
    }
    let (u0, u1) = cx.stock.u_range(b);
    let (left, right) = (u0 - c.clearance, u1 + c.clearance);
    let mut out = Built { path: WirePath::new(park(cx.machine, cx.stock, b)), ..Built::default() };
    let mut at_left = true;
    for (i, h) in heights.iter().enumerate() {
        let (s, e) = if at_left { (left, right) } else { (right, left) };
        if i == 0 {
            travel(cx, &c, &mut out.path, WirePos::new(s, *h, b), "left");
        } else {
            // Up or down the side, clear of the foam.
            out.path.push(MoveKind::Rapid, WirePos::new(s, *h, b), 0.0);
        }
        out.path.push(MoveKind::Cut, WirePos::new(e, *h, b), c.feed);
        if alternate {
            at_left = !at_left;
        } else {
            out.path.push(MoveKind::Rapid, WirePos::new(e, top + c.clearance, b), 0.0);
            out.path.push(MoveKind::Rapid, WirePos::new(s, top + c.clearance, b), 0.0);
        }
    }
    if skim > 0.0 {
        let cut = top - skim - c.kerf / 2.0;
        out.keeps.push(Keep {
            b,
            regions: vec![region::rectangle(DVec2::new(-FAR, -FAR), DVec2::new(FAR, cut))],
            outside: false,
            at: out.path.moves.len(),
            sweep: None,
        });
    }
    let slabs = heights.len() - usize::from(skim > 0.0);
    out.warnings.push(format!("{slabs} slabs of {thickness:.3} mm"));
    finish(cx, &c, &mut out);
    check_fits(cx, &mut out);
    Ok(out)
}

/// Warns when the path leaves the machine's travel.
pub(crate) fn check_fits(cx: &Context<'_>, out: &mut Built) {
    let m = cx.machine;
    let mut worst: Option<String> = None;
    for pos in std::iter::once(out.path.start).chain(out.path.moves.iter().map(|mv| mv.to)) {
        let (x, y) = m.machine_xy(pos.u, pos.h);
        if x < -1.0e-6 || x > m.travel_x + 1.0e-6 || y < -1.0e-6 || y > m.travel_y + 1.0e-6 {
            worst = Some(format!("the wire goes outside the machine's travel (X {x:.1}, Y {y:.1} mm)"));
            break;
        }
    }
    out.warnings.extend(worst);
    let lowest = out.path.moves.iter().map(|mv| mv.to.h).fold(f64::INFINITY, f64::min);
    if lowest < -1.0e-6 {
        out.warnings.push(format!("the wire goes {:.1} mm below the table top; raise the stock or the shape", -lowest));
    }
}

/// The parameters a cut was made with, tidied for storing: `name` goes to the cut itself.
pub fn stored_params(p: &Value) -> Value {
    let mut m: Map<String, Value> = p.as_object().cloned().unwrap_or_default();
    m.remove("name");
    m.remove("id");
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape;
    use glam::DVec3;

    fn job() -> (Machine, Stock, Vec<Shape>) {
        let stock = Stock::block(DVec3::new(300.0, 300.0, 300.0), DVec2::ZERO, 0.0, 0.0).unwrap_or_default();
        let shapes = vec![
            Shape::new("shape.1".into(), "square".into(), shape::rectangle(100.0, 100.0, 0.0, 0.01), String::new())
                .unwrap_or_else(|_| Shape {
                    id: String::new(),
                    name: String::new(),
                    paths: vec![],
                    source: String::new(),
                    sketch: None,
                }),
            Shape::new("shape.2".into(), "vase".into(), shape::rectangle(200.0, 250.0, 40.0, 0.01), String::new())
                .unwrap_or_else(|_| Shape {
                    id: String::new(),
                    name: String::new(),
                    paths: vec![],
                    source: String::new(),
                    sketch: None,
                }),
        ];
        (Machine::default(), stock, shapes)
    }

    #[test]
    fn the_wire_goes_no_lower_than_machine_y0() -> EngineResult<()> {
        let (mut m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        // A part standing on the table, cut right through: the wire runs along the table top,
        // not half a kerf under it.
        let on_table = json!({ "shape": "shape.1", "kerf": 1.0, "bottom": "through", "anchor": "bottom_centre" });
        let b = build(&cx, CutKind::Profile, &on_table)?;
        let lowest = b.path.moves.iter().map(|q| q.to.h).fold(f64::INFINITY, f64::min);
        assert!(lowest.abs() < 1e-9, "{lowest}");
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        let along: Vec<_> = b.path.moves.iter().filter(|q| q.kind == MoveKind::Cut && q.to.h.abs() < 1e-9).collect();
        assert!(along.len() >= 2, "the wire runs along the floor across the bottom");
        // With Y0 higher than the part's bottom, it runs level at Y0 and says so.
        m.wire_y0 = 20.0;
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let b = build(&cx, CutKind::Profile, &on_table)?;
        let lowest = b.path.moves.iter().map(|q| q.to.h).fold(f64::INFINITY, f64::min);
        assert!((lowest - 20.0).abs() < 1e-9, "{lowest}");
        assert!(b.warnings.iter().any(|w| w.contains("no lower than Y0")), "{:?}", b.warnings);
        Ok(())
    }

    #[test]
    fn a_profile_enters_from_the_left_and_comes_back() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let b = build(&cx, CutKind::Profile, &json!({ "shape": "shape.1", "kerf": 1.0, "bottom": "through" }))?;
        // The wire runs half a kerf outside the 100 mm square, centred in the block.
        let cuts: Vec<&WirePos> = b.path.moves.iter().filter(|m| m.kind == MoveKind::Cut).map(|m| &m.to).collect();
        let min_u = cuts.iter().map(|p| p.u).fold(f64::INFINITY, f64::min);
        assert!((min_u + 50.5).abs() < 0.02, "{min_u}");
        let leads: Vec<&WirePos> = b.path.moves.iter().filter(|m| m.kind == MoveKind::Lead).map(|m| &m.to).collect();
        assert_eq!(leads.len(), 2);
        // It leaves through the kerf it came in by.
        assert!(leads[1].u < -150.0 && (leads[1].h - leads[0].h).abs() < 1e-9);
        assert_eq!(b.keeps.len(), 1);
        // It ends where it left the foam, beside the stock: no climb over it.
        let end = b.path.end();
        let (u0, _) = s.u_range(end.b);
        assert!(end.u < u0 && end.h < s.top(), "{end:?}");
        Ok(())
    }

    #[test]
    fn an_attached_profile_goes_over_the_top_and_the_release_under() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let b = build(
            &cx,
            CutKind::Profile,
            &json!({ "shape": "shape.1", "kerf": 1.0, "anchor": "bottom_centre", "y": 50.0 }),
        )?;
        // In level from the left at the bottom, out level on the right.
        let leads: Vec<&WirePos> = b.path.moves.iter().filter(|m| m.kind == MoveKind::Lead).map(|m| &m.to).collect();
        assert_eq!(leads.len(), 2);
        assert!((leads[0].h - 49.5).abs() < 0.01 && leads[0].u < 0.0, "{leads:?}");
        assert!(leads[1].u > 150.0 && (leads[1].h - leads[0].h).abs() < 1e-9, "{leads:?}");
        // Nothing cut along the bottom; the release does that.
        let mut at = b.path.start;
        for m in &b.path.moves {
            let under =
                m.kind == MoveKind::Cut && at.h < 49.51 && m.to.h < 49.51 && at.u.abs() < 45.0 && m.to.u.abs() < 45.0;
            assert!(!under, "cuts under the part: {at:?} -> {:?}", m.to);
            at = m.to;
        }
        let rel = b.release.as_ref().map(|r| r.moves.iter().filter(|m| m.to.h < 50.0).count()).unwrap_or(0);
        assert!(rel >= 2);
        Ok(())
    }

    #[test]
    fn alternating_passes_start_where_the_last_one_finished() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let p = |alt: bool| json!({ "shape": "shape.1", "copies": 4, "step": 45.0, "alternate": alt });
        let both = build(&cx, CutKind::Profile, &p(true))?;
        let one = build(&cx, CutKind::Profile, &p(false))?;
        // The lead into each pass: left, right, left, right.
        let entries: Vec<f64> = both
            .path
            .moves
            .windows(2)
            .filter(|w| matches!(w[0].kind, MoveKind::Rapid | MoveKind::Rotate) && w[1].kind == MoveKind::Lead)
            .map(|w| w[0].to.u)
            .collect();
        assert_eq!(entries.len(), 4, "{entries:?}");
        assert!(entries[0] < 0.0 && entries[1] > 0.0 && entries[2] < 0.0 && entries[3] > 0.0, "{entries:?}");
        let air = |b: &Built| b.path.stats(m.rapid, m.b_rate).1;
        assert!(air(&both) < air(&one) * 0.9, "{} vs {}", air(&both), air(&one));
        Ok(())
    }

    #[test]
    fn indexed_and_two_view_cuts_turn_the_table_clear_of_the_foam() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        for (kind, p) in [
            (CutKind::Profile, json!({ "shape": "shape.1", "copies": 4 })),
            (CutKind::TwoView, json!({ "front": "shape.1", "side": "shape.2" })),
        ] {
            let b = build(&cx, kind, &p)?;
            let mut at = b.path.start;
            let mut turns = 0;
            for mv in &b.path.moves {
                if (mv.to.b - at.b).abs() > 1e-9 {
                    turns += 1;
                    // Above the foam, or out past its turning circle.
                    let clear = |p: &WirePos| p.h > s.top() || p.u.abs() >= s.reach();
                    assert!(clear(&at) && clear(&mv.to), "{kind:?} turns at {at:?}");
                }
                at = mv.to;
            }
            assert!(turns >= 1, "{kind:?}");
        }
        Ok(())
    }

    #[test]
    fn lathe_passes_follow_the_half_profile() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let b = build(&cx, CutKind::Lathe, &json!({ "shape": "shape.2", "step": 30, "compensate": false }))?;
        assert_eq!(b.keeps.len(), 12);
        let r = b.path.moves.iter().filter(|m| m.kind == MoveKind::Cut).map(|m| m.to.u).fold(0.0, f64::max);
        assert!((r - 100.0).abs() < 1e-6, "{r}");
        Ok(())
    }

    #[test]
    fn slabs_step_by_thickness_and_kerf() -> EngineResult<()> {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        let b = build(&cx, CutKind::Slab, &json!({ "thickness": 50.0, "kerf": 1.0, "skim": 5.0 }))?;
        let hs: Vec<f64> = b.path.moves.iter().filter(|m| m.kind == MoveKind::Cut).map(|m| m.to.h).collect();
        assert!((hs[0] - 295.0).abs() < 1e-9);
        assert!((hs[0] - hs[1] - 51.0).abs() < 1e-9);
        assert_eq!(hs.len(), 6);
        Ok(())
    }

    #[test]
    fn hostile_parameters_are_errors() {
        let (m, s, shapes) = job();
        let cx = Context { machine: &m, stock: &s, shapes: &shapes, models: &[], ignore: &[] };
        for p in [
            json!({}),
            json!({ "shape": "nope" }),
            json!({ "shape": "shape.1", "kerf": 1e9 }),
            json!({ "shape": "shape.1", "anchor": "moon" }),
            json!({ "shape": "shape.1", "kerf": 500.0 }),
            json!({ "shape": "shape.1", "copies": 1e12 }),
        ] {
            for kind in CutKind::ALL {
                let _ = build(&cx, kind, &p);
            }
        }
    }
}
