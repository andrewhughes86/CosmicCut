//! 3D models: imported meshes (STL, OBJ, STEP, IGES and the rest CosmicDMS reads), laid on the
//! turntable on a face of the user's choosing and always centred on its axis, and the 2D
//! outlines cuts are made from: silhouettes seen along the wire, and the radial envelope for
//! lathe cuts.

use std::sync::Arc;

use CosmicCut_geom::faces::{self, FaceKind, FaceMap};
use CosmicCut_geom::region;
use CosmicCut_geom::{Aabb, Mesh, Quat, Vec3};
use CosmicCut_io::profile::Path2;
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::units::DisplayUnits;
use crate::wire::{u_axis, wire_dir};

#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// The file it was read from.
    pub source: String,
    /// The units it was read in, when the user chose them.
    pub units: Option<String>,
    /// As read, in millimetres.
    raw: Arc<Mesh>,
    /// Its faces, recognised on the mesh as read (triangle indices match the placed mesh).
    pub faces: Arc<FaceMap>,
    /// Which faces share an edge, worked out when first wanted.
    neighbours: Arc<std::sync::OnceLock<Vec<Vec<usize>>>>,
    /// The direction, in the model as read, that points down onto the table.
    pub down: DVec3,
    /// Turn about the table's axis after laying it down, degrees.
    pub rotate: f64,
    pub scale: f64,
    /// Height of its bottom above the stock's bottom, mm.
    pub lift: f64,
    /// Stood on the turntable's axis (for cuts that turn the table); off, it stands at `at`.
    pub centred: bool,
    /// Where its middle stands on the table when it is not centred, mm.
    pub at: glam::DVec2,
    /// Placed in the table frame.
    pub mesh: Arc<Mesh>,
    pub visible: bool,
}

/// Most triangles a model may have for its outlines to be worked out.
pub const MAX_TRIANGLES: usize = 3_000_000;

impl Model {
    pub fn new(id: String, name: String, source: String, units: Option<String>, raw: Mesh) -> EngineResult<Self> {
        if raw.is_empty() {
            return Err(EngineError::Import("the model has no triangles".into()));
        }
        if raw.triangles().len() > MAX_TRIANGLES {
            return Err(EngineError::Import(format!("the model has more than {MAX_TRIANGLES} triangles")));
        }
        let faces = Arc::new(faces::segment(&raw));
        let raw = Arc::new(raw);
        Ok(Self {
            id,
            name,
            source,
            units,
            mesh: Arc::clone(&raw),
            raw,
            faces,
            neighbours: Arc::default(),
            down: DVec3::NEG_Z,
            rotate: 0.0,
            scale: 1.0,
            lift: 0.0,
            centred: true,
            at: glam::DVec2::ZERO,
            visible: true,
        })
    }

    /// Lays the model on the table: `down` pointing down, turned by `rotate`, scaled, centred on
    /// the turntable's axis with its bottom `lift` above `floor`.
    pub fn place(&mut self, floor: f64) {
        let down = self.down.normalize_or(DVec3::NEG_Z).as_vec3();
        let lay = Quat::from_rotation_arc(down, Vec3::NEG_Z);
        let turn = Quat::from_rotation_z((self.rotate as f32).to_radians());
        let q = turn * lay;
        let s = self.scale as f32;
        let turned: Vec<Vec3> = self.raw.positions().iter().map(|p| q * (*p * s)).collect();
        let b = Aabb::from_points(turned.iter().copied());
        let middle = if self.centred { Vec3::ZERO } else { Vec3::new(self.at.x as f32, self.at.y as f32, 0.0) };
        let shift = Vec3::new(
            middle.x - (b.min.x + b.max.x) / 2.0,
            middle.y - (b.min.y + b.max.y) / 2.0,
            (floor + self.lift) as f32 - b.min.z,
        );
        let placed: Vec<Vec3> = turned.into_iter().map(|p| p + shift).collect();
        // A placement that fails (it cannot for finite input) leaves the model where it was.
        if let Ok(m) = Mesh::new(placed, self.raw.triangles().to_vec()) {
            self.mesh = Arc::new(m);
        }
    }

    /// The down direction that lays the face holding `triangle` flat on the table: a flat
    /// face's outward normal, else the triangle's.
    pub fn down_for(&self, triangle: usize) -> Option<DVec3> {
        let normal = match self.faces.face_of(triangle).and_then(|f| self.faces.faces.get(f)).map(|f| f.kind) {
            Some(FaceKind::Plane { normal, .. }) => normal,
            _ => {
                let t = *self.raw.triangles().get(triangle)?;
                self.raw.face_normal(t)
            }
        };
        let n = normal.as_dvec3();
        (n.length_squared() > 0.5).then(|| n.normalize())
    }

    /// The triangles of the face holding `triangle`, for highlighting it.
    pub fn face_triangles(&self, triangle: usize) -> Vec<usize> {
        match self.faces.face_of(triangle) {
            Some(f) => self.faces.triangles_of(f).collect(),
            None => vec![triangle],
        }
    }

    /// The faces that share an edge with each face, by face. Worked out once per model.
    pub fn face_neighbours(&self) -> &[Vec<usize>] {
        self.neighbours.get_or_init(|| {
            let count = self.faces.faces.len();
            let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
            let key =
                |v: Vec3| [(v.x * 1.0e3).round() as i64, (v.y * 1.0e3).round() as i64, (v.z * 1.0e3).round() as i64];
            let tris = self.raw.triangles();
            let mut edges: std::collections::HashMap<([i64; 3], [i64; 3]), usize> =
                std::collections::HashMap::with_capacity(tris.len() * 2);
            for (t, idx) in tris.iter().enumerate() {
                let Some(f) = self.faces.face_of(t) else { continue };
                let [a, b, c] = self.raw.corners(*idx).map(key);
                for (p, q) in [(a, b), (b, c), (c, a)] {
                    let edge = if p < q { (p, q) } else { (q, p) };
                    match edges.get(&edge).copied() {
                        Some(g) if g != f => {
                            for (x, y) in [(f, g), (g, f)] {
                                if let Some(list) = out.get_mut(x)
                                    && !list.contains(&y)
                                {
                                    list.push(y);
                                }
                            }
                        }
                        Some(_) => {}
                        None => {
                            edges.insert(edge, f);
                        }
                    }
                }
            }
            out
        })
    }

    pub fn bounds(&self) -> Aabb {
        self.mesh.bounds()
    }

    pub fn to_json(&self) -> Value {
        let b = self.bounds();
        json!({
            "id": self.id,
            "name": self.name,
            "source": self.source,
            "triangles": self.mesh.triangles().len(),
            "faces": self.faces.faces.len(),
            "min": b.min.to_array(),
            "max": b.max.to_array(),
            "down": [self.down.x, self.down.y, self.down.z],
            "rotate": self.rotate,
            "scale": self.scale,
            "lift": self.lift,
            "centred": self.centred,
            "at": [self.at.x, self.at.y],
            "visible": self.visible,
        })
    }

    /// What a job file keeps: where to read it from and how it is placed.
    pub fn save_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "source": self.source, "units": self.units,
            "down": [self.down.x, self.down.y, self.down.z],
            "rotate": self.rotate, "scale": self.scale, "lift": self.lift, "visible": self.visible,
            "centred": self.centred, "at": [self.at.x, self.at.y],
        })
    }

    pub fn properties(&self, units: DisplayUnits) -> Vec<(String, String)> {
        let b = self.bounds();
        let s = b.size();
        let d = self.down;
        vec![
            ("Size".into(), units.fmt_size([s.x, s.y, s.z])),
            ("Triangles".into(), self.mesh.triangles().len().to_string()),
            ("Faces".into(), self.faces.faces.len().to_string()),
            ("Face down".into(), format!("{:.3}, {:.3}, {:.3}", d.x, d.y, d.z)),
            ("Turned".into(), format!("{:.1}°", self.rotate)),
            ("Scale".into(), format!("{}", self.scale)),
            ("Bottom".into(), units.fmt_len(f64::from(b.min.z))),
            (
                "On the table".into(),
                if self.centred {
                    "centred on the turntable".into()
                } else {
                    format!("at {}, {}", units.fmt_len(self.at.x), units.fmt_len(self.at.y))
                },
            ),
            ("Source".into(), self.source.clone()),
        ]
    }
}

/// The sample penguin (`samples/penguin.stl`, made by `tools/penguin.py`), built into the program
/// so the sample job opens anywhere. Jobs refer to it by this source name.
pub const SAMPLE_PENGUIN: &str = "sample:penguin.stl";
const PENGUIN_STL: &[u8] = include_bytes!("../../../samples/penguin.stl");

/// Reads a model file into millimetres.
pub fn read(path: &str, units: Option<&str>) -> EngineResult<(Mesh, Vec<String>)> {
    if path == SAMPLE_PENGUIN {
        let mesh = CosmicCut_io::read_mesh(PENGUIN_STL, CosmicCut_io::ModelFormat::Stl)
            .map_err(|e| EngineError::Import(e.to_string()))?;
        return Ok((mesh, Vec::new()));
    }
    let units = units.and_then(CosmicCut_io::Units::from_id);
    let (imported, _) = CosmicCut_io::import_mesh(std::path::Path::new(path), units, CosmicCut_io::Options::default())
        .map_err(|e| EngineError::Import(e.to_string()))?;
    Ok((imported.mesh, imported.warnings))
}

/// Douglas–Peucker: the points of `pts` (a closed loop) within `tol` of it.
fn simplify(pts: &[DVec2], tol: f64) -> Vec<DVec2> {
    if pts.len() < 4 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    // Split the loop at its first point and the point farthest from it.
    let first = pts.first().copied().unwrap_or_default();
    let far = (0..pts.len())
        .max_by(|a, b| pts[*a].distance_squared(first).total_cmp(&pts[*b].distance_squared(first)))
        .unwrap_or(0);
    keep[0] = true;
    keep[far] = true;
    let mut stack = vec![(0usize, far), (far, pts.len())];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let pa = pts[a];
        let pb = pts[b % pts.len()];
        let d = pb - pa;
        let l2 = d.length_squared();
        let mut best = (0usize, 0.0f64);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let t = if l2 > 0.0 { ((*p - pa).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let dist = (pa + d * t).distance(*p);
            if dist > best.1 {
                best = (i, dist);
            }
        }
        if best.1 > tol {
            keep[best.0] = true;
            stack.push((a, best.0));
            stack.push((best.0, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// The model's outline seen along the wire with the table at `b` degrees, in the cut plane
/// (u across, h up): the outer outlines, and with `holes` the openings that go right through.
pub fn silhouette(mesh: &Mesh, b: f64, holes: bool, tol: f64) -> Vec<Path2> {
    let (ua, wd) = (u_axis(b).as_vec3(), wire_dir(b).as_vec3());
    let to_plane = |p: Vec3| DVec2::new(f64::from(p.dot(ua)), f64::from(p.z));
    // Every triangle facing the viewer covers the silhouette of a closed mesh; the others are
    // added only when the mesh is open, so a sheet still casts its outline.
    let tris: Vec<Vec<DVec2>> = mesh
        .triangles()
        .iter()
        .filter_map(|t| {
            let c = mesh.corners(*t);
            let n = (c[1] - c[0]).cross(c[2] - c[0]);
            (n.dot(wd).abs() > 1.0e-9).then(|| c.iter().map(|p| to_plane(*p)).collect())
        })
        .collect();
    let shapes = region::union_polygons(&tris);
    let mut out = Vec::new();
    for shape in shapes {
        for (i, contour) in shape.into_iter().enumerate() {
            if i > 0 && !holes {
                break;
            }
            let pts = simplify(&contour, tol);
            if pts.len() >= 3 {
                out.push(Path2 { points: pts, closed: true });
            }
        }
    }
    out
}

/// The model's radial envelope about the turntable's axis: at each height the farthest it
/// reaches, as an open (radius, height) profile from the top down, closed to the axis at the top.
pub fn radial_profile(mesh: &Mesh, samples: usize) -> Vec<DVec2> {
    let b = mesh.bounds();
    let (z0, z1) = (f64::from(b.min.z), f64::from(b.max.z));
    let n = samples.clamp(8, 4000);
    let dz = (z1 - z0) / n as f64;
    if dz.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Vec::new();
    }
    let mut r = vec![0.0f64; n + 1];
    let radius = |p: DVec3| DVec2::new(p.x, p.y).length();
    for t in mesh.triangles() {
        let c = mesh.corners(*t).map(|p| p.as_dvec3());
        let lo = c.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
        let hi = c.iter().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        let k0 = ((lo - z0) / dz).ceil().max(0.0) as usize;
        let k1 = (((hi - z0) / dz).floor().max(0.0) as usize).min(n);
        for k in k0..=k1 {
            let z = z0 + k as f64 * dz;
            // Where the plane at z cuts the triangle's edges; the farthest from the axis is on one.
            let mut far = 0.0f64;
            for (a, bb) in [(c[0], c[1]), (c[1], c[2]), (c[2], c[0])] {
                if (a.z - z) * (bb.z - z) <= 0.0 {
                    let p = if (bb.z - a.z).abs() < 1.0e-12 { a } else { a + (bb - a) * ((z - a.z) / (bb.z - a.z)) };
                    far = far.max(radius(p));
                    if (bb.z - a.z).abs() < 1.0e-12 {
                        far = far.max(radius(bb));
                    }
                }
            }
            if let Some(slot) = r.get_mut(k) {
                *slot = slot.max(far);
            }
        }
    }
    let mut out = vec![DVec2::new(0.0, z1)];
    out.extend((0..=n).rev().map(|k| DVec2::new(r[k], z0 + k as f64 * dz)));
    let open = Path2 { points: out, closed: false };
    // Fewer points where the envelope runs straight.
    simplify_open(&open.points, 0.02)
}

fn simplify_open(pts: &[DVec2], tol: f64) -> Vec<DVec2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let d = pb - pa;
        let l2 = d.length_squared();
        let mut best = (0usize, 0.0f64);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let t = if l2 > 0.0 { ((*p - pa).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let dist = (pa + d * t).distance(*p);
            if dist > best.1 {
                best = (i, dist);
            }
        }
        if best.1 > tol {
            keep[best.0] = true;
            stack.push((a, best.0));
            stack.push((best.0, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Cuts paths at `floor`: only what lies above it, for outlines whose bottom is lower than the
/// wire can reach. Closed outlines are clipped as regions.
pub fn clip_above(paths: Vec<Path2>, floor: f64) -> Vec<Path2> {
    let loops: Vec<Vec<DVec2>> = paths.iter().filter(|p| p.closed).map(|p| p.points.clone()).collect();
    if loops.is_empty() {
        return paths;
    }
    let shapes = crate::shape::nest(&loops);
    let keep = vec![region::rectangle(DVec2::new(-1.0e6, floor), DVec2::new(1.0e6, 1.0e6))];
    region::boolean(&shapes, &keep, region::Op::Intersect)
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| Path2 { points: c, closed: true })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from `lo` to `hi`.
    fn cube(lo: Vec3, hi: Vec3) -> EngineResult<Mesh> {
        let c = |i: usize| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        };
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [1, 3, 7, 5], [0, 4, 6, 2]];
        let soup = quads.iter().flat_map(|q| [[c(q[0]), c(q[1]), c(q[2])], [c(q[0]), c(q[2]), c(q[3])]]);
        Mesh::from_soup(soup).map_err(|e| EngineError::Internal(e.to_string()))
    }

    #[test]
    fn a_model_lies_on_the_face_chosen_and_stays_centred() -> EngineResult<()> {
        let mut m = Model::new(
            "model.1".into(),
            "box".into(),
            String::new(),
            None,
            cube(Vec3::ZERO, Vec3::new(100.0, 40.0, 20.0))?,
        )?;
        m.place(10.0);
        let b = m.bounds();
        assert!((b.min.z - 10.0).abs() < 1e-3 && (b.max.z - 30.0).abs() < 1e-3);
        assert!((b.min.x + 50.0).abs() < 1e-3 && (b.min.y + 20.0).abs() < 1e-3);
        // Stand it on its 40 × 20 end: the +X face down.
        m.down = DVec3::X;
        m.place(10.0);
        let b = m.bounds();
        assert!((b.max.z - b.min.z - 100.0).abs() < 1e-3, "{b:?}");
        assert!((b.min.z - 10.0).abs() < 1e-3 && (b.min.x + b.max.x).abs() < 1e-3);
        Ok(())
    }

    #[test]
    fn silhouettes_and_envelopes_have_the_model_s_size() -> EngineResult<()> {
        let mut m = Model::new(
            "model.1".into(),
            "box".into(),
            String::new(),
            None,
            cube(Vec3::ZERO, Vec3::new(100.0, 40.0, 20.0))?,
        )?;
        m.place(0.0);
        let front = silhouette(&m.mesh, 0.0, false, 0.01);
        assert_eq!(front.len(), 1);
        let area = CosmicCut_geom::offset::signed_area2(&front[0].points).abs() / 2.0;
        assert!((area - 2000.0).abs() < 1.0, "{area}");
        let side = silhouette(&m.mesh, 90.0, false, 0.01);
        let area = CosmicCut_geom::offset::signed_area2(&side[0].points).abs() / 2.0;
        assert!((area - 800.0).abs() < 1.0, "{area}");
        let env = radial_profile(&m.mesh, 50);
        let r = env.iter().map(|p| p.x).fold(0.0, f64::max);
        assert!((r - (50f64.powi(2) + 20f64.powi(2)).sqrt()).abs() < 0.1, "{r}");
        assert!(env.first().is_some_and(|p| p.x == 0.0 && (p.y - 20.0).abs() < 1e-3));
        let clipped = clip_above(front, 5.0);
        let area = CosmicCut_geom::offset::signed_area2(&clipped[0].points).abs() / 2.0;
        assert!((area - 1500.0).abs() < 1.0, "{area}");
        Ok(())
    }
}
