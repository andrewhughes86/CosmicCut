//! Shapes: the 2D outlines cuts follow, imported from drawings or made here (rectangles,
//! circles, polygons, stars, NACA airfoils), in millimetres.

use std::f64::consts::{PI, TAU};

use CosmicCut_geom::offset::signed_area2;
use CosmicCut_geom::region::{self, Shape as Region};
use CosmicCut_io::profile::Path2;
use glam::DVec2;
use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::units::DisplayUnits;

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub id: String,
    pub name: String,
    pub paths: Vec<Path2>,
    /// Where it came from: a file path, or how it was made.
    pub source: String,
    /// The sketch it is drawn from, if it is a sketch; its outlines follow the sketch.
    pub sketch: Option<Box<crate::sketch::Sketch>>,
}

/// Most points a shape keeps.
pub const MAX_POINTS: usize = 500_000;

impl Shape {
    pub fn new(id: String, name: String, paths: Vec<Path2>, source: String) -> EngineResult<Self> {
        let paths: Vec<Path2> =
            paths.into_iter().filter(|p| p.points.len() >= 2 && p.points.iter().all(|q| q.is_finite())).collect();
        if paths.is_empty() {
            return Err(EngineError::InvalidParams("the shape has no outline".into()));
        }
        if paths.iter().map(|p| p.points.len()).sum::<usize>() > MAX_POINTS {
            return Err(EngineError::InvalidParams(format!("the shape has more than {MAX_POINTS} points")));
        }
        Ok(Self { id, name, paths, source, sketch: None })
    }

    /// A shape drawn from `sketch` (which may be empty yet).
    pub fn from_sketch(id: String, name: String, sketch: crate::sketch::Sketch) -> Self {
        let mut shape =
            Self { id, name, paths: Vec::new(), source: format!("Sketch on {}", sketch.plane.label()), sketch: None };
        shape.paths = sketch.paths();
        shape.sketch = Some(Box::new(sketch));
        shape
    }

    /// The outlines again from the sketch, after it changed.
    pub fn redraw(&mut self) {
        if let Some(s) = &self.sketch {
            self.paths = s.paths();
        }
    }

    /// (min, max) of every point.
    pub fn bounds(&self) -> (DVec2, DVec2) {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for p in self.paths.iter().flat_map(|p| &p.points) {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (lo, hi)
    }

    pub fn closed(&self) -> impl Iterator<Item = &Path2> {
        self.paths.iter().filter(|p| p.closed && p.points.len() >= 3)
    }

    pub fn is_closed(&self) -> bool {
        self.paths.iter().all(|p| p.closed)
    }

    pub fn point_count(&self) -> usize {
        self.paths.iter().map(|p| p.points.len()).sum()
    }

    pub fn to_json(&self) -> Value {
        let (lo, hi) = self.bounds();
        json!({
            "id": self.id,
            "name": self.name,
            "source": self.source,
            "paths": self.paths.len(),
            "closed": self.is_closed(),
            "points": self.point_count(),
            "min": [lo.x, lo.y],
            "max": [hi.x, hi.y],
        })
    }

    /// The whole shape, for job files.
    pub fn save_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "source": self.source,
            "sketch": self.sketch.as_ref().map(|s| s.to_json()),
            "paths": self.paths.iter().map(|p| json!({
                "closed": p.closed,
                "points": p.points.iter().map(|q| [q.x, q.y]).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn load_json(v: &Value) -> EngineResult<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let paths = v
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .take(crate::shape::MAX_POINTS)
                    .map(|p| Path2 {
                        closed: p.get("closed").and_then(Value::as_bool).unwrap_or(false),
                        points: p
                            .get("points")
                            .and_then(Value::as_array)
                            .map(|pts| {
                                pts.iter()
                                    .filter_map(|q| {
                                        let q = q.as_array()?;
                                        Some(DVec2::new(q.first()?.as_f64()?, q.get(1)?.as_f64()?))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(sketch) = v.get("sketch").filter(|s| !s.is_null()) {
            return Ok(Self::from_sketch(s("id"), s("name"), crate::sketch::Sketch::from_json(sketch)?));
        }
        Self::new(s("id"), s("name"), paths, s("source"))
    }

    pub fn properties(&self, units: DisplayUnits) -> Vec<(String, String)> {
        let (lo, hi) = self.bounds();
        let size = hi - lo;
        vec![
            ("Size".into(), format!("{} × {}", units.fmt_len(size.x), units.fmt_len(size.y))),
            ("Outlines".into(), format!("{} ({} closed)", self.paths.len(), self.closed().count())),
            ("Points".into(), self.point_count().to_string()),
            ("Area".into(), units.fmt_area(region::area(&self.regions()))),
            ("Source".into(), self.source.clone()),
        ]
        .into_iter()
        .chain(self.sketch.as_ref().map(|s| {
            let free = s.freedom();
            (
                "Sketch".into(),
                if free == 0 { "fully constrained".into() } else { format!("{free} degrees of freedom left") },
            )
        }))
        .collect()
    }

    /// The closed outlines as regions: outlines inside an odd number of others are holes.
    pub fn regions(&self) -> Vec<Region> {
        let loops: Vec<Vec<DVec2>> = self.closed().map(|p| p.points.clone()).collect();
        nest(&loops)
    }

    /// The shape scaled, turned (degrees) and mirrored (in X) about the origin.
    pub fn transformed(&self, scale: f64, rotate: f64, mirror: bool) -> Vec<Path2> {
        let (s, c) = rotate.to_radians().sin_cos();
        self.paths
            .iter()
            .map(|p| Path2 {
                closed: p.closed,
                points: p
                    .points
                    .iter()
                    .map(|q| {
                        let q = if mirror { DVec2::new(-q.x, q.y) } else { *q } * scale;
                        DVec2::new(q.x * c - q.y * s, q.x * s + q.y * c)
                    })
                    .collect(),
            })
            .collect()
    }
}

/// Closed loops (any orientation) as regions, by even-odd nesting: each loop's depth is how many
/// others hold it; even depths are outlines, odd ones holes of the smallest outline around them.
pub fn nest(loops: &[Vec<DVec2>]) -> Vec<Region> {
    let loops: Vec<&Vec<DVec2>> = loops.iter().filter(|l| l.len() >= 3 && signed_area2(l).abs() > 1.0e-12).collect();
    let inside = |p: DVec2, l: &[DVec2]| region::contains(&[vec![l.to_vec()]], p);
    let area = |l: &[DVec2]| signed_area2(l).abs();
    let holders: Vec<Vec<usize>> = loops
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let probe = l.first().copied().unwrap_or_default();
            (0..loops.len()).filter(|&j| j != i && area(loops[j]) > area(l) && inside(probe, loops[j])).collect()
        })
        .collect();
    let mut out: Vec<(usize, Region)> = Vec::new();
    for (i, l) in loops.iter().enumerate() {
        if holders[i].len().is_multiple_of(2) {
            let mut outer = (*l).clone();
            if signed_area2(&outer) < 0.0 {
                outer.reverse();
            }
            out.push((i, vec![outer]));
        }
    }
    for (i, l) in loops.iter().enumerate() {
        if !holders[i].len().is_multiple_of(2) {
            // The smallest outline around it.
            let parent = holders[i]
                .iter()
                .filter(|&&j| holders[j].len().is_multiple_of(2))
                .min_by(|&&a, &&b| area(loops[a]).total_cmp(&area(loops[b])));
            if let Some(slot) = parent.and_then(|p| out.iter_mut().find(|(o, _)| o == p)) {
                let mut hole = (*l).clone();
                if signed_area2(&hole) > 0.0 {
                    hole.reverse();
                }
                slot.1.push(hole);
            }
        }
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// A rectangle `w` × `h` centred on the origin, with corners rounded to `r`.
pub fn rectangle(w: f64, h: f64, r: f64, tol: f64) -> Vec<Path2> {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let (hx, hy) = (w / 2.0, h / 2.0);
    if r <= 0.0 {
        let pts = vec![DVec2::new(-hx, -hy), DVec2::new(hx, -hy), DVec2::new(hx, hy), DVec2::new(-hx, hy)];
        return vec![Path2 { points: pts, closed: true }];
    }
    let n = segments(r, PI / 2.0, tol);
    let mut pts = Vec::new();
    for (c, a0) in [
        (DVec2::new(hx - r, -hy + r), -PI / 2.0),
        (DVec2::new(hx - r, hy - r), 0.0),
        (DVec2::new(-hx + r, hy - r), PI / 2.0),
        (DVec2::new(-hx + r, -hy + r), PI),
    ] {
        for k in 0..=n {
            let a = a0 + PI / 2.0 * k as f64 / n as f64;
            pts.push(c + DVec2::new(a.cos(), a.sin()) * r);
        }
    }
    vec![Path2 { points: pts, closed: true }]
}

/// A circle of diameter `d` centred on the origin.
pub fn circle(d: f64, tol: f64) -> Vec<Path2> {
    let r = d / 2.0;
    let n = segments(r, TAU, tol).max(12);
    let pts = (0..n).map(|k| DVec2::from_angle(TAU * k as f64 / n as f64) * r).collect();
    vec![Path2 { points: pts, closed: true }]
}

/// A regular polygon with `sides`, across corners `d`, one corner straight up.
pub fn polygon(sides: usize, d: f64) -> Vec<Path2> {
    let n = sides.clamp(3, 1000);
    let pts = (0..n).map(|k| DVec2::from_angle(PI / 2.0 + TAU * k as f64 / n as f64) * (d / 2.0)).collect();
    vec![Path2 { points: pts, closed: true }]
}

/// A star with `points` tips on a circle of diameter `outer` and its valleys on `inner`.
pub fn star(points: usize, outer: f64, inner: f64) -> Vec<Path2> {
    let n = points.clamp(2, 500);
    let pts = (0..2 * n)
        .map(|k| {
            let r = if k % 2 == 0 { outer } else { inner } / 2.0;
            DVec2::from_angle(PI / 2.0 + PI * k as f64 / n as f64) * r
        })
        .collect();
    vec![Path2 { points: pts, closed: true }]
}

/// A NACA four-digit airfoil ("2412": 2 % camber at 40 % chord, 12 % thick) of `chord` mm, its
/// leading edge at the origin, with a closed trailing edge and `n` points a side (cosine spaced).
pub fn naca4(code: &str, chord: f64, n: usize) -> EngineResult<Vec<Path2>> {
    let digits: Vec<u32> = code.trim().chars().filter_map(|c| c.to_digit(10)).collect();
    let [m, p, t1, t2] = digits.as_slice() else {
        return Err(EngineError::InvalidParams("a NACA four-digit code has four digits, e.g. 2412".into()));
    };
    let (m, p, t) = (f64::from(*m) / 100.0, f64::from(*p) / 10.0, f64::from(t1 * 10 + t2) / 100.0);
    if t <= 0.0 {
        return Err(EngineError::InvalidParams("the airfoil needs some thickness".into()));
    }
    let n = n.clamp(10, 2000);
    let side = |x: f64, upper: bool| {
        let yt = 5.0 * t * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x.powi(3) - 0.1036 * x.powi(4));
        let (yc, dyc) = if m == 0.0 || p == 0.0 {
            (0.0, 0.0)
        } else if x < p {
            (m / (p * p) * (2.0 * p * x - x * x), 2.0 * m / (p * p) * (p - x))
        } else {
            (m / ((1.0 - p).powi(2)) * (1.0 - 2.0 * p + 2.0 * p * x - x * x), 2.0 * m / ((1.0 - p).powi(2)) * (p - x))
        };
        let th = dyc.atan();
        let s = if upper { 1.0 } else { -1.0 };
        DVec2::new(x - s * yt * th.sin(), yc + s * yt * th.cos()) * chord
    };
    let xs: Vec<f64> = (0..=n).map(|k| 0.5 * (1.0 - (PI * k as f64 / n as f64).cos())).collect();
    // Trailing edge over the top to the nose, then back along the bottom.
    let mut pts: Vec<DVec2> = xs.iter().rev().map(|x| side(*x, true)).collect();
    pts.extend(xs.iter().skip(1).take(n.saturating_sub(1)).map(|x| side(*x, false)));
    Ok(vec![Path2 { points: pts, closed: true }])
}

fn segments(r: f64, sweep: f64, tol: f64) -> usize {
    let step = 2.0 * (1.0 - tol / r.max(tol * 2.0)).clamp(-1.0, 1.0).acos();
    ((sweep / step.max(1.0e-3)).ceil() as usize).clamp(2, 4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_shapes_have_the_right_size() -> EngineResult<()> {
        let r = Shape::new("s".into(), "r".into(), rectangle(100.0, 50.0, 10.0, 0.01), String::new())?;
        let (lo, hi) = r.bounds();
        assert!((hi - lo - DVec2::new(100.0, 50.0)).length() < 1e-9);
        let area = region::area(&r.regions());
        assert!((area - (5000.0 - (4.0 - PI) * 100.0)).abs() < 1.0, "{area}");
        let wing = naca4("0012", 200.0, 60)?;
        let max_t = wing[0].points.iter().map(|p| p.y).fold(0.0, f64::max) * 2.0;
        assert!((max_t - 24.0).abs() < 0.2, "{max_t}");
        assert!(naca4("12", 100.0, 50).is_err());
        assert_eq!(star(5, 100.0, 40.0)[0].points.len(), 10);
        Ok(())
    }

    #[test]
    fn nested_loops_become_outlines_and_holes() {
        let sq = |s: f64| vec![DVec2::new(-s, -s), DVec2::new(s, -s), DVec2::new(s, s), DVec2::new(-s, s)];
        // A frame (outer and hole) with an island in the hole.
        let r = nest(&[sq(30.0), sq(20.0), sq(5.0)]);
        assert_eq!(r.len(), 2);
        let area = region::area(&r);
        assert!((area - (3600.0 - 1600.0 + 100.0)).abs() < 1e-6, "{area}");
        assert!(region::contains(&r, DVec2::new(25.0, 0.0)));
        assert!(!region::contains(&r, DVec2::new(10.0, 0.0)));
        assert!(region::contains(&r, DVec2::ZERO));
    }
}
