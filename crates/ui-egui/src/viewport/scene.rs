//! What the viewport draws from the job, in the table frame: the turntable, the stock, the cut
//! paths traced across the foam, the machine's towers and wire, and the simulated foam.

use std::f32::consts::TAU;

use CosmicCut_engine::Session;
use CosmicCut_engine::stock::{Stock, StockShape};
use CosmicCut_engine::wire::{MoveKind, WirePath, WirePos, on_wire, wire_dir};
use CosmicCut_geom::Vec3;
use CosmicCut_geom::voxel::Voxels;
use CosmicCut_render::ScenePalette;
use CosmicCut_render::scene::{Rgba, SceneGeometry, Vertex};

use crate::theme::{self, Tokens};

/// Space between rulings along a cut, mm (more on long paths).
const RULING_SPACING: f64 = 25.0;
/// Most rulings drawn for one path.
const MAX_RULINGS: usize = 600;

pub fn colours(tokens: &Tokens) -> (ScenePalette, Rgba, Rgba) {
    (tokens.scene_palette(), theme::linear(tokens.wire), theme::linear(tokens.machine))
}

fn with_alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], a]
}

/// The turntable: a disc under the stock with marks every 15°, and its rim.
pub fn turntable(g: &mut SceneGeometry, radius: f32, colour: Rgba, mark: Rgba) {
    let top = -0.6;
    let thick = (radius * 0.04).clamp(8.0, 30.0);
    g.push_cylinder(Vec3::new(0.0, 0.0, top - thick), radius, thick, 96, colour, None);
    for k in 0..24 {
        let a = TAU * k as f32 / 24.0;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        let inner = if k % 6 == 0 { 0.82 } else { 0.9 };
        g.push_line(d * radius * inner + Vec3::Z * 0.2, d * radius + Vec3::Z * 0.2, mark);
    }
    // B = 0 points across the table, along +X.
    g.push_line(Vec3::new(0.0, 0.0, 0.3), Vec3::new(radius * 0.25, 0.0, 0.3), mark);
}

/// The stock: see-through faces and its outline.
pub fn stock(g: &mut SceneGeometry, s: &Stock, fill: Rgba, edge: Rgba) {
    match s.shape {
        StockShape::Block { .. } => {
            let c = s.corners();
            let (lo, hi) = (s.bottom() as f32, s.top() as f32);
            let p = |i: usize, z: f32| c.get(i % 4).map_or(Vec3::ZERO, |q| Vec3::new(q.x as f32, q.y as f32, z));
            for i in 0..4 {
                g.push_triangle(p(i, lo), p(i + 1, lo), p(i + 1, hi), fill);
                g.push_triangle(p(i, lo), p(i + 1, hi), p(i, hi), fill);
                g.push_line(p(i, lo), p(i + 1, lo), edge);
                g.push_line(p(i, hi), p(i + 1, hi), edge);
                g.push_line(p(i, lo), p(i, hi), edge);
            }
            g.push_triangle(p(0, hi), p(1, hi), p(2, hi), fill);
            g.push_triangle(p(0, hi), p(2, hi), p(3, hi), fill);
            g.bounds.include(p(0, lo));
            g.bounds.include(p(2, hi));
        }
        StockShape::Cylinder { radius, height } => {
            let base = Vec3::new(s.centre.x as f32, s.centre.y as f32, s.bottom() as f32);
            g.push_cylinder(base, radius as f32, height as f32, 72, fill, Some(edge));
        }
    }
}

/// The span of the stock along the wire at angle `b`: where the wire goes in and comes out.
fn faces(s: &Stock, b: f64) -> (f64, f64) {
    let d = wire_dir(b);
    let d2 = glam::DVec2::new(d.x, d.y);
    match s.shape {
        StockShape::Block { .. } => {
            let ts: Vec<f64> = s.corners().iter().map(|p| p.dot(d2)).collect();
            (ts.iter().copied().fold(f64::INFINITY, f64::min), ts.iter().copied().fold(f64::NEG_INFINITY, f64::max))
        }
        StockShape::Cylinder { radius, .. } => {
            let c = s.centre.dot(d2);
            (c - radius, c + radius)
        }
    }
}

/// A wire path, traced where the wire goes in and comes out of the foam, with rulings across
/// the foam along the cuts.
pub fn path(g: &mut SceneGeometry, p: &WirePath, s: &Stock, palette: &ScenePalette, rulings: bool, bold: bool) {
    let margin = 2.0;
    let mut at = p.start;
    let mut since_ruling = f64::INFINITY;
    let total: f64 =
        p.moves.iter().filter(|m| m.kind == MoveKind::Cut).map(|m| m.to.uh().distance(at.uh())).sum::<f64>();
    let spacing = RULING_SPACING.max(total / MAX_RULINGS as f64);
    let ruling = [palette.path_cut[0], palette.path_cut[1], palette.path_cut[2], if bold { 0.45 } else { 0.25 }];
    for m in &p.moves {
        let colour = match m.kind {
            MoveKind::Cut => palette.path_cut,
            MoveKind::Lead => palette.path_lead,
            MoveKind::Rapid => palette.path_rapid,
            MoveKind::Rotate => palette.path_link,
        };
        // Turns are drawn in steps so the arc shows.
        let steps = (((m.to.b - at.b).abs() / 5.0).ceil() as usize).clamp(1, 360);
        for k in 1..=steps {
            let a = at.lerp(&m.to, (k - 1) as f64 / steps as f64);
            let b = at.lerp(&m.to, k as f64 / steps as f64);
            for side in [0, 1] {
                let pick = |q: &WirePos| {
                    let (t0, t1) = faces(s, q.b);
                    let t = if side == 0 { t0 - margin } else { t1 + margin };
                    on_wire(q, t).as_vec3()
                };
                g.push_line(pick(&a), pick(&b), colour);
            }
        }
        if rulings && m.kind == MoveKind::Cut {
            since_ruling += m.to.uh().distance(at.uh());
            if since_ruling >= spacing {
                since_ruling = 0.0;
                let (t0, t1) = faces(s, m.to.b);
                g.push_line(on_wire(&m.to, t0).as_vec3(), on_wire(&m.to, t1).as_vec3(), ruling);
            }
        }
        g.bounds.include(on_wire(&m.to, 0.0).as_vec3());
        at = m.to;
    }
}

/// A rod from `a` to `b`, `r` thick.
pub fn rod(g: &mut SceneGeometry, a: Vec3, b: Vec3, r: f32, colour: Rgba) {
    let axis = (b - a).normalize_or_zero();
    if axis == Vec3::ZERO {
        return;
    }
    let (u, v) = axis.any_orthonormal_pair();
    let n = 10;
    let ring = |i: usize| {
        let t = TAU * i as f32 / n as f32;
        u * t.cos() + v * t.sin()
    };
    for i in 0..n {
        let (d0, d1) = (ring(i), ring(i + 1));
        let (a0, a1, b0, b1) = (a + d0 * r, a + d1 * r, b + d0 * r, b + d1 * r);
        for (p, nn) in [(a0, d0), (a1, d1), (b1, d1), (a0, d0), (b1, d1), (b0, d0)] {
            g.triangles.push(Vertex::lit(p, nn, colour));
        }
    }
}

/// The wire between its towers at `p`, and the towers.
pub fn machine(g: &mut SceneGeometry, session: &Session, p: &WirePos, wire: Rgba, frame: Rgba) {
    let m = &session.job.machine;
    let half = m.wire_span / 2.0;
    let (a, b) = (on_wire(p, -half).as_vec3(), on_wire(p, half).as_vec3());
    let r = (m.wire_span as f32 * 0.0015).clamp(0.8, 3.0);
    rod(g, a, b, r, wire);
    // A faint glow round the hot wire.
    rod(g, a, b, r * 3.0, with_alpha(wire, 0.18));
    // The towers stand at the wire's ends, from the table to the top of the travel.
    let top = (m.wire_y0 + m.travel_y) as f32;
    let post = (m.wire_span as f32 * 0.012).clamp(8.0, 40.0);
    for end in [a, b] {
        let foot = Vec3::new(end.x, end.y, -40.0);
        rod(g, foot, Vec3::new(end.x, end.y, top), post * 0.5, frame);
        // The carriage holding the wire.
        let c = Vec3::new(end.x, end.y, end.z);
        let s = Vec3::splat(post * 0.9);
        g.push_box(&CosmicCut_geom::Aabb::new(c - s, c + s), wire, None);
    }
}

/// The simulated foam as a smooth mesh, cut faces in the accent colour.
pub fn foam(v: &Voxels, stock: Rgba, cut: Rgba, coarse: bool) -> SceneGeometry {
    let [nx, ny, nz] = v.dims();
    let cells = (nx * ny * nz) as f64;
    let step = if coarse && cells > 4.0e5 { 2 } else { 1 };
    let surface = v.surface(step);
    let mut g = SceneGeometry::default();
    g.triangles.reserve(surface.triangles.len() * 3);
    for t in &surface.triangles {
        for &i in t {
            let i = i as usize;
            let (Some(p), Some(n)) = (surface.positions.get(i), surface.normals.get(i)) else { continue };
            let c = if surface.cut.get(i).copied().unwrap_or(false) { cut } else { stock };
            g.triangles.push(Vertex::lit(*p, *n, c));
            g.bounds.include(*p);
        }
    }
    g
}

/// Turns geometry about the table's axis by `b` degrees.
pub fn rotate_z(g: &mut SceneGeometry, b: f64) {
    let (s, c) = (b as f32).to_radians().sin_cos();
    let turn = |v: [f32; 3]| [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
    for v in g.lines.iter_mut().chain(g.triangles.iter_mut()) {
        v.pos = turn(v.pos);
        v.normal = turn(v.normal);
    }
}

/// The ground grid, sized to the scene.
/// The floor grid, level with the top of the turntable (the table frame's Z0): a line every
/// `step` mm (4 in, or 100 mm in metric), the X and Y axes through the turntable's axis.
pub fn grid(around: &CosmicCut_geom::Aabb, palette: &ScenePalette, step: f32) -> SceneGeometry {
    let mut g = SceneGeometry::default();
    let step = if step.is_finite() && step > 1.0 { step } else { 100.0 };
    let reach = if around.is_empty() {
        1000.0
    } else {
        let (s, c) = (around.size(), around.center());
        (s.x.max(s.y) * 0.5 + c.x.abs().max(c.y.abs())) * 1.6
    };
    let half = ((reach / step).ceil() as i32).clamp(2, 200);
    let extent = half as f32 * step;
    for i in -half..=half {
        if i == 0 {
            continue;
        }
        let t = i as f32 * step;
        g.push_line(Vec3::new(t, -extent, 0.0), Vec3::new(t, extent, 0.0), palette.grid_major);
        g.push_line(Vec3::new(-extent, t, 0.0), Vec3::new(extent, t, 0.0), palette.grid_major);
    }
    let fade = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * 0.55];
    g.push_line(Vec3::ZERO, Vec3::new(extent, 0.0, 0.0), fade(palette.axis_x));
    g.push_line(Vec3::ZERO, Vec3::new(0.0, extent, 0.0), fade(palette.axis_y));
    g.push_line(Vec3::new(-extent, 0.0, 0.0), Vec3::ZERO, palette.grid_major);
    g.push_line(Vec3::new(0.0, -extent, 0.0), Vec3::ZERO, palette.grid_major);
    g
}
