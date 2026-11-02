//! The hot-wire machine: its travel, where the turntable sits, the wire and how programs for it
//! are written.

use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::post::PostSettings;
use crate::units::DisplayUnits;

#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub name: String,
    /// Wire travel from machine zero, mm.
    pub travel_x: f64,
    pub travel_y: f64,
    /// Machine X of the turntable's axis, mm.
    pub table_x: f64,
    /// Height of the wire above the table top with the machine at Y0, mm.
    pub wire_y0: f64,
    pub table_radius: f64,
    /// Distance between the wire's two towers (the length of wire across the table), mm.
    pub wire_span: f64,
    /// The machine has a turntable (B axis).
    pub b_axis: bool,
    /// Positive B turns the table clockwise seen from above (the frame here is counter-clockwise).
    pub b_reversed: bool,
    /// Rapid rate of X and Y, mm/min.
    pub rapid: f64,
    pub max_feed: f64,
    /// Turntable rate, degrees/min.
    pub b_rate: f64,
    /// Feed new cuts start with, mm/min.
    pub default_feed: f64,
    /// Width of foam the wire melts away, mm.
    pub kerf: f64,
    /// How far from the foam the wire waits, enters and turns the table, mm.
    pub clearance: f64,
    /// The post processor (from the library) that writes its programs.
    pub post_name: String,
    /// How programs for it are written: a copy of that post's settings.
    pub post: PostSettings,
}

impl Default for Machine {
    /// A 100 × 55 in machine with a 48 in turntable in the middle of X.
    fn default() -> Self {
        Self {
            name: "Hot-wire XYB".into(),
            travel_x: 2540.0,
            travel_y: 1397.0,
            table_x: 1270.0,
            wire_y0: 0.0,
            table_radius: 609.6,
            wire_span: 1828.8,
            b_axis: true,
            b_reversed: false,
            rapid: 19050.0,
            max_feed: 25400.0,
            b_rate: 3600.0,
            default_feed: 1016.0,
            kerf: 0.8382,
            clearance: 25.4,
            post_name: crate::library::DEFAULT_POST.into(),
            post: PostSettings::default(),
        }
    }
}

/// `machine.edit` parameters, in the order its dialog shows them.
pub const PARAMS: &[crate::Param] = {
    use crate::ParamKind::*;
    use crate::cmd::Param;
    &[
        Param::optional("name", Str, "Machine name"),
        Param::optional("post", Str, "The post processor that writes its programs (from the post processor library)"),
        Param::optional("travel_x", Length { min: 10.0, max: 100_000.0 }, "Wire travel across, X"),
        Param::optional("travel_y", Length { min: 10.0, max: 100_000.0 }, "Wire travel up, Y"),
        Param::optional("table_x", Length { min: -100_000.0, max: 100_000.0 }, "Machine X of the turntable's axis"),
        Param::optional(
            "wire_y0",
            Length { min: -10_000.0, max: 10_000.0 },
            "Height of the wire above the table top at machine Y0",
        ),
        Param::optional("table_radius", Length { min: 1.0, max: 50_000.0 }, "Turntable radius"),
        Param::optional("wire_span", Length { min: 10.0, max: 100_000.0 }, "Distance between the wire's towers"),
        Param::optional("b_axis", Bool, "The machine has a turntable (B axis)"),
        Param::optional("b_reversed", Bool, "Positive B turns the table clockwise seen from above"),
        Param::optional("rapid", Rate { min: 1.0, max: 1.0e6 }, "Rapid rate of X and Y"),
        Param::optional("max_feed", Rate { min: 1.0, max: 1.0e6 }, "Fastest feed the machine takes"),
        Param::optional("b_rate", Number { min: 1.0, max: 1.0e6 }, "Turntable rate, degrees per minute"),
        Param::optional("default_feed", Rate { min: 1.0, max: 1.0e6 }, "Feed new cuts start with"),
        Param::optional("kerf", Length { min: 0.0, max: 50.0 }, "Width of foam the wire melts away"),
        Param::optional(
            "clearance",
            Length { min: 0.0, max: 10_000.0 },
            "How far from the foam the wire waits, enters and turns the table",
        ),
    ]
};

impl Machine {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "travel_x": self.travel_x,
            "travel_y": self.travel_y,
            "table_x": self.table_x,
            "wire_y0": self.wire_y0,
            "table_radius": self.table_radius,
            "wire_span": self.wire_span,
            "b_axis": self.b_axis,
            "b_reversed": self.b_reversed,
            "rapid": self.rapid,
            "max_feed": self.max_feed,
            "b_rate": self.b_rate,
            "default_feed": self.default_feed,
            "kerf": self.kerf,
            "clearance": self.clearance,
            "post_name": self.post_name,
            "post": self.post.to_json(),
        })
    }

    /// Applies the fields present in `p` (checked against [`PARAMS`]).
    pub fn apply(&mut self, p: &Value) -> EngineResult<()> {
        let mut m = self.clone();
        if let Some(name) = params::opt_str(p, "name")? {
            let name = name.trim();
            if name.is_empty() {
                return Err(EngineError::InvalidParams("`name` must not be empty".into()));
            }
            m.name = name.to_owned();
        }
        let num = |key: &str, slot: &mut f64| -> EngineResult<()> {
            if let Some(v) = params::opt_f64(p, key, -1.0e9, 1.0e9)? {
                *slot = v;
            }
            Ok(())
        };
        num("travel_x", &mut m.travel_x)?;
        num("travel_y", &mut m.travel_y)?;
        num("table_x", &mut m.table_x)?;
        num("wire_y0", &mut m.wire_y0)?;
        num("table_radius", &mut m.table_radius)?;
        num("wire_span", &mut m.wire_span)?;
        num("rapid", &mut m.rapid)?;
        num("max_feed", &mut m.max_feed)?;
        num("b_rate", &mut m.b_rate)?;
        num("default_feed", &mut m.default_feed)?;
        num("kerf", &mut m.kerf)?;
        num("clearance", &mut m.clearance)?;
        if let Some(b) = params::opt_bool(p, "b_axis")? {
            m.b_axis = b;
        }
        if let Some(b) = params::opt_bool(p, "b_reversed")? {
            m.b_reversed = b;
        }
        m.check()?;
        *self = m;
        Ok(())
    }

    /// Reads a machine saved by [`Machine::to_json`]; missing fields keep their defaults.
    pub fn from_json(v: &Value) -> EngineResult<Self> {
        let mut m = Self::default();
        let mut p = v.clone();
        if let Some(o) = p.as_object_mut() {
            o.retain(|k, _| PARAMS.iter().any(|q| q.name == k));
        }
        m.apply(&p)?;
        if let Some(post) = v.get("post") {
            m.post.apply(post)?;
        }
        if let Some(name) = v.get("post_name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
            m.post_name = name.chars().take(120).collect();
        }
        Ok(m)
    }

    fn check(&self) -> EngineResult<()> {
        let bad = |what: &str| Err(EngineError::InvalidParams(what.into()));
        if !(self.travel_x > 0.0 && self.travel_y > 0.0) {
            return bad("the travel must be more than zero");
        }
        if !(self.table_radius > 0.0 && self.wire_span > 0.0) {
            return bad("the turntable and wire span must be more than zero");
        }
        if !(self.rapid > 0.0 && self.max_feed > 0.0 && self.b_rate > 0.0 && self.default_feed > 0.0) {
            return bad("rates must be more than zero");
        }
        if !(self.kerf >= 0.0 && self.clearance >= 0.0) {
            return bad("kerf and clearance cannot be negative");
        }
        Ok(())
    }

    /// Machine X and Y of a wire position.
    pub fn machine_xy(&self, u: f64, h: f64) -> (f64, f64) {
        (self.table_x + u, h - self.wire_y0)
    }

    /// The program's B word for a table-frame angle.
    pub fn machine_b(&self, b: f64) -> f64 {
        if self.b_reversed { -b } else { b }
    }

    /// Properties for the inspector.
    pub fn properties(&self, units: DisplayUnits) -> Vec<(String, String)> {
        vec![
            ("Travel".into(), format!("{} × {}", units.fmt_len(self.travel_x), units.fmt_len(self.travel_y))),
            ("Turntable axis at X".into(), units.fmt_len(self.table_x)),
            ("Turntable radius".into(), units.fmt_len(self.table_radius)),
            ("Wire above table at Y0".into(), units.fmt_len(self.wire_y0)),
            ("Wire span".into(), units.fmt_len(self.wire_span)),
            ("B axis".into(), if self.b_axis { "yes" } else { "no" }.into()),
            ("Kerf".into(), units.fmt_len(self.kerf)),
            ("Clearance".into(), units.fmt_len(self.clearance)),
            ("Rapid".into(), units.fmt_rate(self.rapid)),
            ("Default feed".into(), units.fmt_rate(self.default_feed)),
            ("Turntable rate".into(), format!("{:.0}°/min", self.b_rate)),
            ("Post processor".into(), self.post_name.clone()),
            ("Program units".into(), self.post.units.label().into()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_round_trip_and_bad_values_are_refused() {
        let mut m = Machine::default();
        assert!(m.apply(&json!({ "kerf": 1.0 })).is_ok());
        assert!(m.post.apply(&json!({ "units": "mm", "wire_on": "M3 S40" })).is_ok());
        assert_eq!(m.post.units, DisplayUnits::Millimetres);
        let back = Machine::from_json(&m.to_json());
        assert_eq!(back.as_ref().ok(), Some(&m));
        assert!(m.apply(&json!({ "rapid": 0.0 })).is_err());
        assert_eq!(m.rapid, Machine::default().rapid);
        assert_eq!(m.machine_xy(10.0, 0.0), (1280.0, 0.0));
    }
}
