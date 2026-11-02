//! Job files (`.ccut`): JSON holding the machine, the stock, the shapes and the cuts' settings.
//! Cut paths are not stored; they are made again when the job opens.

use serde_json::{Value, json};

use crate::cut::{Cut, CutKind};
use crate::error::{EngineError, EngineResult};
use crate::machine::Machine;
use crate::session::JobState;
use crate::shape::Shape;
use crate::stock::Stock;
use crate::wire::WirePath;

pub const EXTENSION: &str = "ccut";
pub const FORMAT: &str = "CosmicCut/1";
/// Largest job file read.
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;

pub fn to_json(job: &JobState) -> Value {
    json!({
        "format": FORMAT,
        "name": job.name,
        "machine": job.machine.to_json(),
        "stock": job.stock.to_json(),
        "shapes": job.shapes.iter().map(Shape::save_json).collect::<Vec<_>>(),
        "models": job.models.iter().map(crate::model::Model::save_json).collect::<Vec<_>>(),
        "ops": job.ops.iter().map(crate::op::Operation::to_json).collect::<Vec<_>>(),
        "cuts": job.cuts.iter().map(|c| json!({
            "id": c.id, "name": c.name, "kind": c.kind.id(), "params": c.params, "visible": c.visible, "op": c.op,
        })).collect::<Vec<_>>(),
        "next_id": job.next_id,
    })
}

pub fn from_json(v: &Value) -> EngineResult<JobState> {
    let bad = |what: &str| EngineError::InvalidParams(format!("not a CosmicCut job: {what}"));
    match v.get("format").and_then(Value::as_str) {
        Some(f) if f.starts_with("CosmicCut/") => {}
        _ => return Err(bad("no CosmicCut format tag")),
    }
    let mut job = JobState {
        name: v.get("name").and_then(Value::as_str).unwrap_or("Untitled").chars().take(200).collect(),
        machine: v.get("machine").map(Machine::from_json).transpose()?.unwrap_or_default(),
        stock: v.get("stock").map(Stock::from_json).transpose()?.unwrap_or_default(),
        ..JobState::default()
    };
    for s in v.get("shapes").and_then(Value::as_array).into_iter().flatten().take(10_000) {
        job.shapes.push(Shape::load_json(s)?);
    }
    // Models are read again from their files; one that cannot be read is left out (its shapes
    // and cuts stay).
    for m in v.get("models").and_then(Value::as_array).into_iter().flatten().take(1000) {
        let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let units = m.get("units").and_then(Value::as_str);
        let Ok((mesh, _)) = crate::model::read(&s("source"), units) else { continue };
        let Ok(mut model) = crate::model::Model::new(s("id"), s("name"), s("source"), units.map(str::to_owned), mesh)
        else {
            continue;
        };
        let num = |k: &str, d: f64| m.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d);
        if let Some(d) = m.get("down").and_then(Value::as_array)
            && let [Some(x), Some(y), Some(z)] = [0, 1, 2].map(|i| d.get(i).and_then(Value::as_f64))
        {
            model.down = glam::DVec3::new(x, y, z);
        }
        model.rotate = num("rotate", 0.0);
        model.scale = num("scale", 1.0).clamp(1.0e-6, 1.0e6);
        model.lift = num("lift", 0.0);
        model.visible = m.get("visible").and_then(Value::as_bool).unwrap_or(true);
        model.centred = m.get("centred").and_then(Value::as_bool).unwrap_or(true);
        if let Some(a) = m.get("at").and_then(Value::as_array) {
            model.at = glam::DVec2::new(
                a.first().and_then(Value::as_f64).unwrap_or(0.0),
                a.get(1).and_then(Value::as_f64).unwrap_or(0.0),
            );
        }
        job.models.push(model);
    }
    for c in v.get("cuts").and_then(Value::as_array).into_iter().flatten().take(10_000) {
        let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let kind = CutKind::from_id(&s("kind")).ok_or_else(|| bad("a cut of an unknown kind"))?;
        job.cuts.push(Cut {
            id: s("id"),
            name: s("name"),
            kind,
            params: c.get("params").cloned().unwrap_or(Value::Null),
            visible: c.get("visible").and_then(Value::as_bool).unwrap_or(true),
            path: WirePath::default(),
            keeps: Vec::new(),
            release: None,
            warnings: Vec::new(),
            error: None,
            op: s("op"),
        });
    }
    // Operations; a job from before they existed has the one every job starts with.
    let ops: Vec<crate::op::Operation> = v
        .get("ops")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1000)
        .map(crate::op::Operation::from_json)
        .collect::<EngineResult<_>>()?;
    if !ops.is_empty() {
        job.ops = ops;
    }
    let first = job.ops.first().map(|o| o.id.clone()).unwrap_or_else(|| crate::op::FIRST.into());
    for c in &mut job.cuts {
        if !job.ops.iter().any(|o| o.id == c.op) {
            c.op = first.clone();
        }
    }
    job.active_op = first;
    // Ids stay unique even when the counter was lost.
    let highest = job
        .shapes
        .iter()
        .map(|s| s.id.as_str())
        .chain(job.cuts.iter().map(|c| c.id.as_str()))
        .chain(job.models.iter().map(|m| m.id.as_str()))
        .chain(job.ops.iter().map(|o| o.id.as_str()))
        .filter_map(|id| id.rsplit('.').next()?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    job.next_id = v.get("next_id").and_then(Value::as_u64).unwrap_or(1).max(highest + 1);
    job.rebuild();
    Ok(job)
}

pub fn read(path: &str) -> EngineResult<JobState> {
    let bytes = CosmicCut_io::files::read(path, MAX_BYTES)
        .map_err(|e| EngineError::InvalidParams(format!("could not read {path}: {e}")))?
        .ok_or_else(|| EngineError::InvalidParams(format!("{path} is too large to be a job")))?;
    let text = String::from_utf8(bytes).map_err(|_| EngineError::InvalidParams(format!("{path} is not text")))?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| EngineError::InvalidParams(format!("{path} is not valid JSON: {e}")))?;
    let mut job = from_json(&v)?;
    job.path = Some(path.to_owned());
    Ok(job)
}

pub fn write(job: &JobState, path: &str) -> EngineResult<()> {
    let text = serde_json::to_string_pretty(&to_json(job))
        .map_err(|e| EngineError::Internal(format!("could not write the job: {e}")))?;
    CosmicCut_io::files::write(path, text.as_bytes())
        .map_err(|e| EngineError::InvalidParams(format!("could not write {path}: {e}")))
}
