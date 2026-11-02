//! Shape commands: import outlines from drawings, make common ones, edit and remove them.

use CosmicCut_io::Units;
use CosmicCut_io::profile::{self, Path2};
use glam::DVec2;
use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::session::Session;
use crate::shape::{self, Shape};

const SIZE: ParamKind = ParamKind::Length { min: 0.01, max: 50_000.0 };
const NAME: Param = Param::optional("name", ParamKind::Str, "Name for the shape");
const TOL: f64 = 0.01;

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "shape.import",
        label: "Import sketch",
        tooltip: "Read outlines from a DXF or SVG drawing, airfoil coordinates (.dat) or an XY point list",
        placement: Some(Placement::large(RibbonTab::Sketch, "Sketch")),
        icon: None,
        shortcut: Some("Ctrl+I"),
        params: &[
            Param::required("path", ParamKind::Str, "The file"),
            NAME,
            Param::optional("units", ParamKind::Choice(Units::IDS), "Units the file is drawn in, when it does not say"),
            Param::optional(
                "chord",
                ParamKind::Length { min: 0.01, max: 50_000.0 },
                "Chord of an airfoil read from .dat coordinates (default 10 in)",
            ),
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let path = params::req_str(p, "path")?;
            let format = profile::ProfileFormat::from_path(std::path::Path::new(path))
                .map_err(|e| EngineError::Import(e.to_string()))?;
            let read = profile::import_profile(std::path::Path::new(path), TOL)
                .map_err(|e| EngineError::Import(e.to_string()))?;
            let asked = params::opt_str(p, "units")?.and_then(Units::from_id);
            let scale = match format {
                profile::ProfileFormat::Dat => params::opt_f64(p, "chord", 0.01, 50_000.0)?.unwrap_or(254.0),
                _ => f64::from(read.units.or(asked).unwrap_or(Units::Millimetres).to_mm()),
            };
            let paths = read
                .paths
                .into_iter()
                .map(|q| Path2 { closed: q.closed, points: q.points.into_iter().map(|pt| pt * scale).collect() })
                .collect();
            let stem = std::path::Path::new(path).file_stem().and_then(|n| n.to_str()).unwrap_or("Shape");
            let mut out = add(s, p, stem, paths, path.to_owned())?;
            out["warnings"] = json!(read.warnings);
            Ok(out)
        },
    },
    CommandSpec {
        id: "shape.rectangle",
        label: "Rectangle",
        tooltip: "A rectangle, with rounded corners if wanted",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("width", SIZE, "Width"),
            Param::required("height", SIZE, "Height"),
            Param::optional("radius", ParamKind::Length { min: 0.0, max: 50_000.0 }, "Corner radius"),
            NAME,
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let (w, h) = (len(p, "width")?, len(p, "height")?);
            let r = params::opt_f64(p, "radius", 0.0, 50_000.0)?.unwrap_or(0.0);
            add(s, p, "Rectangle", shape::rectangle(w, h, r, TOL), "rectangle".into())
        },
    },
    CommandSpec {
        id: "shape.circle",
        label: "Circle",
        tooltip: "A circle",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("diameter", SIZE, "Diameter"), NAME],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let d = len(p, "diameter")?;
            add(s, p, "Circle", shape::circle(d, TOL), "circle".into())
        },
    },
    CommandSpec {
        id: "shape.polygon",
        label: "Polygon",
        tooltip: "A regular polygon",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("sides", ParamKind::Integer { min: 3, max: 1000 }, "Number of sides"),
            Param::required("diameter", SIZE, "Across the corners"),
            NAME,
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let n = params::opt_f64(p, "sides", 3.0, 1000.0)?.unwrap_or(6.0) as usize;
            let d = len(p, "diameter")?;
            add(s, p, "Polygon", shape::polygon(n, d), format!("{n}-sided polygon"))
        },
    },
    CommandSpec {
        id: "shape.star",
        label: "Star",
        tooltip: "A star",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("points", ParamKind::Integer { min: 2, max: 500 }, "Number of points"),
            Param::required("outer", SIZE, "Diameter through the tips"),
            Param::required("inner", SIZE, "Diameter through the valleys"),
            NAME,
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let n = params::opt_f64(p, "points", 2.0, 500.0)?.unwrap_or(5.0) as usize;
            let (o, i) = (len(p, "outer")?, len(p, "inner")?);
            add(s, p, "Star", shape::star(n, o, i), format!("{n}-point star"))
        },
    },
    CommandSpec {
        id: "shape.airfoil",
        label: "NACA airfoil",
        tooltip: "A NACA four-digit wing section (e.g. 2412) for wing cores",
        placement: Some(Placement::large(RibbonTab::Sketch, "Sketch")),
        icon: None,
        shortcut: None,
        params: &[
            Param::required("code", ParamKind::Str, "Four digits: camber %, its place (tenths), thickness %"),
            Param::required("chord", SIZE, "Chord"),
            Param::optional("points", ParamKind::Integer { min: 10, max: 2000 }, "Points along each side (default 80)"),
            NAME,
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let code = params::req_str(p, "code")?.to_owned();
            let chord = len(p, "chord")?;
            let n = params::opt_f64(p, "points", 10.0, 2000.0)?.unwrap_or(80.0) as usize;
            let paths = shape::naca4(&code, chord, n)?;
            add(s, p, &format!("NACA {code}"), paths, format!("NACA {code}"))
        },
    },
    CommandSpec {
        id: "shape.points",
        label: "Shape from points",
        tooltip: "A shape from a list of points",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("points", ParamKind::PointList { max: 100_000 }, "Points [x, y, 0], mm"),
            Param::optional("closed", ParamKind::Bool, "The last point joins the first (default true)"),
            NAME,
        ],
        returns: "shape",
        enabled: always,
        run: |s, p| {
            let pts: Vec<DVec2> = params::point_list(p, "points", 100_000)?.into_iter().map(|q| q.truncate()).collect();
            let closed = params::opt_bool(p, "closed")?.unwrap_or(true);
            add(s, p, "Points", vec![Path2 { points: pts, closed }], "points".into())
        },
    },
    CommandSpec {
        id: "shape.transform",
        label: "Transform",
        tooltip: "Scale, turn, mirror or re-centre the selected shape (cuts using it follow)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The shape"),
            Param::optional("scale", ParamKind::Number { min: 1.0e-6, max: 1.0e6 }, "Scale factor"),
            Param::optional(
                "rotate",
                ParamKind::Number { min: -3600.0, max: 3600.0 },
                "Turn, degrees counter-clockwise",
            ),
            Param::optional("mirror", ParamKind::Bool, "Mirror left to right"),
            Param::optional("centre", ParamKind::Bool, "Move its middle to the origin"),
        ],
        returns: "shape",
        enabled: super::has_shapes,
        run: |s, p| {
            let i = index(s, p)?;
            let scale = params::opt_f64(p, "scale", 1.0e-6, 1.0e6)?.unwrap_or(1.0);
            let rotate = params::opt_f64(p, "rotate", -3600.0, 3600.0)?.unwrap_or(0.0);
            let mirror = params::opt_bool(p, "mirror")?.unwrap_or(false);
            let Some(sh) = s.job.shapes.get_mut(i) else { return Err(missing()) };
            sh.paths = sh.transformed(scale, rotate, mirror);
            if params::opt_bool(p, "centre")?.unwrap_or(false) {
                let (lo, hi) = sh.bounds();
                let mid = (lo + hi) * 0.5;
                for q in sh.paths.iter_mut().flat_map(|q| q.points.iter_mut()) {
                    *q -= mid;
                }
            }
            if mirror {
                for q in &mut sh.paths {
                    q.points.reverse();
                }
            }
            changed(s, i)
        },
    },
    CommandSpec {
        id: "shape.rename",
        label: "Rename shape",
        tooltip: "Rename a shape",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The shape"),
            Param::required("name", ParamKind::Str, "New name"),
        ],
        returns: "shape",
        enabled: super::has_shapes,
        run: |s, p| {
            let i = index(s, p)?;
            let name = params::req_str(p, "name")?.trim().to_owned();
            if name.is_empty() {
                return Err(EngineError::InvalidParams("`name` must not be empty".into()));
            }
            let Some(sh) = s.job.shapes.get_mut(i) else { return Err(missing()) };
            sh.name = name.chars().take(200).collect();
            changed(s, i)
        },
    },
    CommandSpec {
        id: "shape.remove",
        label: "Delete shape",
        tooltip: "Delete a shape (cuts that use it can no longer be made)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("id", ParamKind::Str, "The shape")],
        returns: "{\"removed\": id}",
        enabled: super::has_shapes,
        run: |s, p| {
            let i = index(s, p)?;
            let sh = s.job.shapes.remove(i);
            if s.view.selection.as_deref() == Some(sh.id.as_str()) {
                s.view.selection = None;
            }
            s.job.rebuild();
            s.job.touch();
            s.sim = None;
            Ok(json!({ "removed": sh.id }))
        },
    },
    CommandSpec {
        id: "shape.get",
        label: "Shape",
        tooltip: "A shape and its points",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("id", ParamKind::Str, "The shape")],
        returns: "shape with its paths",
        enabled: always,
        run: |s, p| {
            let i = index(s, p)?;
            s.job.shapes.get(i).map(Shape::save_json).ok_or_else(missing)
        },
    },
];

fn len(p: &Value, key: &str) -> EngineResult<f64> {
    params::opt_f64(p, key, 0.0, 1.0e6)?.ok_or_else(|| EngineError::InvalidParams(format!("`{key}` is required")))
}

fn missing() -> EngineError {
    EngineError::InvalidParams("there is no such shape".into())
}

fn index(s: &Session, p: &Value) -> EngineResult<usize> {
    let id = params::req_str(p, "id")?;
    s.job
        .shapes
        .iter()
        .position(|sh| sh.id == id || sh.name == id)
        .ok_or_else(|| EngineError::InvalidParams(format!("there is no shape `{id}`")))
}

fn changed(s: &mut Session, i: usize) -> EngineResult<Value> {
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
    s.job.shapes.get(i).map(Shape::to_json).ok_or_else(missing)
}

pub(crate) fn add(s: &mut Session, p: &Value, base: &str, paths: Vec<Path2>, source: String) -> EngineResult<Value> {
    let name = match params::opt_str(p, "name")?.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.chars().take(200).collect(),
        None => s.job.fresh_name(base),
    };
    let id = s.job.fresh_id("shape");
    let mut shape = Shape::new(id.clone(), name, paths, source)?;
    // Drawn as a sketch on the front plane when it fits one, so it takes constraints and
    // dimensions like anything drawn.
    if let Some(sketch) = crate::sketch::Sketch::from_paths(crate::sketch::Plane::Front, &shape.paths) {
        let source = std::mem::take(&mut shape.source);
        shape = Shape::from_sketch(id.clone(), shape.name.clone(), sketch);
        shape.source = source;
    }
    let out = shape.to_json();
    s.job.shapes.push(shape);
    s.job.touch();
    s.view.selection = Some(id);
    Ok(out)
}
