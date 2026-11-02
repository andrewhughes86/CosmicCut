//! Job commands: new, open, save, close, the sample job, and the tabs the open jobs are in.

use glam::{DVec2, DVec3};
use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, always};
use crate::error::{EngineError, EngineResult};
use crate::jobfile;
use crate::params;
use crate::session::{JobState, Session};
use crate::shape::Shape;
use crate::stock::Stock;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "job.new",
        label: "New",
        tooltip: "Start a new job, in a new tab, with the same machine",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+N"),
        params: &[],
        returns: "the job",
        enabled: always,
        run: |s, _| {
            let machine = s.job.machine.clone();
            s.open_tab(JobState { machine, ..JobState::default() }, false)?;
            Ok(job_json(s))
        },
    },
    CommandSpec {
        id: "job.close",
        label: "Close",
        tooltip: "Close the job and its tab; closing the last one leaves an empty job",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+W"),
        params: &[],
        returns: "the job",
        enabled: always,
        run: |s, _| {
            if s.close_tab() {
                return Ok(job_json(s));
            }
            // The only tab: an empty job with the same machine, which Undo can take back.
            let revision = s.job.revision.wrapping_add(1000);
            let generation = s.job.generation + 1;
            let machine = s.job.machine.clone();
            s.job = JobState { machine, revision, generation, ..JobState::default() };
            s.sim = None;
            s.view.selection = None;
            Ok(job_json(s))
        },
    },
    CommandSpec {
        id: "job.open",
        label: "Open",
        tooltip: "Open a CosmicCut job (.ccut) in a new tab",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+O"),
        params: &[Param::required("path", ParamKind::Str, "The job file")],
        returns: "the job",
        enabled: always,
        run: |s, p| {
            let path = params::req_str(p, "path")?;
            // A job already open is brought to the front, not opened twice.
            if let Some(tab) = s.tab_with_path(path) {
                s.switch_tab(tab)?;
                return Ok(job_json(s));
            }
            let job = jobfile::read(path)?;
            s.open_tab(job, true)?;
            Ok(job_json(s))
        },
    },
    CommandSpec {
        id: "job.save",
        label: "Save",
        tooltip: "Save the job to its file",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+S"),
        params: &[Param::optional("path", ParamKind::Str, "Where to save; the job's own file if left out")],
        returns: "{path}",
        enabled: always,
        run: save,
    },
    CommandSpec {
        id: "job.save.as",
        label: "Save as",
        tooltip: "Save the job to a new file",
        placement: None,
        icon: Some("job_save"),
        shortcut: Some("Ctrl+Shift+S"),
        params: &[Param::required("path", ParamKind::Str, "Where to save")],
        returns: "{path}",
        enabled: always,
        run: save,
    },
    CommandSpec {
        id: "job.sample",
        label: "Sample",
        tooltip: "Open the sample job in a new tab: a low-poly penguin cut from a foam block",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "the job",
        enabled: always,
        run: |s, _| {
            load_sample(s)?;
            Ok(job_json(s))
        },
    },
    CommandSpec {
        id: "job.tab.set",
        label: "Switch job",
        tooltip: "Bring the job in tab `index` (from 0, left to right) to the front",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("index", ParamKind::Integer { min: 0, max: 31 }, "The tab, from 0")],
        returns: "{tabs: [{name, path, modified}], active}",
        enabled: always,
        run: |s, p| {
            let index = params::opt_f64(p, "index", 0.0, 31.0)?.unwrap_or(0.0).round();
            s.switch_tab(index as usize)?;
            Ok(tabs_json(s))
        },
    },
    CommandSpec {
        id: "job.tab.next",
        label: "Next job",
        tooltip: "Bring the job in the next tab to the front",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+Tab"),
        params: &[],
        returns: "{tabs: [{name, path, modified}], active}",
        enabled: always,
        run: |s, _| {
            let count = s.tabs().len().max(1);
            s.switch_tab((s.active_tab() + 1) % count)?;
            Ok(tabs_json(s))
        },
    },
    CommandSpec {
        id: "job.tabs.get",
        label: "Open jobs",
        tooltip: "The jobs open in tabs, and which is current",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "{tabs: [{name, path, modified}], active}",
        enabled: always,
        run: |s, _| Ok(tabs_json(s)),
    },
    CommandSpec {
        id: "job.rename",
        label: "Rename job",
        tooltip: "Rename the job",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("name", ParamKind::Str, "New name"),
            Param::optional("id", ParamKind::Str, "Ignored: there is one job (the explorer sends its row's id)"),
        ],
        returns: "the job",
        enabled: always,
        run: |s, p| {
            let name = params::req_str(p, "name")?.trim();
            if name.is_empty() {
                return Err(EngineError::InvalidParams("`name` must not be empty".into()));
            }
            s.job.name = name.chars().take(200).collect();
            s.job.touch();
            Ok(job_json(s))
        },
    },
    CommandSpec {
        id: "job.get",
        label: "Job",
        tooltip: "The job: machine, stock, shapes and cuts",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "the job",
        enabled: always,
        run: |s, _| Ok(job_json(s)),
    },
];

fn save(s: &mut Session, p: &Value) -> EngineResult<Value> {
    let path = match params::opt_str(p, "path")? {
        Some(path) => path.to_owned(),
        None => s.job.path.clone().ok_or_else(|| EngineError::InvalidParams("`path` is required".into()))?,
    };
    let path =
        if std::path::Path::new(&path).extension().is_none() { format!("{path}.{}", jobfile::EXTENSION) } else { path };
    jobfile::write(&s.job, &path)?;
    if s.job.name == "Untitled"
        && let Some(stem) = std::path::Path::new(&path).file_stem().and_then(|n| n.to_str())
    {
        s.job.name = stem.to_owned();
    }
    s.job.path = Some(path.clone());
    s.job.saved_edits = s.job.edits;
    Ok(json!({ "path": path }))
}

fn tabs_json(s: &Session) -> Value {
    let tabs: Vec<Value> =
        s.tabs().iter().map(|t| json!({ "name": t.name, "path": t.path, "modified": t.modified })).collect();
    json!({ "tabs": tabs, "active": s.active_tab() })
}

pub fn job_json(s: &Session) -> Value {
    let j = &s.job;
    json!({
        "name": j.name,
        "path": j.path,
        "modified": j.is_modified(),
        "machine": j.machine.to_json(),
        "stock": j.stock.to_json(),
        "shapes": j.shapes.iter().map(Shape::to_json).collect::<Vec<_>>(),
        "cuts": j.cuts.iter().map(|c| c.summary_json(&j.machine)).collect::<Vec<_>>(),
    })
}

/// The sample job: a chubby low-poly penguin stood in a 13 × 11 × 15 in block, an inch up so
/// the wire can cut under its feet, cut from the model with a profile every 22.5°, the part held
/// on the foam until it is parted off at the end.
pub fn load_sample(s: &mut Session) -> EngineResult<()> {
    use crate::model::{self, Model};
    let machine = s.job.machine.clone();
    let mut job = JobState { name: "Sample penguin".into(), machine, ..JobState::default() };
    job.stock = Stock::block(DVec3::new(330.2, 279.4, 381.0), DVec2::ZERO, 0.0, 0.0)?;
    let (mesh, _) = model::read(model::SAMPLE_PENGUIN, Some("mm"))?;
    let id = job.fresh_id("model");
    let mut penguin = Model::new(id.clone(), "Penguin".into(), model::SAMPLE_PENGUIN.into(), Some("mm".into()), mesh)?;
    penguin.lift = 25.4;
    penguin.place(job.stock.bottom());
    job.models.push(penguin);
    s.open_tab(job, true)?;
    super::cut::create(s, crate::cut::CutKind::Model, &json!({ "name": "Penguin", "model": id, "divisions": 8 }))?;
    s.view.selection = None;
    // A fresh sample has nothing to save.
    s.job.saved_edits = s.job.edits;
    Ok(())
}
