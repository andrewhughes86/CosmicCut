//! Simulation: the program played back, with the foam as voxels. When a pass finishes, the
//! waste it cut free falls away: the foam is intersected with the region the pass keeps.

use CosmicCut_geom::region::Shape as Region;
use CosmicCut_geom::voxel::Voxels;
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use crate::cut::{Cut, Keep};
use crate::machine::Machine;
use crate::program::Program;
use crate::stock::{Stock, StockShape};
use crate::wire::{WirePos, pos_json, u_axis};

/// Voxel cells the foam is simulated with, at most.
pub const MAX_CELLS: usize = 1_500_000;
/// Cells of a 2D region field, at most.
const MAX_FIELD: usize = 4_000_000;

pub struct Sim {
    pub program: Program,
    pub time: f64,
    pub playing: bool,
    /// Playback speed: program seconds per real second.
    pub speed: f64,
    pub voxels: Option<Voxels>,
    stock: Stock,
    /// Keeps applied to the voxels so far.
    applied: usize,
    /// The cells that are part of a model: a piece of foam holding none of them is waste.
    parts: Vec<bool>,
    /// Each model's id and the cells inside it.
    model_cells: Vec<(String, Vec<usize>)>,
    /// For each keep, the models off the table by then (its operation's and its cut's).
    off_table: Vec<Vec<String>>,
    /// The cells of pieces taken off the table: later cuts do not touch them.
    set_aside: Vec<usize>,
    /// The same, cell by cell.
    aside: Vec<bool>,
    /// Cuts the program came from, by index: their names.
    pub cut_names: Vec<String>,
    /// Where the machine stops for the operator: (seconds, what to tell them).
    pub stops: Vec<(f64, String)>,
    /// The stop the simulation is waiting at, if any.
    pub notice: Option<String>,
}

pub const SPEEDS: &[f64] = &[1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 1000.0];

impl Sim {
    pub fn new(
        machine: &Machine,
        stock: &Stock,
        cuts: &[Cut],
        ops: &[crate::op::Operation],
        models: &[crate::model::Model],
    ) -> Self {
        let program = Program::build(machine, stock, cuts, ops);
        let stops: Vec<(f64, String)> = program
            .op_starts
            .iter()
            .filter_map(|(at, o)| {
                let op = ops.get(*o).filter(|op| op.pauses())?;
                let what = if op.message.trim().is_empty() {
                    format!("{}: {}", op.title(*o), op.stop.label())
                } else {
                    format!("{}: {}", op.title(*o), op.message.trim())
                };
                Some((program.start_of(*at), what))
            })
            .collect();
        // Fast enough to watch a long job: about a minute of playback by default.
        let wanted = program.duration() / 60.0;
        let speed = SPEEDS.iter().copied().find(|s| *s >= wanted).unwrap_or(1000.0);
        let voxels = foam(stock);
        let parts = voxels.as_ref().map(|v| part_cells(v, models)).unwrap_or_default();
        let model_cells: Vec<(String, Vec<usize>)> = voxels
            .as_ref()
            .map(|v| {
                models
                    .iter()
                    .map(|m| {
                        let cells = part_cells(v, std::slice::from_ref(m));
                        (m.id.clone(), cells.iter().enumerate().filter(|(_, c)| **c).map(|(n, _)| n).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let off_table: Vec<Vec<String>> = program
            .keeps
            .iter()
            .map(|(_, owner, _)| {
                let Some(cut) = cuts.get(*owner) else { return Vec::new() };
                let mut off: Vec<String> =
                    ops.iter().find(|o| o.id == cut.op).map(|o| o.ignore.clone()).unwrap_or_default();
                off.extend(
                    cut.params
                        .get("ignore")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
                off
            })
            .collect();
        Self {
            program,
            time: 0.0,
            playing: false,
            speed,
            voxels,
            parts,
            model_cells,
            off_table,
            set_aside: Vec::new(),
            aside: Vec::new(),
            stock: *stock,
            applied: 0,
            cut_names: cuts.iter().map(|c| c.name.clone()).collect(),
            notice: stops.iter().find(|(t, _)| *t <= 0.0).map(|(_, m)| m.clone()),
            stops,
        }
    }

    pub fn duration(&self) -> f64 {
        self.program.duration()
    }

    /// Advances the clock while playing.
    pub fn tick(&mut self, dt: f64) {
        if !self.playing || !dt.is_finite() {
            return;
        }
        let mut t = self.time + dt.clamp(0.0, 0.25) * self.speed;
        if t >= self.duration() {
            self.playing = false;
        }
        // The machine stops for the operator: so does the simulation, until Play again.
        if let Some((at, what)) = self.stops.iter().find(|(at, _)| *at > self.time + 1.0e-9 && *at <= t) {
            t = *at;
            self.playing = false;
            self.notice = Some(what.clone());
        } else if t > self.time {
            self.notice = None;
        }
        self.seek(t);
    }

    /// Jumps to `t` seconds, cutting the foam up to there (going back starts the foam over).
    pub fn seek(&mut self, t: f64) {
        let t = if t.is_finite() { t.clamp(0.0, self.duration()) } else { 0.0 };
        if t < self.time - 1.0e-9 {
            self.voxels = foam(&self.stock);
            self.applied = 0;
            self.set_aside.clear();
            self.aside.clear();
        }
        self.time = t;
        let (_, i) = self.program.at_time(t);
        let at_end = t >= self.duration() - 1.0e-9;
        // Keeps whose move has finished.
        while let Some((after, _, keep)) = self.program.keeps.get(self.applied) {
            let done = if at_end {
                true
            } else {
                *after <= i && self.program.ends.get(after.saturating_sub(1)).is_some_and(|e| *e <= t)
            };
            if !done {
                break;
            }
            if let Some(v) = &mut self.voxels {
                // Models off the table by this cut: their pieces are set aside, out of its way.
                let off = self.off_table.get(self.applied).cloned().unwrap_or_default();
                let seeds: Vec<usize> = self
                    .model_cells
                    .iter()
                    .filter(|(id, _)| off.contains(id))
                    .flat_map(|(_, cells)| cells.iter().copied())
                    .collect();
                if !seeds.is_empty() {
                    let [nx, ny, nz] = v.dims();
                    self.aside.resize(nx * ny * nz, false);
                    for n in v.piece_of(&seeds) {
                        if let Some(a) = self.aside.get_mut(n)
                            && !*a
                        {
                            *a = true;
                            self.set_aside.push(n);
                        }
                    }
                }
                let saved = v.values_of(&self.set_aside);
                apply(v, keep);
                // Foam cut free of every part is waste: it falls away.
                v.drop_loose(&self.parts);
                v.restore(&self.set_aside, &saved);
            }
            self.applied += 1;
        }
    }

    pub fn wire(&self) -> WirePos {
        self.program.at_time(self.time).0
    }

    /// The move the wire is on.
    pub fn move_index(&self) -> usize {
        self.program.at_time(self.time).1
    }

    /// The cut the wire is in, if any.
    pub fn current_cut(&self) -> Option<&str> {
        let i = self.move_index();
        self.program.moves.get(i).and_then(|(c, _)| *c).and_then(|c| self.cut_names.get(c)).map(String::as_str)
    }

    /// Jumps to the start of the next (or previous) cut.
    pub fn jump_cut(&mut self, forward: bool) {
        let i = self.move_index();
        let cut_of = |k: usize| self.program.moves.get(k).and_then(|(c, _)| *c);
        let here = cut_of(i);
        let n = self.program.moves.len();
        let target = if forward {
            (i + 1..n).find(|&k| cut_of(k).is_some() && cut_of(k) != here)
        } else {
            // The start of this cut, or of the one before if already there.
            let start_of = |k: usize| {
                let c = cut_of(k);
                let mut s = k;
                while s > 0 && cut_of(s - 1) == c {
                    s -= 1;
                }
                s
            };
            let s = start_of(i);
            if s < i || self.time > self.program.start_of(s) + 1.0e-6 {
                Some(s)
            } else {
                (0..s).rev().find(|&k| cut_of(k).is_some()).map(start_of)
            }
        };
        match target {
            Some(k) => self.seek(self.program.start_of(k)),
            None if forward => self.seek(self.duration()),
            None => self.seek(0.0),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "time": self.time,
            "duration": self.duration(),
            "playing": self.playing,
            "speed": self.speed,
            "move": self.move_index(),
            "moves": self.program.moves.len(),
            "wire": pos_json(&self.wire()),
            "cut": self.current_cut(),
            "notice": self.notice,
            "foam_volume": self.voxels.as_ref().map(Voxels::volume),
        })
    }
}

/// The cells inside the models (and those their corners are in, so a part thinner than a cell
/// still counts).
pub fn part_cells(v: &Voxels, models: &[crate::model::Model]) -> Vec<bool> {
    let tris: Vec<[DVec3; 3]> = models
        .iter()
        .flat_map(|m| m.mesh.triangles().iter().map(|t| m.mesh.corners(*t).map(|p| p.as_dvec3())))
        .collect();
    let mut cells = v.part_cells(&tris);
    let [nx, ny, nz] = v.dims();
    let (o, cell) = (v.origin(), v.cell());
    for p in tris.iter().flatten() {
        let q = (*p - o) / cell;
        if q.min_element() < 0.0 {
            continue;
        }
        let (i, j, k) = (q.x as usize, q.y as usize, q.z as usize);
        if i < nx
            && j < ny
            && k < nz
            && let Some(c) = cells.get_mut((k * ny + j) * nx + i)
        {
            *c = true;
        }
    }
    cells
}

/// The stock as voxels, about [`MAX_CELLS`] of them.
pub fn foam(stock: &Stock) -> Option<Voxels> {
    let b = stock.bounds();
    let (lo, hi) = (b.min.as_dvec3(), b.max.as_dvec3());
    let size = hi - lo;
    let cell = (size.x * size.y * size.z / MAX_CELLS as f64).cbrt().max(0.5);
    // Rounding up the cell counts can tip it over the limit; nudge until it fits.
    let mut v = None;
    for grow in [1.0, 1.05, 1.1, 1.2, 1.4] {
        v = Voxels::new(lo, hi, cell * grow, MAX_CELLS);
        if v.is_some() {
            break;
        }
    }
    let mut v = v?;
    let square = matches!(stock.shape, StockShape::Block { .. }) && (stock.rotation % 90.0).abs() < 1.0e-9;
    if !square {
        v.intersect(|p| stock.inside(p));
    }
    Some(v)
}

/// Keeps only the foam inside `keep`'s region.
pub fn apply(v: &mut Voxels, keep: &Keep) {
    if let Some(sweep) = &keep.sweep {
        v.intersect(|p| sweep.inside(p));
        return;
    }
    let cell = v.cell();
    let band = cell * 2.5;
    // The cut plane's extent over the voxels at this angle.
    let [nx, ny, nz] = v.dims();
    let o = v.origin();
    let far = o + DVec3::new(nx as f64, ny as f64, nz as f64) * cell;
    let ax = u_axis(keep.b);
    let corners = [
        DVec3::new(o.x, o.y, 0.0),
        DVec3::new(far.x, o.y, 0.0),
        DVec3::new(far.x, far.y, 0.0),
        DVec3::new(o.x, far.y, 0.0),
    ];
    let us: Vec<f64> = corners.iter().map(|c| c.dot(ax)).collect();
    let lo = DVec2::new(us.iter().copied().fold(f64::INFINITY, f64::min), o.z) - band * 2.0;
    let hi = DVec2::new(us.iter().copied().fold(f64::NEG_INFINITY, f64::max), far.z) + band * 2.0;
    // Pieces kept apart (parts cut free of each other) stay apart: a cell whose centre is within a
    // cell of another piece goes, even where the gap between them is narrower than a cell.
    if keep.apart && !keep.outside && keep.regions.len() > 1 && keep.regions.len() <= MAX_PIECES {
        let fields: Vec<Field2> =
            keep.regions.iter().filter_map(|r| Field2::new(std::slice::from_ref(r), lo, hi, cell, band)).collect();
        if fields.len() == keep.regions.len() {
            v.intersect(|p| {
                let q = DVec2::new(p.dot(ax), p.z);
                let d: Vec<f64> = fields.iter().map(|f| f.sample(q)).collect();
                let Some((best, inside)) = d.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)) else {
                    return -1.0;
                };
                let crowded = d.iter().enumerate().any(|(k, dk)| k != best && *dk > -cell);
                if inside > 0.0 && crowded { -1.0 } else { inside }
            });
            return;
        }
    }
    let Some(field) = Field2::new(&keep.regions, lo, hi, cell, band) else { return };
    let outside = keep.outside;
    v.intersect(|p| {
        let d = field.sample(DVec2::new(p.dot(ax), p.z));
        if outside { -d } else { d }
    });
}

/// Most separate pieces a keep is simulated with one by one.
const MAX_PIECES: usize = 64;

/// Signed distance to a set of regions on a grid (positive inside), exact within `band` of the
/// edges and ±`band` beyond.
pub struct Field2 {
    lo: DVec2,
    cell: f64,
    nx: usize,
    ny: usize,
    band: f64,
    values: Vec<f32>,
}

impl Field2 {
    pub fn new(regions: &[Region], lo: DVec2, hi: DVec2, cell: f64, band: f64) -> Option<Self> {
        if !(lo.is_finite() && hi.is_finite() && cell > 0.0) {
            return None;
        }
        let mut cell = cell;
        let size = hi - lo;
        while ((size.x / cell).ceil() * (size.y / cell).ceil()) > MAX_FIELD as f64 {
            cell *= 1.5;
        }
        let (nx, ny) = (((size.x / cell).ceil() as usize).max(1), ((size.y / cell).ceil() as usize).max(1));
        let centre = |i: usize, j: usize| lo + (DVec2::new(i as f64, j as f64) + 0.5) * cell;
        let edges: Vec<(DVec2, DVec2)> = regions
            .iter()
            .flatten()
            .flat_map(|c| (0..c.len()).filter_map(move |k| Some((*c.get(k)?, *c.get((k + 1) % c.len())?))))
            .collect();
        // Inside or out, by even-odd crossings along each row.
        let mut values = vec![-(band as f32); nx * ny];
        let mut xs: Vec<f64> = Vec::new();
        for j in 0..ny {
            let y = centre(0, j).y;
            xs.clear();
            for (a, b) in &edges {
                if (a.y > y) != (b.y > y) {
                    xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.as_chunks::<2>().0 {
                let i0 = (((pair[0] - lo.x) / cell - 0.5).ceil().max(0.0)) as usize;
                let i1 = (((pair[1] - lo.x) / cell - 0.5).floor().min(nx as f64 - 1.0)) as isize;
                for i in i0..=(i1.max(-1) as usize).min(nx.saturating_sub(1)) {
                    if (i as isize) <= i1
                        && let Some(slot) = values.get_mut(j * nx + i)
                    {
                        *slot = band as f32;
                    }
                }
            }
        }
        // Exact distances near the edges.
        for (a, b) in &edges {
            let (emin, emax) = (a.min(*b) - band, a.max(*b) + band);
            let i0 = (((emin.x - lo.x) / cell).floor().max(0.0)) as usize;
            let j0 = (((emin.y - lo.y) / cell).floor().max(0.0)) as usize;
            let i1 = (((emax.x - lo.x) / cell).ceil().max(0.0) as usize).min(nx);
            let j1 = (((emax.y - lo.y) / cell).ceil().max(0.0) as usize).min(ny);
            let d = *b - *a;
            let l2 = d.length_squared();
            for j in j0..j1 {
                for i in i0..i1 {
                    let p = centre(i, j);
                    let t = if l2 > 1.0e-24 { ((p - *a).dot(d) / l2).clamp(0.0, 1.0) } else { 0.0 };
                    let dist = (*a + d * t).distance(p) as f32;
                    if let Some(slot) = values.get_mut(j * nx + i)
                        && dist < slot.abs()
                    {
                        *slot = dist.copysign(*slot);
                    }
                }
            }
        }
        Some(Self { lo, cell, nx, ny, band, values })
    }

    /// The distance at `p`, interpolated; far outside off the grid.
    pub fn sample(&self, p: DVec2) -> f64 {
        let g = (p - self.lo) / self.cell - 0.5;
        if !(g.x >= -0.5 && g.y >= -0.5 && g.x <= self.nx as f64 - 0.5 && g.y <= self.ny as f64 - 0.5) {
            return -self.band;
        }
        let (i0, j0) = (g.x.floor().max(0.0) as usize, g.y.floor().max(0.0) as usize);
        let (i1, j1) = ((i0 + 1).min(self.nx - 1), (j0 + 1).min(self.ny - 1));
        let (fx, fy) = ((g.x - i0 as f64).clamp(0.0, 1.0), (g.y - j0 as f64).clamp(0.0, 1.0));
        let v =
            |i: usize, j: usize| f64::from(self.values.get(j * self.nx + i).copied().unwrap_or(-(self.band as f32)));
        let a = v(i0, j0) * (1.0 - fx) + v(i1, j0) * fx;
        let b = v(i0, j1) * (1.0 - fx) + v(i1, j1) * fx;
        a * (1.0 - fy) + b * fy
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;
    use CosmicCut_geom::region;

    #[test]
    fn the_field_is_signed_and_close_to_the_edge() {
        let sq = region::rectangle(DVec2::new(-10.0, -10.0), DVec2::new(10.0, 10.0));
        let f = Field2::new(&[sq], DVec2::splat(-50.0), DVec2::splat(50.0), 1.0, 3.0);
        let Some(f) = f else { panic!("no field") };
        assert!((f.sample(DVec2::ZERO) - 3.0).abs() < 1e-6);
        assert!((f.sample(DVec2::new(9.0, 0.0)) - 1.0).abs() < 0.1);
        assert!((f.sample(DVec2::new(11.5, 0.0)) + 1.5).abs() < 0.1);
        assert!(f.sample(DVec2::new(40.0, 0.0)) < 0.0);
        assert!(f.sample(DVec2::new(1.0e6, 0.0)) < 0.0);
    }
}
