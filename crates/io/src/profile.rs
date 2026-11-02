//! 2D profile import: the outlines a hot wire follows.
//!
//! - DXF (ASCII): `LINE`, `ARC`, `CIRCLE`, `ELLIPSE`, `LWPOLYLINE` and 2D `POLYLINE` (with
//!   bulges) and `SPLINE` in the entities section. Loose segments are chained end to end.
//! - SVG: `path`, `polygon`, `polyline`, `line`, `rect`, `circle` and `ellipse`, with the
//!   `transform`s of the elements and their groups. User units are CSS pixels (96 per inch);
//!   Y is flipped so the drawing reads upright.
//! - DAT: airfoil coordinates in Selig (one loop, trailing edge round to trailing edge) or
//!   Lednicer (upper and lower surfaces, each from the leading edge) layout, chord 1.
//! - TXT / CSV: one `x y` (or `x,y`) point per line.
//!
//! Every reader treats its input as hostile: counts are capped, numbers must be finite, and a
//! bad file is an [`ImportError`], never a panic.

use std::f64::consts::{PI, TAU};

use CosmicCut_geom::DVec2;

use crate::{ImportError, Units};

/// Most points in one imported profile, all paths together.
pub const MAX_POINTS: usize = 2_000_000;
/// Most paths in one imported profile.
pub const MAX_PATHS: usize = 100_000;
/// Default chord tolerance for flattening arcs and curves, mm.
pub const DEFAULT_TOLERANCE: f64 = 0.02;

/// One polyline of a profile, in the file's units.
#[derive(Clone, Debug, PartialEq)]
pub struct Path2 {
    pub points: Vec<DVec2>,
    /// The last point joins back to the first.
    pub closed: bool,
}

impl Path2 {
    pub fn length(&self) -> f64 {
        let open: f64 = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
        match (self.closed, self.points.first(), self.points.last()) {
            (true, Some(a), Some(b)) => open + a.distance(*b),
            _ => open,
        }
    }
}

/// 2D file formats read as profiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileFormat {
    Dxf,
    Svg,
    Dat,
    Points,
}

impl ProfileFormat {
    pub const ALL: [Self; 4] = [Self::Dxf, Self::Svg, Self::Dat, Self::Points];

    pub fn id(self) -> &'static str {
        match self {
            Self::Dxf => "dxf",
            Self::Svg => "svg",
            Self::Dat => "dat",
            Self::Points => "points",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dxf => "DXF drawing",
            Self::Svg => "SVG drawing",
            Self::Dat => "Airfoil coordinates",
            Self::Points => "XY point list",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Dxf => &["dxf"],
            Self::Svg => &["svg"],
            Self::Dat => &["dat"],
            Self::Points => &["txt", "csv", "xy"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    pub fn from_path(path: &std::path::Path) -> Result<Self, ImportError> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
        Self::from_extension(ext).ok_or_else(|| {
            ImportError::Malformed(format!(
                "`.{ext}` is not a profile file; profiles are read from {}",
                extension_list()
            ))
        })
    }
}

/// "DXF, SVG, DAT, TXT, CSV, XY".
pub fn extension_list() -> String {
    ProfileFormat::ALL.iter().flat_map(|f| f.extensions()).map(|e| e.to_uppercase()).collect::<Vec<_>>().join(", ")
}

/// A profile read from a file.
#[derive(Clone, Debug, Default)]
pub struct ImportedProfile {
    pub paths: Vec<Path2>,
    /// The unit the file says it uses, when it says.
    pub units: Option<Units>,
    /// What was skipped or approximated, for the user.
    pub warnings: Vec<String>,
}

/// Reads a profile from `bytes`. Curves are flattened to within `tolerance` (file units).
pub fn read_profile(bytes: &[u8], format: ProfileFormat, tolerance: f64) -> Result<ImportedProfile, ImportError> {
    if bytes.len() as u64 > crate::MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance.max(1.0e-6) } else { DEFAULT_TOLERANCE };
    let text = String::from_utf8_lossy(bytes);
    let mut out = match format {
        ProfileFormat::Dxf => dxf(&text, tolerance)?,
        ProfileFormat::Svg => svg(&text, tolerance)?,
        ProfileFormat::Dat => dat(&text)?,
        ProfileFormat::Points => points(&text)?,
    };
    out.paths.retain(|p| p.points.len() >= 2);
    for p in &mut out.paths {
        dedupe(p);
    }
    out.paths.retain(|p| p.points.len() >= 2);
    if out.paths.is_empty() {
        return Err(ImportError::Malformed(format!("{}: the file has no outlines", format.label())));
    }
    let total: usize = out.paths.iter().map(|p| p.points.len()).sum();
    if total > MAX_POINTS || out.paths.len() > MAX_PATHS {
        return Err(ImportError::Malformed(format!("the profile has more than {MAX_POINTS} points")));
    }
    Ok(out)
}

/// Reads a profile file (from disk, or an upload in a browser).
pub fn import_profile(path: &std::path::Path, tolerance: f64) -> Result<ImportedProfile, ImportError> {
    let format = ProfileFormat::from_path(path)?;
    let bytes = match crate::files::read(&path.to_string_lossy(), crate::MAX_FILE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Err(ImportError::TooLarge),
        Err(e) => return Err(ImportError::Io { path: path.display().to_string(), message: e.to_string() }),
    };
    read_profile(&bytes, format, tolerance)
}

/// Drops repeated points, and a closing point that repeats the first.
fn dedupe(p: &mut Path2) {
    const EPS: f64 = 1.0e-9;
    p.points.dedup_by(|a, b| a.distance_squared(*b) < EPS * EPS);
    if p.points.len() > 2
        && let (Some(a), Some(b)) = (p.points.first(), p.points.last())
        && a.distance_squared(*b) < EPS * EPS
    {
        p.points.pop();
        p.closed = true;
    }
    if p.points.len() < 3 {
        p.closed = false;
    }
}

/// Segments to split an arc of `sweep` radians and radius `r` into, so its chords stray at most
/// `tol` from it.
fn arc_segments(r: f64, sweep: f64, tol: f64) -> usize {
    let r = r.abs();
    if !(r.is_finite() && sweep.is_finite()) || r <= tol {
        return 1;
    }
    let step = 2.0 * (1.0 - tol / r).clamp(-1.0, 1.0).acos();
    let n = if step > 1.0e-9 { (sweep.abs() / step).ceil() } else { 1.0 };
    (n as usize).clamp(1, 4096)
}

/// Points along the arc about `c`, radius `r`, from angle `a0` sweeping `sweep` (signed),
/// without the first point.
fn arc_points(c: DVec2, r: f64, a0: f64, sweep: f64, tol: f64, out: &mut Vec<DVec2>) {
    let n = arc_segments(r, sweep, tol);
    for i in 1..=n {
        let a = a0 + sweep * i as f64 / n as f64;
        out.push(c + DVec2::new(a.cos(), a.sin()) * r);
    }
}

/// Points of the bulged segment from `a` to `b` (bulge = tan of a quarter of the included
/// angle, positive counter-clockwise), without `a`.
fn bulge_points(a: DVec2, b: DVec2, bulge: f64, tol: f64, out: &mut Vec<DVec2>) {
    let chord = b - a;
    let len = chord.length();
    if bulge.abs() < 1.0e-9 || len < 1.0e-12 || !bulge.is_finite() {
        out.push(b);
        return;
    }
    let sweep = 4.0 * bulge.atan();
    let r = len / (2.0 * (sweep / 2.0).sin());
    // The centre sits off the chord's middle, on its left for a counter-clockwise bulge.
    let mid = (a + b) * 0.5;
    let normal = DVec2::new(-chord.y, chord.x) / len;
    let h = r * (sweep / 2.0).cos();
    let c = mid + normal * h;
    let a0 = (a - c).to_angle();
    arc_points(c, r.abs(), a0, sweep, tol, out);
    if let Some(last) = out.last_mut() {
        *last = b;
    }
}

// ---------------------------------------------------------------------------------------------
// DXF

#[derive(Debug, Default)]
struct Entity {
    kind: String,
    codes: Vec<(i32, String)>,
}

impl Entity {
    fn get(&self, code: i32) -> Option<&str> {
        self.codes.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
    }

    fn num(&self, code: i32, default: f64) -> f64 {
        self.get(code).and_then(|v| crate::finite64(v.trim())).unwrap_or(default)
    }

    fn int(&self, code: i32) -> i64 {
        self.get(code).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
    }

    fn all(&self, code: i32) -> Vec<f64> {
        self.codes.iter().filter(|(c, _)| *c == code).filter_map(|(_, v)| crate::finite64(v.trim())).collect()
    }
}

/// The group-code pairs of the entities section (and the header's unit), grouped at each code 0.
fn dxf_entities(text: &str) -> Result<(Vec<Entity>, Option<i64>), ImportError> {
    let mut lines = text.lines();
    let mut all: Vec<Entity> = Vec::new();
    let mut line = 0usize;
    while let Some(code) = lines.next() {
        line += 2;
        let value = lines.next().unwrap_or_default().trim().to_owned();
        let code: i32 = code.trim().parse().map_err(|_| ImportError::Syntax {
            format: "DXF",
            line: line - 1,
            message: format!("`{}` is not a group code", code.trim().chars().take(32).collect::<String>()),
        })?;
        if code == 0 {
            if all.len() >= 10_000_000 {
                return Err(ImportError::Malformed("DXF: too many entities".into()));
            }
            all.push(Entity { kind: value, codes: Vec::new() });
        } else if let Some(e) = all.last_mut() {
            e.codes.push((code, value));
        }
    }
    // $INSUNITS sits in the header section's first entity as a 9/70 pair.
    let mut units = None;
    if let Some(header) = all.first() {
        let mut it = header.codes.iter();
        while let Some((c, v)) = it.next() {
            if *c == 9
                && v == "$INSUNITS"
                && let Some((70, n)) = it.next()
            {
                units = n.trim().parse().ok();
            }
        }
    }
    // Only the ENTITIES section: SECTION/2 ENTITIES … ENDSEC.
    let mut out = Vec::new();
    let mut inside = false;
    for e in all {
        match e.kind.as_str() {
            "SECTION" => inside = e.get(2) == Some("ENTITIES"),
            "ENDSEC" => inside = false,
            _ if inside => out.push(e),
            _ => {}
        }
    }
    Ok((out, units))
}

fn dxf_units(code: Option<i64>) -> Option<Units> {
    match code? {
        1 => Some(Units::Inches),
        4 => Some(Units::Millimetres),
        5 => Some(Units::Centimetres),
        6 => Some(Units::Metres),
        _ => None,
    }
}

fn dxf(text: &str, tol: f64) -> Result<ImportedProfile, ImportError> {
    let (entities, units) = dxf_entities(text)?;
    let mut out = ImportedProfile { units: dxf_units(units), ..Default::default() };
    let mut loose: Vec<Path2> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(e) = entities.get(i) {
        i += 1;
        let p = |x: i32| DVec2::new(e.num(x, 0.0), e.num(x + 10, 0.0));
        match e.kind.as_str() {
            "LINE" => loose.push(Path2 { points: vec![p(10), p(11)], closed: false }),
            "ARC" => {
                let (c, r) = (p(10), e.num(40, 0.0));
                let (a0, mut a1) = (e.num(50, 0.0).to_radians(), e.num(51, 360.0).to_radians());
                while a1 <= a0 {
                    a1 += TAU;
                }
                let mut pts = vec![c + DVec2::new(a0.cos(), a0.sin()) * r];
                arc_points(c, r, a0, a1 - a0, tol, &mut pts);
                loose.push(Path2 { points: pts, closed: false });
            }
            "CIRCLE" => {
                let (c, r) = (p(10), e.num(40, 0.0));
                let mut pts = vec![c + DVec2::new(r, 0.0)];
                arc_points(c, r, 0.0, TAU, tol, &mut pts);
                pts.pop();
                out.paths.push(Path2 { points: pts, closed: true });
            }
            "ELLIPSE" => {
                let (c, major) = (p(10), p(11));
                let ratio = e.num(40, 1.0);
                let (t0, mut t1) = (e.num(41, 0.0), e.num(42, TAU));
                while t1 <= t0 {
                    t1 += TAU;
                }
                let minor = DVec2::new(-major.y, major.x) * ratio;
                let n = arc_segments(major.length(), t1 - t0, tol);
                let pts: Vec<DVec2> = (0..=n)
                    .map(|k| {
                        let t = t0 + (t1 - t0) * k as f64 / n as f64;
                        c + major * t.cos() + minor * t.sin()
                    })
                    .collect();
                let closed = (t1 - t0 - TAU).abs() < 1.0e-6;
                push(&mut out.paths, &mut loose, Path2 { points: pts, closed });
            }
            "LWPOLYLINE" => {
                // Vertices in order: each 10 starts one; a 42 after it is its bulge.
                let mut verts: Vec<(DVec2, f64)> = Vec::new();
                let mut pending_x: Option<f64> = None;
                for (code, v) in &e.codes {
                    let n = crate::finite64(v.trim());
                    match (code, n) {
                        (10, Some(x)) => pending_x = Some(x),
                        (20, Some(y)) => {
                            if let Some(x) = pending_x.take() {
                                verts.push((DVec2::new(x, y), 0.0));
                            }
                        }
                        (42, Some(b)) => {
                            if let Some(last) = verts.last_mut() {
                                last.1 = b;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = e.int(70) & 1 == 1;
                push(&mut out.paths, &mut loose, polyline(&verts, closed, tol));
            }
            "POLYLINE" => {
                let flags = e.int(70);
                let mut verts = Vec::new();
                while let Some(v) = entities.get(i) {
                    i += 1;
                    match v.kind.as_str() {
                        "VERTEX" => verts.push((DVec2::new(v.num(10, 0.0), v.num(20, 0.0)), v.num(42, 0.0))),
                        _ => break,
                    }
                }
                if flags & (16 | 64) != 0 {
                    skipped.push("polygon meshes".into());
                } else {
                    push(&mut out.paths, &mut loose, polyline(&verts, flags & 1 == 1, tol));
                }
            }
            "SPLINE" => match spline(e, tol) {
                Some(path) => push(&mut out.paths, &mut loose, path),
                None => skipped.push("a spline it could not evaluate".into()),
            },
            "INSERT" => skipped.push("block references (explode them first)".into()),
            "TEXT" | "MTEXT" => skipped.push("text (convert it to outlines first)".into()),
            "HATCH" | "DIMENSION" | "POINT" | "VIEWPORT" | "SEQEND" | "ATTRIB" | "ATTDEF" | "LEADER" => {}
            other => skipped.push(format!("{other} entities")),
        }
        if out.paths.len() + loose.len() > MAX_PATHS {
            return Err(ImportError::Malformed(format!("DXF: more than {MAX_PATHS} outlines")));
        }
    }
    out.paths.extend(chain(loose, tol.max(1.0e-6) * 5.0));
    skipped.sort();
    skipped.dedup();
    out.warnings.extend(skipped.into_iter().map(|s| format!("DXF: skipped {s}")));
    Ok(out)
}

/// A closed path goes straight in; an open one waits to be chained with the loose segments.
fn push(paths: &mut Vec<Path2>, loose: &mut Vec<Path2>, p: Path2) {
    if p.closed { paths.push(p) } else { loose.push(p) }
}

fn polyline(verts: &[(DVec2, f64)], closed: bool, tol: f64) -> Path2 {
    let mut pts = Vec::with_capacity(verts.len());
    let Some(first) = verts.first() else {
        return Path2 { points: pts, closed };
    };
    pts.push(first.0);
    for w in verts.windows(2) {
        bulge_points(w[0].0, w[1].0, w[0].1, tol, &mut pts);
    }
    if closed && let Some(last) = verts.last() {
        bulge_points(last.0, first.0, last.1, tol, &mut pts);
        pts.pop();
    }
    Path2 { points: pts, closed }
}

/// A DXF spline by de Boor's algorithm, or its fit points when it has no control points.
fn spline(e: &Entity, tol: f64) -> Option<Path2> {
    let degree = e.int(71).clamp(1, 11) as usize;
    let knots = e.all(40);
    let (xs, ys) = (e.all(10), e.all(20));
    let ctrl: Vec<DVec2> = xs.iter().zip(&ys).map(|(x, y)| DVec2::new(*x, *y)).collect();
    let closed = e.int(70) & 1 == 1;
    if ctrl.len() < 2 {
        let (fx, fy) = (e.all(11), e.all(21));
        let fit: Vec<DVec2> = fx.iter().zip(&fy).map(|(x, y)| DVec2::new(*x, *y)).collect();
        return (fit.len() >= 2).then_some(Path2 { points: fit, closed });
    }
    if ctrl.len() > 100_000 || knots.len() != ctrl.len() + degree + 1 {
        return None;
    }
    let weights = e.all(41);
    let w = |i: usize| weights.get(i).copied().filter(|w| *w > 0.0).unwrap_or(1.0);
    let (t0, t1) = (*knots.get(degree)?, *knots.get(ctrl.len())?);
    if t1.partial_cmp(&t0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    // Enough samples for the control polygon's length at the tolerance.
    let poly: f64 = ctrl.windows(2).map(|p| p[0].distance(p[1])).sum();
    let n = ((poly / tol.max(1.0e-6)).sqrt() * 4.0).ceil().clamp(8.0, 20_000.0) as usize;
    let mut pts = Vec::with_capacity(n + 1);
    for s in 0..=n {
        let t = (t0 + (t1 - t0) * s as f64 / n as f64).min(t1 - 1.0e-12 * (t1 - t0).max(1.0));
        let t = if s == n { t1 - 1.0e-12 } else { t };
        // The knot span holding t.
        let mut k = degree;
        while k + 1 < ctrl.len() && knots.get(k + 1).is_some_and(|kn| t >= *kn) {
            k += 1;
        }
        // Homogeneous de Boor.
        let mut d: Vec<(DVec2, f64)> = (0..=degree)
            .map(|j| {
                let idx = (k + j).saturating_sub(degree);
                let c = ctrl.get(idx).copied().unwrap_or_default();
                (c * w(idx), w(idx))
            })
            .collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = k + j - degree;
                let (a, b) = (knots.get(i).copied()?, knots.get(i + degree + 1 - r).copied()?);
                let alpha = if b > a { (t - a) / (b - a) } else { 0.0 };
                let (p0, w0) = *d.get(j - 1)?;
                let (p1, w1) = *d.get(j)?;
                if let Some(slot) = d.get_mut(j) {
                    *slot = (p0 * (1.0 - alpha) + p1 * alpha, w0 * (1.0 - alpha) + w1 * alpha);
                }
            }
        }
        let (p, wt) = *d.get(degree)?;
        if wt.abs() > 1.0e-12 && (p / wt).is_finite() {
            pts.push(p / wt);
        }
    }
    Some(Path2 { points: pts, closed })
}

/// Joins open paths whose ends meet (within `gap`) into longer paths; a chain that comes back
/// to its start is closed.
pub fn chain(mut open: Vec<Path2>, gap: f64) -> Vec<Path2> {
    let mut out = Vec::new();
    let near = |a: DVec2, b: DVec2| a.distance_squared(b) <= gap * gap;
    while let Some(mut cur) = open.pop() {
        while let (Some(&head), Some(&tail)) = (cur.points.first(), cur.points.last()) {
            if cur.points.len() > 2 && near(head, tail) {
                cur.points.pop();
                cur.closed = true;
                break;
            }
            let found = open.iter().position(|p| {
                let (a, b) =
                    (p.points.first().copied().unwrap_or_default(), p.points.last().copied().unwrap_or_default());
                near(a, tail) || near(b, tail) || near(a, head) || near(b, head)
            });
            let Some(idx) = found else { break };
            let mut next = open.swap_remove(idx);
            let (a, b) =
                (next.points.first().copied().unwrap_or_default(), next.points.last().copied().unwrap_or_default());
            if near(a, tail) {
                cur.points.extend(next.points.into_iter().skip(1));
            } else if near(b, tail) {
                next.points.reverse();
                cur.points.extend(next.points.into_iter().skip(1));
            } else if near(b, head) {
                next.points.pop();
                next.points.extend(cur.points);
                cur.points = next.points;
            } else {
                next.points.reverse();
                next.points.pop();
                next.points.extend(cur.points);
                cur.points = next.points;
            }
        }
        out.push(cur);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// SVG

/// A 2D affine map: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine([f64; 6]);

impl Affine {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn apply(&self, p: DVec2) -> DVec2 {
        let [a, b, c, d, e, f] = self.0;
        DVec2::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// `self` after `other`: applies `other` first.
    fn then(&self, other: &Self) -> Self {
        let [a1, b1, c1, d1, e1, f1] = other.0;
        let [a2, b2, c2, d2, e2, f2] = self.0;
        Self([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }

    /// How much the map scales lengths, for flattening tolerances.
    fn scale(&self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        ((a * d - b * c).abs()).sqrt().max(1.0e-9)
    }
}

/// Parses an SVG `transform` list.
fn parse_transform(s: &str) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let args = numbers(&rest[open + 1..open + close]);
        let g = |i: usize, d: f64| args.get(i).copied().unwrap_or(d);
        let t = match name {
            "matrix" if args.len() == 6 => Affine([g(0, 1.0), g(1, 0.0), g(2, 0.0), g(3, 1.0), g(4, 0.0), g(5, 0.0)]),
            "translate" => Affine([1.0, 0.0, 0.0, 1.0, g(0, 0.0), g(1, 0.0)]),
            "scale" => Affine([g(0, 1.0), 0.0, 0.0, g(1, g(0, 1.0)), 0.0, 0.0]),
            "rotate" => {
                let (r, cx, cy) = (g(0, 0.0).to_radians(), g(1, 0.0), g(2, 0.0));
                let (s, c) = r.sin_cos();
                let rot = Affine([c, s, -s, c, 0.0, 0.0]);
                Affine([1.0, 0.0, 0.0, 1.0, cx, cy]).then(&rot.then(&Affine([1.0, 0.0, 0.0, 1.0, -cx, -cy])))
            }
            "skewX" => Affine([1.0, 0.0, g(0, 0.0).to_radians().tan(), 1.0, 0.0, 0.0]),
            "skewY" => Affine([1.0, g(0, 0.0).to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => Affine::IDENTITY,
        };
        m = m.then(&t);
        rest = &rest[open + close + 1..];
    }
    m
}

/// Every number in `s` (SVG number syntax: `1e-3`, `.5.5` is two numbers, `-1-2` is two).
fn numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && out.len() < MAX_POINTS * 2 {
        let start = i;
        let c = b.get(i).copied().unwrap_or(b' ');
        if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() {
            i += 1;
            let mut dot = c == b'.';
            while let Some(&d) = b.get(i) {
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == b'.' && !dot {
                    dot = true;
                    i += 1;
                } else if (d == b'e' || d == b'E')
                    && b.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == b'-' || *n == b'+')
                {
                    i += 2;
                    while b.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                    break;
                } else {
                    break;
                }
            }
            if let Some(n) = s.get(start..i).and_then(|t| t.parse::<f64>().ok()).filter(|n| n.is_finite()) {
                out.push(n);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// A length attribute in user units ("12", "12px", "3mm", "1in").
fn svg_length(v: Option<&str>) -> f64 {
    let Some(v) = v else { return 0.0 };
    let v = v.trim();
    let n = numbers(v).first().copied().unwrap_or(0.0);
    let per = if v.ends_with("mm") {
        96.0 / 25.4
    } else if v.ends_with("cm") {
        96.0 / 2.54
    } else if v.ends_with("in") {
        96.0
    } else if v.ends_with("pt") {
        96.0 / 72.0
    } else {
        1.0
    };
    n * per
}

fn svg(text: &str, tol: f64) -> Result<ImportedProfile, ImportError> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Affine> = vec![Affine::IDENTITY];
    let mut out = ImportedProfile::default();
    // SVG user units are CSS pixels, unless the root's viewBox and size say otherwise.
    let mut root_scale = 25.4 / 96.0;
    let mut seen_root = false;
    let mut skipped: Vec<String> = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| ImportError::Malformed(format!("SVG: {e}")))?;
        let (start, empty) = match &event {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(_) => {
                stack.pop();
                if stack.is_empty() {
                    stack.push(Affine::IDENTITY);
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let Some(e) = start else { continue };
        let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
        let mut attrs: Vec<(String, String)> = Vec::new();
        for a in e.attributes().flatten() {
            let key = String::from_utf8_lossy(a.key.local_name().as_ref()).into_owned();
            let value =
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default();
            attrs.push((key, value));
        }
        let attr = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let parent = stack.last().copied().unwrap_or(Affine::IDENTITY);
        let mut m = match attr("transform") {
            Some(t) => parent.then(&parse_transform(t)),
            None => parent,
        };
        if name == "svg" && !seen_root {
            seen_root = true;
            // A physical width with a viewBox fixes the size of one user unit.
            let vb = attr("viewBox").map(numbers).unwrap_or_default();
            let width = attr("width").filter(|w| !w.trim().ends_with('%'));
            if let (Some(w), Some(vw)) = (width, vb.get(2).copied().filter(|v| *v > 0.0)) {
                root_scale = svg_length(Some(w)) / vw * 25.4 / 96.0;
            }
            if let (Some(x), Some(y)) = (vb.first(), vb.get(1)) {
                m = m.then(&Affine([1.0, 0.0, 0.0, 1.0, -x, -y]));
            }
        }
        let tol_here = tol / (m.scale() * root_scale).max(1.0e-9);
        let mut shapes: Vec<Path2> = Vec::new();
        match name.as_str() {
            "path" => shapes = path_data(attr("d").unwrap_or_default(), tol_here),
            "polygon" | "polyline" => {
                let n = numbers(attr("points").unwrap_or_default());
                let pts: Vec<DVec2> = n.as_chunks::<2>().0.iter().map(|[x, y]| DVec2::new(*x, *y)).collect();
                shapes.push(Path2 { points: pts, closed: name == "polygon" });
            }
            "line" => {
                let g = |k| svg_length(attr(k));
                shapes.push(Path2 {
                    points: vec![DVec2::new(g("x1"), g("y1")), DVec2::new(g("x2"), g("y2"))],
                    closed: false,
                });
            }
            "rect" => {
                let g = |k| svg_length(attr(k));
                let (x, y, w, h) = (g("x"), g("y"), g("width"), g("height"));
                if w > 0.0 && h > 0.0 {
                    let mut rx = g("rx");
                    let mut ry = g("ry");
                    if attr("rx").is_none() {
                        rx = ry;
                    }
                    if attr("ry").is_none() {
                        ry = rx;
                    }
                    let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
                    shapes.push(rounded_rect(x, y, w, h, rx, ry, tol_here));
                }
            }
            "circle" | "ellipse" => {
                let g = |k| svg_length(attr(k));
                let (cx, cy) = (g("cx"), g("cy"));
                let (rx, ry) = if name == "circle" { (g("r"), g("r")) } else { (g("rx"), g("ry")) };
                if rx > 0.0 && ry > 0.0 {
                    let n = arc_segments(rx.max(ry), TAU, tol_here).max(8);
                    let pts = (0..n)
                        .map(|k| {
                            let t = TAU * k as f64 / n as f64;
                            DVec2::new(cx + rx * t.cos(), cy + ry * t.sin())
                        })
                        .collect();
                    shapes.push(Path2 { points: pts, closed: true });
                }
            }
            "text" => skipped.push("text (convert it to paths first)".into()),
            "image" => skipped.push("images".into()),
            "use" => skipped.push("<use> references".into()),
            _ => {}
        }
        // Definitions are templates, not drawing.
        let in_defs = name == "defs" || name == "clipPath" || name == "mask" || name == "symbol";
        for mut s in shapes {
            for p in &mut s.points {
                let q = m.apply(*p) * root_scale;
                *p = DVec2::new(q.x, -q.y);
            }
            out.paths.push(s);
        }
        if !empty {
            // Children of definitions are pushed with a map that throws them away.
            stack.push(if in_defs { Affine([0.0; 6]) } else { m });
        }
        if out.paths.len() > MAX_PATHS {
            return Err(ImportError::Malformed(format!("SVG: more than {MAX_PATHS} outlines")));
        }
    }
    // Shapes squashed to nothing by a definitions map are dropped.
    out.paths.retain(|p| p.points.iter().any(|q| q.length_squared() > 0.0));
    out.units = Some(Units::Millimetres);
    skipped.sort();
    skipped.dedup();
    out.warnings.extend(skipped.into_iter().map(|s| format!("SVG: skipped {s}")));
    Ok(out)
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64, tol: f64) -> Path2 {
    if rx <= 0.0 || ry <= 0.0 {
        let pts = vec![DVec2::new(x, y), DVec2::new(x + w, y), DVec2::new(x + w, y + h), DVec2::new(x, y + h)];
        return Path2 { points: pts, closed: true };
    }
    let mut pts = Vec::new();
    let corners = [
        (DVec2::new(x + w - rx, y + ry), -PI / 2.0),
        (DVec2::new(x + w - rx, y + h - ry), 0.0),
        (DVec2::new(x + rx, y + h - ry), PI / 2.0),
        (DVec2::new(x + rx, y + ry), PI),
    ];
    let n = arc_segments(rx.max(ry), PI / 2.0, tol);
    for (c, a0) in corners {
        for k in 0..=n {
            let t = a0 + PI / 2.0 * k as f64 / n as f64;
            pts.push(c + DVec2::new(rx * t.cos(), ry * t.sin()));
        }
    }
    Path2 { points: pts, closed: true }
}

/// Flattens SVG path data into polylines.
fn path_data(d: &str, tol: f64) -> Vec<Path2> {
    let mut out: Vec<Path2> = Vec::new();
    let mut cur: Vec<DVec2> = Vec::new();
    let mut pos = DVec2::ZERO;
    let mut start = DVec2::ZERO;
    // The last control point, for S and T.
    let mut last_ctrl: Option<(char, DVec2)> = None;
    let mut cmd = ' ';
    let bytes = d.as_bytes();
    let mut i = 0;
    let finish = |cur: &mut Vec<DVec2>, out: &mut Vec<Path2>, closed: bool| {
        if cur.len() >= 2 {
            out.push(Path2 { points: std::mem::take(cur), closed });
        } else {
            cur.clear();
        }
    };
    let mut guard = 0usize;
    while i < bytes.len() {
        guard += 1;
        if guard > MAX_POINTS {
            break;
        }
        let c = bytes.get(i).copied().unwrap_or(b' ') as char;
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            cmd = c;
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                pos = start;
                finish(&mut cur, &mut out, true);
                last_ctrl = None;
            }
            continue;
        }
        if c.is_whitespace() || c == ',' {
            i += 1;
            continue;
        }
        let arity = match cmd.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => {
                i += 1;
                continue;
            }
        };
        // Read `arity` numbers; arc flags may be written without separators ("011").
        let mut args = Vec::with_capacity(arity);
        while args.len() < arity && i < bytes.len() {
            let ch = bytes.get(i).copied().unwrap_or(b' ');
            if ch.is_ascii_whitespace() || ch == b',' {
                i += 1;
                continue;
            }
            if cmd.eq_ignore_ascii_case(&'a') && (args.len() == 3 || args.len() == 4) && (ch == b'0' || ch == b'1') {
                args.push(f64::from(ch - b'0'));
                i += 1;
                continue;
            }
            let rest = d.get(i..).unwrap_or_default();
            let len = number_len(rest);
            if len == 0 {
                break;
            }
            match rest.get(..len).and_then(|t| t.parse::<f64>().ok()).filter(|n| n.is_finite()) {
                Some(n) => args.push(n),
                None => break,
            }
            i += len;
        }
        if args.len() < arity {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { pos } else { DVec2::ZERO };
        let a = |k: usize| args.get(k).copied().unwrap_or(0.0);
        match cmd.to_ascii_uppercase() {
            'M' => {
                finish(&mut cur, &mut out, false);
                pos = base + DVec2::new(a(0), a(1));
                start = pos;
                cur.push(pos);
                // Later pairs after a move are lines.
                cmd = if rel { 'l' } else { 'L' };
                last_ctrl = None;
            }
            'L' => {
                pos = base + DVec2::new(a(0), a(1));
                cur.push(pos);
                last_ctrl = None;
            }
            'H' => {
                pos = DVec2::new(if rel { pos.x + a(0) } else { a(0) }, pos.y);
                cur.push(pos);
                last_ctrl = None;
            }
            'V' => {
                pos = DVec2::new(pos.x, if rel { pos.y + a(0) } else { a(0) });
                cur.push(pos);
                last_ctrl = None;
            }
            'C' | 'S' => {
                let (c1, c2, end) = if cmd.eq_ignore_ascii_case(&'c') {
                    (base + DVec2::new(a(0), a(1)), base + DVec2::new(a(2), a(3)), base + DVec2::new(a(4), a(5)))
                } else {
                    let c1 = match last_ctrl {
                        Some(('C', c)) => pos * 2.0 - c,
                        _ => pos,
                    };
                    (c1, base + DVec2::new(a(0), a(1)), base + DVec2::new(a(2), a(3)))
                };
                if cur.is_empty() {
                    cur.push(pos);
                }
                cubic(pos, c1, c2, end, tol, &mut cur);
                last_ctrl = Some(('C', c2));
                pos = end;
            }
            'Q' | 'T' => {
                let (c, end) = if cmd.eq_ignore_ascii_case(&'q') {
                    (base + DVec2::new(a(0), a(1)), base + DVec2::new(a(2), a(3)))
                } else {
                    let c = match last_ctrl {
                        Some(('Q', c)) => pos * 2.0 - c,
                        _ => pos,
                    };
                    (c, base + DVec2::new(a(0), a(1)))
                };
                if cur.is_empty() {
                    cur.push(pos);
                }
                // A quadratic as the cubic with the same shape.
                let (c1, c2) = (pos + (c - pos) * (2.0 / 3.0), end + (c - end) * (2.0 / 3.0));
                cubic(pos, c1, c2, end, tol, &mut cur);
                last_ctrl = Some(('Q', c));
                pos = end;
            }
            'A' => {
                let end = base + DVec2::new(a(5), a(6));
                if cur.is_empty() {
                    cur.push(pos);
                }
                svg_arc(pos, a(0), a(1), a(2), a(3) != 0.0, a(4) != 0.0, end, tol, &mut cur);
                pos = end;
                last_ctrl = None;
            }
            _ => {}
        }
    }
    finish(&mut cur, &mut out, false);
    out
}

/// Length of the number at the start of `s`, 0 if there is none.
fn number_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let mut digits = false;
    let mut dot = false;
    while let Some(&c) = b.get(i) {
        if c.is_ascii_digit() {
            digits = true;
            i += 1;
        } else if c == b'.' && !dot {
            dot = true;
            i += 1;
        } else {
            break;
        }
    }
    if !digits {
        return 0;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'-' | b'+')) {
            j += 1;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            i = j;
        }
    }
    i
}

/// A cubic Bézier flattened into `out` (without its start point).
fn cubic(p0: DVec2, p1: DVec2, p2: DVec2, p3: DVec2, tol: f64, out: &mut Vec<DVec2>) {
    // Segments from the control polygon's deviation: n = sqrt(|d| / (8 tol)) * 3/2 bound.
    let dd = ((p0 - p1 * 2.0 + p2).length()).max((p1 - p2 * 2.0 + p3).length());
    let n = ((dd * 0.75 / tol.max(1.0e-9)).sqrt().ceil() as usize).clamp(1, 1000);
    for k in 1..=n {
        let t = k as f64 / n as f64;
        let u = 1.0 - t;
        out.push(p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t));
    }
}

/// An SVG elliptical arc (endpoint form) flattened into `out` (without its start point).
#[allow(clippy::too_many_arguments)]
fn svg_arc(
    p0: DVec2,
    rx: f64,
    ry: f64,
    phi_deg: f64,
    large: bool,
    sweep: bool,
    p1: DVec2,
    tol: f64,
    out: &mut Vec<DVec2>,
) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1.0e-12 || ry < 1.0e-12 || p0.distance_squared(p1) < 1.0e-24 {
        out.push(p1);
        return;
    }
    let (s, c) = phi_deg.to_radians().sin_cos();
    let h = (p0 - p1) * 0.5;
    let x1 = DVec2::new(c * h.x + s * h.y, -s * h.x + c * h.y);
    let lambda = (x1.x * x1.x) / (rx * rx) + (x1.y * x1.y) / (ry * ry);
    if lambda > 1.0 {
        let k = lambda.sqrt();
        rx *= k;
        ry *= k;
    }
    let num = rx * rx * ry * ry - rx * rx * x1.y * x1.y - ry * ry * x1.x * x1.x;
    let den = rx * rx * x1.y * x1.y + ry * ry * x1.x * x1.x;
    let mut coef = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
    if large == sweep {
        coef = -coef;
    }
    let cp = DVec2::new(coef * rx * x1.y / ry, -coef * ry * x1.x / rx);
    let mid = (p0 + p1) * 0.5;
    let centre = DVec2::new(c * cp.x - s * cp.y, s * cp.x + c * cp.y) + mid;
    let angle = |u: DVec2, v: DVec2| {
        let a = u.x * v.y - u.y * v.x;
        a.atan2(u.dot(v))
    };
    let u = DVec2::new((x1.x - cp.x) / rx, (x1.y - cp.y) / ry);
    let v = DVec2::new((-x1.x - cp.x) / rx, (-x1.y - cp.y) / ry);
    let t0 = angle(DVec2::X, u);
    let mut dt = angle(u, v);
    if !sweep && dt > 0.0 {
        dt -= TAU;
    } else if sweep && dt < 0.0 {
        dt += TAU;
    }
    let n = arc_segments(rx.max(ry), dt, tol);
    for k in 1..=n {
        let t = t0 + dt * k as f64 / n as f64;
        let e = DVec2::new(rx * t.cos(), ry * t.sin());
        out.push(centre + DVec2::new(c * e.x - s * e.y, s * e.x + c * e.y));
    }
    if let Some(last) = out.last_mut() {
        *last = p1;
    }
}

// ---------------------------------------------------------------------------------------------
// DAT and point lists

/// The numeric (x, y) pairs of each line, by line; lines that are not two numbers are None.
fn pairs(text: &str) -> Vec<Option<DVec2>> {
    text.lines()
        .take(MAX_POINTS)
        .map(|l| {
            let n: Vec<f64> = l
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .filter(|t| !t.is_empty())
                .map_while(|t| t.parse().ok())
                .collect();
            match n.as_slice() {
                [x, y, ..] if x.is_finite() && y.is_finite() => Some(DVec2::new(*x, *y)),
                _ => None,
            }
        })
        .collect()
}

fn dat(text: &str) -> Result<ImportedProfile, ImportError> {
    let rows: Vec<DVec2> = pairs(text).into_iter().flatten().collect();
    let Some(first) = rows.first() else {
        return Err(ImportError::Malformed("airfoil: no coordinates".into()));
    };
    let mut out = ImportedProfile::default();
    // Lednicer: the first pair counts the upper and lower points.
    let pts = if first.x > 1.5 && first.y > 1.5 {
        let (nu, nl) = (first.x as usize, first.y as usize);
        let upper: Vec<DVec2> = rows.iter().skip(1).take(nu).copied().collect();
        let lower: Vec<DVec2> = rows.iter().skip(1 + nu).take(nl).copied().collect();
        if upper.len() != nu || lower.len() != nl {
            return Err(ImportError::Malformed("airfoil: fewer points than the Lednicer header says".into()));
        }
        // Round from the upper trailing edge, over the nose and back along the lower surface.
        let mut pts: Vec<DVec2> = upper.into_iter().rev().collect();
        pts.extend(lower.into_iter().skip(1));
        pts
    } else {
        rows
    };
    out.paths.push(Path2 { points: pts, closed: true });
    Ok(out)
}

fn points(text: &str) -> Result<ImportedProfile, ImportError> {
    let mut out = ImportedProfile::default();
    // A blank or non-numeric line starts a new path.
    let mut cur = Vec::new();
    for row in pairs(text) {
        match row {
            Some(p) => cur.push(p),
            None if !cur.is_empty() => out.paths.push(Path2 { points: std::mem::take(&mut cur), closed: false }),
            None => {}
        }
    }
    if !cur.is_empty() {
        out.paths.push(Path2 { points: cur, closed: false });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    fn read(text: &str, f: ProfileFormat) -> ImportedProfile {
        read_profile(text.as_bytes(), f, 0.01).unwrap_or_else(|e| panic!("{e}"))
    }

    fn area(p: &Path2) -> f64 {
        CosmicCut_geom::offset::signed_area2(&p.points) / 2.0
    }

    #[test]
    fn dxf_lines_and_arcs_chain_into_a_closed_slot() {
        // A 40 × 20 slot: two lines and two half circles, drawn in any order and direction.
        let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLINE\n10\n0\n20\n0\n11\n40\n21\n0\n\
0\nARC\n10\n0\n20\n10\n40\n10\n50\n90\n51\n270\n\
0\nLINE\n10\n0\n20\n20\n11\n40\n21\n20\n\
0\nARC\n10\n40\n20\n10\n40\n10\n50\n270\n51\n90\n\
0\nENDSEC\n0\nEOF\n";
        let p = read(dxf, ProfileFormat::Dxf);
        assert_eq!(p.units, Some(Units::Millimetres));
        assert_eq!(p.paths.len(), 1);
        assert!(p.paths[0].closed);
        let want = 40.0 * 20.0 + PI * 100.0;
        assert!((area(&p.paths[0]).abs() - want).abs() < 1.0, "{}", area(&p.paths[0]));
    }

    #[test]
    fn dxf_lwpolyline_bulges_make_arcs() {
        // A circle of radius 10 as two bulged segments.
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nLWPOLYLINE\n90\n2\n70\n1\n10\n-10\n20\n0\n42\n1\n10\n10\n20\n0\n42\n1\n0\nENDSEC\n0\nEOF\n";
        let p = read(dxf, ProfileFormat::Dxf);
        assert_eq!(p.paths.len(), 1);
        assert!((area(&p.paths[0]).abs() - PI * 100.0).abs() < 0.5);
        assert!(p.paths[0].points.iter().all(|q| (q.length() - 10.0).abs() < 1.0e-6));
    }

    #[test]
    fn svg_paths_flip_and_scale() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
            <g transform="translate(10,0)"><path d="M0 0 H 20 V 10 H 0 Z"/></g>
            <circle cx="50" cy="25" r="10"/>
            <defs><rect width="5" height="5"/></defs></svg>"#;
        let p = read(svg, ProfileFormat::Svg);
        assert_eq!(p.paths.len(), 2, "{:?}", p.paths);
        let sq = &p.paths[0];
        assert!(sq.closed);
        assert!((area(sq).abs() - 200.0).abs() < 1.0e-6);
        // Upright: y down in the file is negative here.
        assert!(sq.points.iter().all(|q| q.y <= 1.0e-9 && q.x >= 10.0 - 1.0e-9));
        assert!((area(&p.paths[1]).abs() - PI * 100.0).abs() < 0.5);
    }

    #[test]
    fn svg_arcs_and_curves_end_where_they_should() {
        let p = path_data("M0,0 A10 10 0 0 1 20 0 c 0 10 10 10 10 0 q5-5 10 0 t10 0z", 0.01);
        assert_eq!(p.len(), 1);
        let pts = &p[0].points;
        // The arc's top (sweep 1 = positive angle in SVG's y-down frame) passes y = -10.
        let lowest = pts.iter().map(|q| q.y).fold(f64::INFINITY, f64::min);
        assert!((lowest + 10.0).abs() < 0.05, "{lowest}");
        assert!((pts.last().map(|q| q.x).unwrap_or(0.0) - 50.0).abs() < 1.0e-9);
    }

    #[test]
    fn selig_and_lednicer_airfoils_read_alike() {
        let selig = "NACA test\n1.0 0.0\n0.5 0.06\n0.0 0.0\n0.5 -0.06\n1.0 0.0\n";
        let led = "NACA test\n3. 3.\n\n0.0 0.0\n0.5 0.06\n1.0 0.0\n\n0.0 0.0\n0.5 -0.06\n1.0 0.0\n";
        let a = read(selig, ProfileFormat::Dat);
        let b = read(led, ProfileFormat::Dat);
        assert_eq!(a.paths[0].points.len(), 4);
        assert_eq!(b.paths[0].points.len(), 4);
        assert!((area(&a.paths[0]).abs() - area(&b.paths[0]).abs()).abs() < 1.0e-12);
    }

    #[test]
    fn hostile_input_is_an_error_not_a_panic() {
        for f in ProfileFormat::ALL {
            for junk in ["", "0\nLINE\n10\nNaN", "<svg><path d='M 1e999 0 L'/></svg>", "x y\n1", "\u{0}\u{1}"] {
                let _ = read_profile(junk.as_bytes(), f, 0.01);
            }
        }
        let _ = path_data("A 1 1 0 1 1", 0.01);
        let _ = path_data("M0 0 A0 0 0 0 0 5 5 L 1e308 1e308", 0.01);
    }
}
