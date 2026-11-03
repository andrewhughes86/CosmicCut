//! Model commands: import 3D models, lay them on a face, and make cuts from them.

use CosmicCut_io::Units;
use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, always};
use crate::error::{EngineError, EngineResult};
use crate::model::{self, Model};
use crate::params;
use crate::session::Session;

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "model.import",
        label: "Import model",
        tooltip: "Read a 3D model (STL, OBJ, 3MF, STEP, IGES and more), lay it on the turntable and centre it",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+M"),
        params: &[
            Param::required("path", ParamKind::Str, "The file"),
            Param::optional("units", ParamKind::Choice(Units::IDS), "Units the file is drawn in, when it does not say"),
            Param::optional("name", ParamKind::Str, "Name for the model"),
        ],
        returns: "model",
        enabled: always,
        run: |s, p| {
            let path = params::req_str(p, "path")?.to_owned();
            let units = params::opt_str(p, "units")?.map(str::to_owned);
            let (mesh, warnings) = model::read(&path, units.as_deref())?;
            let stem = std::path::Path::new(&path).file_stem().and_then(|n| n.to_str()).unwrap_or("Model").to_owned();
            let name = params::opt_str(p, "name")?
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map_or_else(|| s.job.fresh_name(&stem), str::to_owned);
            let id = s.job.fresh_id("model");
            let mut m = Model::new(id.clone(), name, path, units, mesh)?;
            m.place(s.job.stock.bottom());
            let mut out = m.to_json();
            out["warnings"] = json!(warnings);
            s.job.models.push(m);
            s.job.touch();
            s.sim = None;
            s.view.selection = Some(id);
            Ok(out)
        },
    },
    CommandSpec {
        id: "model.lay_flat",
        label: "Bottom face",
        tooltip: "Click the face of the model that should sit on the turntable: the model turns so that face is down \
                  and centres itself on the table again",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The model (default the selected one)"),
            Param::optional(
                "triangle",
                ParamKind::Integer { min: 0, max: 100_000_000 },
                "A triangle of the face to put down",
            ),
            Param::optional("down", ParamKind::Vec3, "Or the direction, in the model as read, to point down"),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let down = match (params::opt_f64(p, "triangle", 0.0, 1.0e8)?, params::opt_vec3(p, "down")?) {
                (Some(t), _) => s.job.models.get(i).and_then(|m| m.down_for(t as usize)),
                (None, Some(d)) => Some(d.as_dvec3()).filter(|d| d.length_squared() > 1.0e-12),
                (None, None) => return Err(EngineError::InvalidParams("give `triangle` or `down`".into())),
            }
            .ok_or_else(|| EngineError::InvalidParams("that face cannot be laid down".into()))?;
            let floor = s.job.stock.bottom();
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            m.down = down.normalize();
            m.tilt_x = 0.0;
            m.tilt_y = 0.0;
            m.centred = true;
            m.place(floor);
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.place",
        label: "Position",
        tooltip: "Turn the model on the table, raise it off the stock's bottom, or (not centred) move it across the table",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The model (default the selected one)"),
            Param::optional("rotate", ParamKind::Number { min: -3600.0, max: 3600.0 }, "Turn on the table, degrees"),
            Param::optional("scale", ParamKind::Number { min: 1.0e-6, max: 1.0e6 }, "Scale factor"),
            Param::optional(
                "lift",
                ParamKind::Length { min: -1.0e5, max: 1.0e5 },
                "Height of its bottom above the stock's",
            ),
            Param::optional(
                "x",
                ParamKind::Length { min: -1.0e5, max: 1.0e5 },
                "Where its middle stands across the table (frees it from the centre)",
            ),
            Param::optional(
                "y",
                ParamKind::Length { min: -1.0e5, max: 1.0e5 },
                "Where its middle stands along the wire (frees it from the centre)",
            ),
            Param::optional("reset", ParamKind::Bool, "Back to as it was read: its own bottom down, unturned"),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let floor = s.job.stock.bottom();
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            if params::opt_bool(p, "reset")?.unwrap_or(false) {
                m.down = DVec3::NEG_Z;
                m.rotate = 0.0;
                m.tilt_x = 0.0;
                m.tilt_y = 0.0;
                m.scale = 1.0;
                m.lift = 0.0;
                m.centred = true;
            }
            if let Some(v) = params::opt_f64(p, "rotate", -3600.0, 3600.0)? {
                m.rotate = v;
            }
            if let Some(v) = params::opt_f64(p, "scale", 1.0e-6, 1.0e6)? {
                m.scale = v;
            }
            if let Some(v) = params::opt_f64(p, "lift", -1.0e5, 1.0e5)? {
                m.lift = v;
            }
            let x = params::opt_f64(p, "x", -1.0e5, 1.0e5)?;
            let y = params::opt_f64(p, "y", -1.0e5, 1.0e5)?;
            if x.is_some() || y.is_some() {
                m.centred = false;
                m.at = glam::DVec2::new(x.unwrap_or(m.at.x), y.unwrap_or(m.at.y));
            }
            m.place(floor);
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.move",
        label: "Move",
        tooltip: "Move and turn the model: drag the arrows to slide it along X, Y or Z, the rings to turn it about \
                  them, then type the exact value",
        placement: None,
        icon: Some("model_move"),
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The model (default the selected one)"),
            Param::optional("x", ParamKind::Length { min: -1.0e5, max: 1.0e5 }, "Its middle across the table (X)"),
            Param::optional("y", ParamKind::Length { min: -1.0e5, max: 1.0e5 }, "Its middle along the wire (Y)"),
            Param::optional("z", ParamKind::Length { min: -1.0e5, max: 1.0e5 }, "Its bottom above the stock's (Z)"),
            Param::optional("rx", ParamKind::Number { min: -3600.0, max: 3600.0 }, "Turn about X, degrees"),
            Param::optional("ry", ParamKind::Number { min: -3600.0, max: 3600.0 }, "Turn about Y, degrees"),
            Param::optional("rz", ParamKind::Number { min: -3600.0, max: 3600.0 }, "Turn about Z, degrees"),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let floor = s.job.stock.bottom();
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            let turn = |k: &str| params::opt_f64(p, k, -3600.0, 3600.0);
            if let Some(v) = turn("rx")? {
                m.tilt_x = v;
            }
            if let Some(v) = turn("ry")? {
                m.tilt_y = v;
            }
            if let Some(v) = turn("rz")? {
                m.rotate = v;
            }
            if let Some(v) = params::opt_f64(p, "z", -1.0e5, 1.0e5)? {
                m.lift = v;
            }
            // Turning moves its middle; where it was asked to stand (or the axis) holds.
            let x = params::opt_f64(p, "x", -1.0e5, 1.0e5)?;
            let y = params::opt_f64(p, "y", -1.0e5, 1.0e5)?;
            if x.is_some() || y.is_some() {
                m.centred = false;
                m.at = DVec2::new(x.unwrap_or(m.at.x), y.unwrap_or(m.at.y));
            }
            m.place(floor);
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.duplicate",
        label: "Duplicate model",
        tooltip: "Copy the selected model, beside it on the table",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The model (default the selected one)")],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let m = s.job.models.get(i).cloned().ok_or_else(missing)?;
            let id = add_beside(s, m);
            let i = s.job.models.iter().position(|m| m.id == id).ok_or_else(missing)?;
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.scale",
        label: "Scale",
        tooltip: "Scale the model: as large as fits in the stock, to a height, or by a factor of its file size",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The model (default the selected one)"),
            Param::required(
                "mode",
                ParamKind::Choice(SCALE_MODES),
                "fit: as large as fits the stock; height: to a height; factor: by a factor",
            ),
            Param::optional(
                "margin",
                ParamKind::Length { min: 0.0, max: 10_000.0 },
                "fit: foam left round the sides and over the top (default 1/4 in)",
            ),
            Param::optional("height", ParamKind::Length { min: 0.01, max: 100_000.0 }, "height: the model's height"),
            Param::optional(
                "factor",
                ParamKind::Number { min: 1.0e-6, max: 1.0e6 },
                "factor: its size against the file's (1 = as drawn)",
            ),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let floor = s.job.stock.bottom();
            let stock = s.job.stock;
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            let height = f64::from(m.bounds().size().z);
            let scale = match params::req_str(p, "mode")? {
                "height" => {
                    let want = params::opt_f64(p, "height", 0.01, 1.0e5)?
                        .ok_or_else(|| EngineError::InvalidParams("give the `height` to scale to".into()))?;
                    if height <= 1.0e-9 {
                        return Err(EngineError::InvalidParams("the model is flat; it has no height to scale".into()));
                    }
                    m.scale * want / height
                }
                "factor" => params::opt_f64(p, "factor", 1.0e-6, 1.0e6)?
                    .ok_or_else(|| EngineError::InvalidParams("give the `factor` to scale by".into()))?,
                _ => {
                    let margin = params::opt_f64(p, "margin", 0.0, 1.0e4)?.unwrap_or(6.35);
                    m.scale * fit_ratio(&m.mesh, &stock, floor + m.lift, margin)?
                }
            };
            if !(scale.is_finite() && scale > 1.0e-6 && scale < 1.0e6) {
                return Err(EngineError::InvalidParams("that scale is out of range".into()));
            }
            m.scale = scale;
            m.place(floor);
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.centre",
        label: "Centre on table",
        tooltip: "Keep the model centred on the turntable (for cuts that turn the table), or free it to stand \
                  anywhere on the table for parts that do not need the turntable",
        placement: None,
        icon: Some("model_centre"),
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The model (default the selected one)"),
            Param::optional("on", ParamKind::Bool, "Centred (true) or free (false); omit to switch"),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let floor = s.job.stock.bottom();
            let want = params::opt_bool(p, "on")?;
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            let on = want.unwrap_or(!m.centred);
            if !on && m.centred {
                // Freed where it stands, so nothing jumps.
                let b = m.bounds();
                m.at = glam::DVec2::new(f64::from(b.min.x + b.max.x) / 2.0, f64::from(b.min.y + b.max.y) / 2.0);
            }
            m.centred = on;
            m.place(floor);
            changed(s, i)
        },
    },
    CommandSpec {
        id: "model.remove",
        label: "Delete model",
        tooltip: "Delete a model (shapes and cuts made from it stay)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::optional("id", ParamKind::Str, "The model (default the selected one)")],
        returns: "{\"removed\": id}",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let m = s.job.models.remove(i);
            if s.view.selection.as_deref() == Some(m.id.as_str()) {
                s.view.selection = None;
            }
            s.job.rebuild();
            s.job.touch();
            s.sim = None;
            Ok(json!({ "removed": m.id }))
        },
    },
    CommandSpec {
        id: "model.visibility.set",
        label: "Show or hide model",
        tooltip: "Show or hide a model in the viewport",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The model"),
            Param::optional("visible", ParamKind::Bool, "Show it; omit to toggle"),
        ],
        returns: "model",
        enabled: has_models,
        run: |s, p| {
            let i = index(s, p)?;
            let want = params::opt_bool(p, "visible")?;
            let m = s.job.models.get_mut(i).ok_or_else(missing)?;
            m.visible = want.unwrap_or(!m.visible);
            s.job.revision += 1;
            Ok(s.job.models.get(i).map(Model::to_json).unwrap_or_default())
        },
    },
    CommandSpec {
        id: "stock.fit",
        label: "Fit to model",
        tooltip: "Size a block round the model, with a margin on the sides and top",
        placement: None,
        icon: Some("stock_fit"),
        shortcut: None,
        params: &[
            Param::optional("model", ParamKind::Str, "The model (default the selected one, or the first)"),
            Param::optional("margin", ParamKind::Length { min: 0.0, max: 10_000.0 }, "Margin (default 1 in)"),
        ],
        returns: "stock",
        enabled: has_models,
        run: |s, p| {
            let m = match params::opt_str(p, "model")? {
                Some(id) => s.job.model(id),
                None => s.selected_model().or(s.job.models.first()),
            }
            .ok_or_else(missing)?;
            let margin = params::opt_f64(p, "margin", 0.0, 1.0e4)?.unwrap_or(25.4);
            let b = m.bounds();
            let base = s.job.stock.bottom();
            let size = DVec3::new(
                f64::from(b.size().x) + 2.0 * margin,
                f64::from(b.size().y) + 2.0 * margin,
                f64::from(b.max.z) - base + margin,
            );
            let stock = crate::stock::Stock::block(size, DVec2::ZERO, base, 0.0)?;
            s.job.stock = stock;
            s.job.rebuild();
            s.job.touch();
            s.sim = None;
            Ok(s.job.stock.to_json())
        },
    },
];

pub const SCALE_MODES: &[&str] = &["fit", "height", "factor"];

/// How much larger (or smaller) the placed model can be and still fit inside the stock, `margin`
/// clear of its sides and top: scaled about its centre on the turntable and its bottom at `floor`.
fn fit_ratio(mesh: &CosmicCut_geom::Mesh, stock: &crate::stock::Stock, floor: f64, margin: f64) -> EngineResult<f64> {
    let pts = mesh.positions();
    // Enough of the points to be sure of the outline, quickly.
    let stride = (pts.len() / 50_000).max(1);
    let sample: Vec<DVec3> = pts.iter().step_by(stride).map(|p| p.as_dvec3()).collect();
    let mid = (stock.bottom() + stock.top()) / 2.0;
    let fits = |k: f64| {
        sample.iter().all(|v| {
            let p = DVec3::new(v.x * k, v.y * k, floor + (v.z - floor) * k);
            p.z <= stock.top() - margin && stock.inside(DVec3::new(p.x, p.y, mid)) >= margin
        })
    };
    if !fits(1.0e-6) {
        return Err(EngineError::InvalidParams(
            "the model cannot fit: its middle is not inside the stock, or the margin is too large".into(),
        ));
    }
    let (mut lo, mut hi) = (1.0e-6, 1.0);
    while fits(hi) && hi < 1.0e6 {
        lo = hi;
        hi *= 2.0;
    }
    for _ in 0..60 {
        let k = (lo + hi) / 2.0;
        if fits(k) {
            lo = k;
        } else {
            hi = k;
        }
    }
    Ok(lo)
}

/// Adds a copy of `m` standing beside the original (to its right, an inch clear), selected.
/// Returns its id.
pub(crate) fn add_beside(s: &mut Session, mut m: Model) -> String {
    let floor = s.job.stock.bottom();
    let at = m.position(floor);
    let width = f64::from(m.bounds().size().x);
    m.id = s.job.fresh_id("model");
    m.name = s.job.fresh_name(&m.name);
    m.centred = false;
    m.at = DVec2::new(at.x + width + 25.4, at.y);
    m.place(floor);
    let id = m.id.clone();
    s.job.models.push(m);
    s.job.rebuild();
    s.view.selection = Some(id.clone());
    id
}

fn has_models(s: &Session) -> bool {
    !s.job.models.is_empty()
}

fn missing() -> EngineError {
    EngineError::InvalidParams("there is no such model".into())
}

/// The model `id` (or `model`) names, else the selected one, else the only one.
fn index(s: &Session, p: &Value) -> EngineResult<usize> {
    let named = match params::opt_str(p, "id")? {
        Some(id) => Some(id),
        None => params::opt_str(p, "model")?,
    };
    let id = match named {
        Some(id) => id.to_owned(),
        None => s
            .selected_model()
            .map(|m| m.id.clone())
            .or_else(|| (s.job.models.len() == 1).then(|| s.job.models.first().map(|m| m.id.clone())).flatten())
            .ok_or_else(|| EngineError::InvalidParams("select a model first".into()))?,
    };
    s.job
        .models
        .iter()
        .position(|m| m.id == id || m.name == id)
        .ok_or_else(|| EngineError::InvalidParams(format!("there is no model `{id}`")))
}

fn changed(s: &mut Session, i: usize) -> EngineResult<Value> {
    // Cuts made from the model follow it.
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
    s.job.models.get(i).map(Model::to_json).ok_or_else(missing)
}
