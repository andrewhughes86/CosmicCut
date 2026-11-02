//! The job's program: every cut in order, joined by moves clear of the foam. The simulation plays
//! it and the post writes it, so what is simulated is what the machine gets.

use crate::cut::{Cut, Keep, beside};
use crate::machine::Machine;
use crate::op::Operation;
use crate::stock::Stock;
use crate::wire::{Move, MoveKind, WirePath, WirePos, move_time};

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub start: WirePos,
    /// Each move with the cut it belongs to (an index into the job's cuts); None for the moves
    /// between cuts.
    pub moves: Vec<(Option<usize>, Move)>,
    /// When each move ends, seconds from the start.
    pub ends: Vec<f64>,
    /// The regions the cuts keep, each with the move after which it applies and its cut.
    pub keeps: Vec<(usize, usize, Keep)>,
    /// Where the release starts: the last cut, freeing the part from the foam under it.
    pub release_at: Option<usize>,
    /// Where each operation with cuts starts (the move it starts before) and its place in the
    /// job's operations.
    pub op_starts: Vec<(usize, usize)>,
    /// Where the return home starts, after the last cut.
    pub home_at: Option<usize>,
    /// The moves before this one take the wire from home to where the first cut begins; the
    /// wire comes on after them.
    pub approach_end: usize,
    /// The angle the table turns to at the end, if it does.
    pub end_turn: Option<f64>,
}

impl Program {
    /// The program of `cuts` (indices in it are kept), cut operation by operation.
    pub fn build(machine: &Machine, stock: &Stock, cuts: &[Cut], ops: &[Operation]) -> Self {
        let mut prog = Program::default();
        let safe = stock.top() + machine.clearance;
        let reach = stock.reach() + machine.clearance;
        let link = |from: WirePos, to: WirePos| link(stock, from, to, safe, reach);
        let mut at: Option<WirePos> = None;
        let op_of = |c: &Cut| ops.iter().position(|o| o.id == c.op).unwrap_or(usize::MAX);
        let mut order: Vec<usize> = (0..cuts.len()).collect();
        order.sort_by_key(|&i| cuts.get(i).map_or(usize::MAX, op_of));
        let mut current_op: Option<usize> = None;
        for &ci in &order {
            let Some(cut) = cuts.get(ci) else { continue };
            if cut.error.is_some() || cut.path.moves.is_empty() {
                continue;
            }
            let op = op_of(cut);
            if current_op != Some(op) {
                current_op = Some(op);
                if op != usize::MAX {
                    prog.op_starts.push((prog.moves.len(), op));
                }
            }
            // The first cut is reached from home: straight there, or over the stock only when it
            // stands between home and where the cut begins. The cut's own way down from above
            // the foam is left out.
            let mut skip = 0;
            match at {
                None => {
                    prog.start = WirePos::new(-machine.table_x, machine.wire_y0, 0.0);
                    skip = cut
                        .path
                        .moves
                        .iter()
                        .position(|m| matches!(m.kind, MoveKind::Lead | MoveKind::Cut))
                        .unwrap_or(0);
                    let begin = match skip.checked_sub(1) {
                        Some(k) => cut.path.moves.get(k).map_or(cut.path.start, |m| m.to),
                        None => cut.path.start,
                    };
                    for m in link(prog.start, begin) {
                        prog.moves.push((None, m));
                    }
                    prog.approach_end = prog.moves.len();
                    // An operation that starts the job starts at home, before the approach.
                    if let Some(first) = prog.op_starts.first_mut()
                        && first.0 == prog.approach_end
                    {
                        first.0 = 0;
                    }
                }
                Some(from) => {
                    let to = cut.path.start;
                    for m in link(from, to) {
                        prog.moves.push((None, m));
                    }
                }
            }
            let base = prog.moves.len();
            for m in cut.path.moves.iter().skip(skip) {
                prog.moves.push((Some(ci), *m));
            }
            for k in &cut.keeps {
                prog.keeps.push((base + k.at.min(cut.path.moves.len()).saturating_sub(skip), ci, k.clone()));
            }
            at = Some(cut.path.end());
        }
        // Last of all, the part is cut free: the release of the last cut that holds it.
        if let (Some(from), Some((ci, rel))) = (
            at,
            order.iter().rev().find_map(|&ci| {
                let c = cuts.get(ci)?;
                c.release.as_ref().filter(|r| c.error.is_none() && !r.moves.is_empty()).map(|r| (ci, r))
            }),
        ) {
            // Cut from whichever end the wire is beside, so it need not go over the part first.
            let rel =
                if beside(stock, from, rel.start, reach).is_none() && beside(stock, from, rel.end(), reach).is_some() {
                    reversed(rel)
                } else {
                    rel.clone()
                };
            for m in link(from, rel.start) {
                prog.moves.push((None, m));
            }
            prog.release_at = Some(prog.moves.len());
            for m in &rel.moves {
                prog.moves.push((Some(ci), *m));
            }
        }
        // Then the table turned to its end angle (clear of the foam), and home, clear of the part.
        // B0 or B180, whichever is nearer where the last cut left the table.
        let turning =
            machine.post.square_b.then_some(()).filter(|_| machine.b_axis && !prog.moves.is_empty()).and_then(|_| {
                let end = prog.moves.last().map_or(prog.start, |(_, m)| m.to);
                let b = machine.machine_b(end.b);
                let to = if b.abs() <= (b - 180.0).abs() { 0.0 } else { 180.0 };
                ((b - to).abs() > 1.0e-9).then_some(to)
            });
        if (machine.post.return_home || turning.is_some()) && !prog.moves.is_empty() {
            prog.home_at = Some(prog.moves.len());
        }
        if let Some(b) = turning {
            let end = prog.moves.last().map_or(prog.start, |(_, m)| m.to);
            for m in end_turn(stock, machine, end, machine.machine_b(b)) {
                prog.moves.push((None, m));
            }
            prog.end_turn = Some(b);
        }
        if machine.post.return_home && !prog.moves.is_empty() {
            let end = prog.moves.last().map_or(prog.start, |(_, m)| m.to);
            for m in home(machine, stock, end) {
                prog.moves.push((None, m));
            }
        }
        prog.keeps.sort_by_key(|(at, _, _)| *at);
        let mut t = 0.0;
        let mut from = prog.start;
        for (_, m) in &prog.moves {
            t += move_time(&from, m, machine.rapid, machine.b_rate).1;
            prog.ends.push(t);
            from = m.to;
        }
        prog
    }

    pub fn duration(&self) -> f64 {
        self.ends.last().copied().unwrap_or(0.0)
    }

    /// Where move `i` starts.
    pub fn from(&self, i: usize) -> WirePos {
        match i.checked_sub(1) {
            Some(j) => self.moves.get(j).map_or(self.start, |(_, m)| m.to),
            None => self.start,
        }
    }

    /// The wire at `t` seconds: its position and the move it is on.
    pub fn at_time(&self, t: f64) -> (WirePos, usize) {
        if self.moves.is_empty() {
            return (self.start, 0);
        }
        let i = self.ends.partition_point(|end| *end < t).min(self.moves.len() - 1);
        let begin = if i == 0 { 0.0 } else { self.ends.get(i - 1).copied().unwrap_or(0.0) };
        let end = self.ends.get(i).copied().unwrap_or(begin);
        let f = if end > begin { ((t - begin) / (end - begin)).clamp(0.0, 1.0) } else { 1.0 };
        let to = self.moves.get(i).map_or(self.start, |(_, m)| m.to);
        (self.from(i).lerp(&to, f), i)
    }

    /// The start time of move `i`.
    pub fn start_of(&self, i: usize) -> f64 {
        if i == 0 { 0.0 } else { self.ends.get(i - 1).copied().unwrap_or_else(|| self.duration()) }
    }
}

/// From the end of one cut to the start of the next. Beside the foam on the same side, the wire
/// stays there (the table turning past the stock's reach); otherwise up above the foam, the
/// table turning there, across, and down.
///
/// Every move over the stock is made at the safe height: the wire first rises straight up where
/// it is (clear of the foam, or in its own kerf), so it never crosses the part on the way.
fn link(stock: &Stock, from: WirePos, to: WirePos, safe: f64, reach: f64) -> Vec<Move> {
    let rapid = |to: WirePos| Move { kind: MoveKind::Rapid, to, feed: 0.0 };
    if let Some(moves) = beside(stock, from, to, reach) {
        let mut out: Vec<Move> = moves.into_iter().map(|(kind, to)| Move { kind, to, feed: 0.0 }).collect();
        out.retain(|m| m.to.is_finite());
        return out;
    }
    let mut out = Vec::new();
    let mut at = from;
    let across = (at.u - to.u).abs() > 1.0e-9 || (at.b - to.b).abs() > 1.0e-9;
    let high = safe.max(to.h);
    if at.h < high && across {
        at = WirePos::new(at.u, high, at.b);
        out.push(rapid(at));
    }
    if (at.b - to.b).abs() > 1.0e-9 {
        at = WirePos::new(at.u, at.h, to.b);
        out.push(Move { kind: MoveKind::Rotate, to: at, feed: 0.0 });
    }
    if (at.u - to.u).abs() > 1.0e-9 {
        at = WirePos::new(to.u, at.h, to.b);
        out.push(rapid(at));
    }
    out.push(rapid(to));
    out.retain(|m| m.to.is_finite());
    out
}

/// The table turned to `b` at the end: where the wire is, if it is above the foam or out past
/// its turning circle; otherwise up (over the stock) or out to the side first.
fn end_turn(stock: &Stock, machine: &Machine, at: WirePos, b: f64) -> Vec<Move> {
    let reach = stock.reach() + machine.clearance;
    let safe = stock.top() + machine.clearance;
    let (u0, u1) = stock.u_range(at.b);
    let mut out = Vec::new();
    let mut to = at;
    if at.h < safe - 1.0e-6 && at.u.abs() < reach {
        to = if at.u > u0 && at.u < u1 {
            // Over the stock: straight up.
            WirePos::new(at.u, safe, at.b)
        } else {
            // Beside it: out past where the foam swings.
            WirePos::new(if at.u < 0.0 { -reach } else { reach }, at.h, at.b)
        };
        out.push(Move { kind: MoveKind::Rapid, to, feed: 0.0 });
    }
    out.push(Move { kind: MoveKind::Rotate, to: WirePos::new(to.u, to.h, b), feed: 0.0 });
    out
}

/// `path` run backwards: from its end, through the same points, to its start.
fn reversed(path: &WirePath) -> WirePath {
    let mut out = WirePath::new(path.end());
    for i in (0..path.moves.len()).rev() {
        let (Some(m), to) =
            (path.moves.get(i), if i == 0 { Some(path.start) } else { path.moves.get(i - 1).map(|m| m.to) })
        else {
            continue;
        };
        if let Some(to) = to {
            out.push(m.kind, to, m.feed);
        }
    }
    out
}

/// The moves home to machine X0 Y0 from `at`, with the table where it is. When the stock stands
/// between the wire and home, the wire goes up to the top of its travel, across to X0 and down;
/// otherwise straight home.
fn home(machine: &Machine, stock: &Stock, at: WirePos) -> Vec<Move> {
    let rapid = |u: f64, h: f64| Move { kind: MoveKind::Rapid, to: WirePos::new(u, h, at.b), feed: 0.0 };
    let x0 = -machine.table_x;
    let y0 = machine.wire_y0;
    let (u0, _) = stock.u_range(at.b);
    let in_the_way = at.u > u0 - 1.0e-6;
    let mut out = Vec::new();
    if in_the_way {
        let top = machine.travel_y + machine.wire_y0;
        out.push(rapid(at.u, top.max(at.h)));
        out.push(rapid(x0, top.max(at.h)));
    }
    out.push(rapid(x0, y0));
    out
}
