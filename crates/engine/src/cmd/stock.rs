//! Stock commands: a foam block or a round billet on the turntable.

use glam::{DVec2, DVec3};
use serde_json::Value;

use super::{CommandSpec, Param, ParamKind, always};
use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::session::Session;
use crate::stock::{Stock, StockShape};

const SIDE: ParamKind = ParamKind::Length { min: 0.1, max: 50_000.0 };
const AT: ParamKind = ParamKind::Length { min: -50_000.0, max: 50_000.0 };

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "stock.block",
        label: "Stock",
        tooltip: "Set up the stock: a rectangular foam block, or a round billet, on the turntable",
        placement: None,
        icon: Some("stock_block"),
        shortcut: None,
        params: &[
            Param::required("width", SIDE, "Across the wire at B = 0 (X)"),
            Param::required("depth", SIDE, "Along the wire at B = 0 (Y)"),
            Param::required("height", SIDE, "Height"),
            Param::optional("x", AT, "Centre across the table from the axis (default 0)"),
            Param::optional("y", AT, "Centre along the wire from the axis (default 0)"),
            Param::optional("base", AT, "Height of the block's bottom above the table (a riser)"),
            Param::optional("rotation", ParamKind::Number { min: -360.0, max: 360.0 }, "Turn on the table, degrees"),
        ],
        returns: "stock",
        enabled: always,
        run: |s, p| {
            let num = |k: &str| params::opt_f64(p, k, -50_000.0, 50_000.0);
            let size =
                DVec3::new(num("width")?.unwrap_or(0.0), num("depth")?.unwrap_or(0.0), num("height")?.unwrap_or(0.0));
            let stock = Stock::block(
                size,
                DVec2::new(num("x")?.unwrap_or(0.0), num("y")?.unwrap_or(0.0)),
                num("base")?.unwrap_or(0.0),
                num("rotation")?.unwrap_or(0.0),
            )?;
            set(s, stock)
        },
    },
    CommandSpec {
        id: "stock.cylinder",
        label: "Round billet",
        tooltip: "Set the stock to a round foam billet standing on the turntable",
        placement: None,
        icon: Some("stock_cylinder"),
        shortcut: None,
        params: &[
            Param::required("diameter", SIDE, "Diameter"),
            Param::required("height", SIDE, "Height"),
            Param::optional("x", AT, "Centre across the table from the axis (default 0)"),
            Param::optional("y", AT, "Centre along the wire from the axis (default 0)"),
            Param::optional("base", AT, "Height of the billet's bottom above the table (a riser)"),
        ],
        returns: "stock",
        enabled: always,
        run: |s, p| {
            let num = |k: &str| params::opt_f64(p, k, -50_000.0, 50_000.0);
            let stock = Stock::cylinder(
                num("diameter")?.unwrap_or(0.0) / 2.0,
                num("height")?.unwrap_or(0.0),
                DVec2::new(num("x")?.unwrap_or(0.0), num("y")?.unwrap_or(0.0)),
                num("base")?.unwrap_or(0.0),
            )?;
            set(s, stock)
        },
    },
    CommandSpec {
        id: "stock.get",
        label: "Stock",
        tooltip: "The stock",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "stock",
        enabled: always,
        run: |s, _| Ok(s.job.stock.to_json()),
    },
];

fn set(s: &mut Session, stock: Stock) -> EngineResult<Value> {
    if stock.reach() > s.job.machine.wire_span / 2.0 {
        return Err(EngineError::InvalidParams("the stock reaches past the wire's towers".into()));
    }
    s.job.stock = stock;
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
    Ok(s.job.stock.to_json())
}

/// The current stock as `stock.block` / `stock.cylinder` parameters, for their dialogs.
pub fn prefill(stock: &Stock) -> Value {
    match stock.shape {
        StockShape::Block { size } => serde_json::json!({
            "width": size.x, "depth": size.y, "height": size.z,
            "x": stock.centre.x, "y": stock.centre.y, "base": stock.base, "rotation": stock.rotation,
        }),
        StockShape::Cylinder { radius, height } => serde_json::json!({
            "diameter": radius * 2.0, "height": height,
            "x": stock.centre.x, "y": stock.centre.y, "base": stock.base,
        }),
    }
}
