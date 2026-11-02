//! The foam stock on the turntable: a block or a round billet, set on the table (or on a riser)
//! and turned about the table's axis.

use CosmicCut_geom::{Aabb, Vec3};
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::units::DisplayUnits;
use crate::wire::u_axis;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StockShape {
    /// Width (X), depth (Y, along the wire at B = 0) and height, mm.
    Block { size: DVec3 },
    /// A round billet standing on end.
    Cylinder { radius: f64, height: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stock {
    pub shape: StockShape,
    /// Centre of the stock's footprint on the table, table frame, mm.
    pub centre: DVec2,
    /// Height of the stock's bottom above the table top (a riser), mm.
    pub base: f64,
    /// Turn of the block about its centre, degrees counter-clockwise from above.
    pub rotation: f64,
}

/// Largest stock side accepted, mm.
pub const MAX_SIZE: f64 = 50_000.0;

impl Default for Stock {
    /// A 24 × 24 × 24 in block in the middle of the table.
    fn default() -> Self {
        Self {
            shape: StockShape::Block { size: DVec3::new(609.6, 609.6, 609.6) },
            centre: DVec2::ZERO,
            base: 0.0,
            rotation: 0.0,
        }
    }
}

impl Stock {
    pub fn block(size: DVec3, centre: DVec2, base: f64, rotation: f64) -> EngineResult<Self> {
        if !(size.is_finite() && size.min_element() > 0.0 && size.max_element() <= MAX_SIZE) {
            return Err(EngineError::InvalidParams(format!("the block's sides must be between 0 and {MAX_SIZE} mm")));
        }
        Ok(Self { shape: StockShape::Block { size }, centre, base, rotation })
    }

    pub fn cylinder(radius: f64, height: f64, centre: DVec2, base: f64) -> EngineResult<Self> {
        if !(radius > 0.0 && height > 0.0 && radius <= MAX_SIZE && height <= MAX_SIZE) {
            return Err(EngineError::InvalidParams("the billet's radius and height must be more than zero".into()));
        }
        Ok(Self { shape: StockShape::Cylinder { radius, height }, centre, base, rotation: 0.0 })
    }

    pub fn height(&self) -> f64 {
        match self.shape {
            StockShape::Block { size } => size.z,
            StockShape::Cylinder { height, .. } => height,
        }
    }

    pub fn bottom(&self) -> f64 {
        self.base
    }

    pub fn top(&self) -> f64 {
        self.base + self.height()
    }

    /// The footprint's corners (a block) in the table frame.
    pub fn corners(&self) -> Vec<DVec2> {
        match self.shape {
            StockShape::Block { size } => {
                let (s, c) = self.rotation.to_radians().sin_cos();
                let (hx, hy) = (size.x / 2.0, size.y / 2.0);
                [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)]
                    .into_iter()
                    .map(|(x, y)| self.centre + DVec2::new(x * c - y * s, x * s + y * c))
                    .collect()
            }
            StockShape::Cylinder { .. } => Vec::new(),
        }
    }

    /// The stock's `u` extent at turntable angle `b`.
    pub fn u_range(&self, b: f64) -> (f64, f64) {
        let ax = u_axis(b);
        let ax = DVec2::new(ax.x, ax.y);
        match self.shape {
            StockShape::Block { .. } => {
                let us: Vec<f64> = self.corners().iter().map(|p| p.dot(ax)).collect();
                (us.iter().copied().fold(f64::INFINITY, f64::min), us.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            }
            StockShape::Cylinder { radius, .. } => {
                let c = self.centre.dot(ax);
                (c - radius, c + radius)
            }
        }
    }

    /// The farthest the stock reaches from the turntable's axis: the wire turns the table clear
    /// of this.
    pub fn reach(&self) -> f64 {
        match self.shape {
            StockShape::Block { .. } => self.corners().iter().map(|p| p.length()).fold(0.0, f64::max),
            StockShape::Cylinder { radius, .. } => self.centre.length() + radius,
        }
    }

    /// Signed distance (mm, positive inside) from a table-frame point into the stock: exact for
    /// the billet and the block's faces, good enough near the block's edges.
    pub fn inside(&self, p: DVec3) -> f64 {
        let vertical = (p.z - self.bottom()).min(self.top() - p.z);
        let q = DVec2::new(p.x, p.y) - self.centre;
        let across = match self.shape {
            StockShape::Block { size } => {
                let (s, c) = self.rotation.to_radians().sin_cos();
                let local = DVec2::new(q.x * c + q.y * s, -q.x * s + q.y * c);
                (size.x / 2.0 - local.x.abs()).min(size.y / 2.0 - local.y.abs())
            }
            StockShape::Cylinder { radius, .. } => radius - q.length(),
        };
        vertical.min(across)
    }

    pub fn bounds(&self) -> Aabb {
        let (lo, hi) = match self.shape {
            StockShape::Block { .. } => {
                let c = self.corners();
                let lo = c.iter().fold(DVec2::splat(f64::INFINITY), |a, p| a.min(*p));
                let hi = c.iter().fold(DVec2::splat(f64::NEG_INFINITY), |a, p| a.max(*p));
                (lo, hi)
            }
            StockShape::Cylinder { radius, .. } => (self.centre - radius, self.centre + radius),
        };
        Aabb::new(
            Vec3::new(lo.x as f32, lo.y as f32, self.bottom() as f32),
            Vec3::new(hi.x as f32, hi.y as f32, self.top() as f32),
        )
    }

    pub fn volume(&self) -> f64 {
        match self.shape {
            StockShape::Block { size } => size.x * size.y * size.z,
            StockShape::Cylinder { radius, height } => std::f64::consts::PI * radius * radius * height,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = match self.shape {
            StockShape::Block { size } => json!({ "kind": "block", "size": [size.x, size.y, size.z] }),
            StockShape::Cylinder { radius, height } => {
                json!({ "kind": "cylinder", "radius": radius, "height": height })
            }
        };
        v["centre"] = json!([self.centre.x, self.centre.y]);
        v["base"] = json!(self.base);
        v["rotation"] = json!(self.rotation);
        v
    }

    pub fn from_json(v: &Value) -> EngineResult<Self> {
        let num = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|n| n.is_finite());
        let centre = v
            .get("centre")
            .and_then(Value::as_array)
            .and_then(|a| Some(DVec2::new(num(a.first())?, num(a.get(1))?)))
            .unwrap_or_default();
        let base = num(v.get("base")).unwrap_or(0.0);
        match v.get("kind").and_then(Value::as_str) {
            Some("cylinder") => {
                Self::cylinder(num(v.get("radius")).unwrap_or(0.0), num(v.get("height")).unwrap_or(0.0), centre, base)
            }
            _ => {
                let s = v.get("size").and_then(Value::as_array);
                let c = |i: usize| s.and_then(|a| num(a.get(i))).unwrap_or(0.0);
                Self::block(DVec3::new(c(0), c(1), c(2)), centre, base, num(v.get("rotation")).unwrap_or(0.0))
            }
        }
    }

    pub fn describe(&self, units: DisplayUnits) -> String {
        match self.shape {
            StockShape::Block { size } => {
                format!("Block {}", units.fmt_size([size.x as f32, size.y as f32, size.z as f32]))
            }
            StockShape::Cylinder { radius, height } => {
                format!("Billet Ø{} × {}", units.fmt_len(2.0 * radius), units.fmt_len(height))
            }
        }
    }

    pub fn properties(&self, units: DisplayUnits) -> Vec<(String, String)> {
        let mut out = vec![("Shape".into(), self.describe(units))];
        out.push(("Centre".into(), format!("{}, {}", units.fmt_len(self.centre.x), units.fmt_len(self.centre.y))));
        out.push(("Bottom (riser)".into(), units.fmt_len(self.base)));
        if let StockShape::Block { .. } = self.shape {
            out.push(("Rotation".into(), format!("{:.2}°", self.rotation)));
        }
        out.push(("Reach from axis".into(), units.fmt_len(self.reach())));
        let cubic = units.mm_per_unit().powi(3);
        out.push(("Volume".into(), format!("{:.1} {}³", self.volume() / cubic, units.length())));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_block_projects_wider() -> EngineResult<()> {
        let s = Stock::block(DVec3::new(100.0, 100.0, 50.0), DVec2::ZERO, 10.0, 0.0)?;
        let (a, b) = s.u_range(0.0);
        assert!((a + 50.0).abs() < 1e-9 && (b - 50.0).abs() < 1e-9);
        let (a, b) = s.u_range(45.0);
        assert!((b - a - 100.0 * 2f64.sqrt()).abs() < 1e-9);
        assert!((s.reach() - 50.0 * 2f64.sqrt()).abs() < 1e-9);
        assert!((s.inside(DVec3::new(0.0, 0.0, 35.0)) - 25.0).abs() < 1e-9);
        assert!(s.inside(DVec3::new(0.0, 0.0, 5.0)) < 0.0);
        let back = Stock::from_json(&s.to_json())?;
        assert_eq!(back, s);
        Ok(())
    }
}
