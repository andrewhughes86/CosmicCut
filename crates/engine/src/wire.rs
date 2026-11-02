//! Wire positions, moves and paths, and where the wire lies in the table frame.

use glam::{DVec2, DVec3};
use serde_json::{Value, json};

/// Where the wire is: `u` across the table from the turntable axis (mm), `h` above the table
/// top (mm) and the turntable at `b` degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WirePos {
    pub u: f64,
    pub h: f64,
    pub b: f64,
}

impl WirePos {
    pub const fn new(u: f64, h: f64, b: f64) -> Self {
        Self { u, h, b }
    }

    /// The (u, h) point in the cut plane.
    pub fn uh(&self) -> DVec2 {
        DVec2::new(self.u, self.h)
    }

    pub fn is_finite(&self) -> bool {
        self.u.is_finite() && self.h.is_finite() && self.b.is_finite()
    }

    pub fn lerp(&self, to: &Self, t: f64) -> Self {
        Self { u: self.u + (to.u - self.u) * t, h: self.h + (to.h - self.h) * t, b: self.b + (to.b - self.b) * t }
    }
}

/// What a move is for, as the viewport colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    /// Through air, at the rapid rate.
    Rapid,
    /// Cutting foam at the cut's feed.
    Cut,
    /// Into and out of the foam at the cut's feed.
    Lead,
    /// The turntable turning (the wire may move with it).
    Rotate,
}

impl MoveKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Rapid => "rapid",
            Self::Cut => "cut",
            Self::Lead => "lead",
            Self::Rotate => "rotate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Move {
    pub kind: MoveKind,
    pub to: WirePos,
    /// mm/min along (u, h); 0 for rapids, which go at the machine's rapid rate.
    pub feed: f64,
}

/// A wire path: where it starts and the moves from there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WirePath {
    pub start: WirePos,
    pub moves: Vec<Move>,
}

/// Most moves one path may hold.
pub const MAX_MOVES: usize = 2_000_000;

impl WirePath {
    pub fn new(start: WirePos) -> Self {
        Self { start, moves: Vec::new() }
    }

    /// Where the path ends.
    pub fn end(&self) -> WirePos {
        self.moves.last().map_or(self.start, |m| m.to)
    }

    /// Adds a move unless it goes nowhere or the path is full.
    pub fn push(&mut self, kind: MoveKind, to: WirePos, feed: f64) {
        let from = self.end();
        if !to.is_finite() || self.moves.len() >= MAX_MOVES {
            return;
        }
        if from.uh().distance_squared(to.uh()) < 1.0e-12 && (from.b - to.b).abs() < 1.0e-9 {
            return;
        }
        self.moves.push(Move { kind, to, feed });
    }

    pub fn extend(&mut self, other: &WirePath) {
        for m in &other.moves {
            self.push(m.kind, m.to, m.feed);
        }
    }

    /// (cut length mm, rapid length mm, seconds) at `rapid` mm/min and `b_rate` degrees/min.
    pub fn stats(&self, rapid: f64, b_rate: f64) -> (f64, f64, f64) {
        let (mut cut, mut air, mut secs) = (0.0, 0.0, 0.0);
        let mut at = self.start;
        for m in &self.moves {
            let (d, t) = move_time(&at, m, rapid, b_rate);
            if m.kind == MoveKind::Rapid {
                air += d;
            } else {
                cut += d;
            }
            secs += t;
            at = m.to;
        }
        (cut, air, secs)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "start": pos_json(&self.start),
            "moves": self.moves.iter().map(|m| json!({
                "kind": m.kind.id(), "u": m.to.u, "h": m.to.h, "b": m.to.b, "feed": m.feed,
            })).collect::<Vec<_>>(),
        })
    }
}

/// (distance in the cut plane, seconds) of `m` starting from `from`.
pub fn move_time(from: &WirePos, m: &Move, rapid: f64, b_rate: f64) -> (f64, f64) {
    let d = from.uh().distance(m.to.uh());
    let rate = if m.kind == MoveKind::Rapid || m.feed <= 0.0 { rapid } else { m.feed };
    let linear = if rate > 0.0 { d / rate * 60.0 } else { 0.0 };
    let turn = if b_rate > 0.0 { (m.to.b - from.b).abs() / b_rate * 60.0 } else { 0.0 };
    (d, linear.max(turn))
}

pub fn pos_json(p: &WirePos) -> Value {
    json!({ "u": p.u, "h": p.h, "b": p.b })
}

/// The direction across the cut plane (increasing `u`) in the table frame at `b` degrees.
pub fn u_axis(b: f64) -> DVec3 {
    let (s, c) = b.to_radians().sin_cos();
    DVec3::new(c, -s, 0.0)
}

/// The wire's direction in the table frame at `b` degrees.
pub fn wire_dir(b: f64) -> DVec3 {
    let (s, c) = b.to_radians().sin_cos();
    DVec3::new(s, c, 0.0)
}

/// The table-frame point `t` along the wire (from its middle) at wire position `p`.
pub fn on_wire(p: &WirePos, t: f64) -> DVec3 {
    u_axis(p.b) * p.u + wire_dir(p.b) * t + DVec3::Z * p.h
}

/// A table-frame point's `u` at turntable angle `b`.
pub fn u_of(p: DVec3, b: f64) -> f64 {
    p.dot(u_axis(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_turns_with_the_table() {
        // At B = 0 the wire runs along Y and u is X.
        assert!((on_wire(&WirePos::new(10.0, 5.0, 0.0), 3.0) - DVec3::new(10.0, 3.0, 5.0)).length() < 1e-12);
        // Turning the table 90° (counter-clockwise from above) brings the part's -Y side to face +X.
        let p = on_wire(&WirePos::new(10.0, 0.0, 90.0), 0.0);
        assert!((p - DVec3::new(0.0, -10.0, 0.0)).length() < 1e-12, "{p}");
        assert!((u_of(DVec3::new(0.0, -10.0, 0.0), 90.0) - 10.0).abs() < 1e-12);
    }

    #[test]
    fn stats_count_cut_air_and_time() {
        let mut p = WirePath::new(WirePos::default());
        p.push(MoveKind::Rapid, WirePos::new(100.0, 0.0, 0.0), 0.0);
        p.push(MoveKind::Cut, WirePos::new(100.0, 100.0, 0.0), 1000.0);
        p.push(MoveKind::Rotate, WirePos::new(100.0, 100.0, 90.0), 0.0);
        // A zero move is dropped.
        p.push(MoveKind::Cut, WirePos::new(100.0, 100.0, 90.0), 1000.0);
        assert_eq!(p.moves.len(), 3);
        let (cut, air, secs) = p.stats(6000.0, 900.0);
        assert!((cut - 100.0).abs() < 1e-9 && (air - 100.0).abs() < 1e-9);
        // 1 s rapid, 6 s cut, 6 s turning.
        assert!((secs - 13.0).abs() < 1e-9, "{secs}");
    }
}
