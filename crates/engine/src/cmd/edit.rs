//! Edit commands: undo and redo the changes to the job, and cut, copy and paste cuts, shapes and
//! models. They are always enabled; with nothing to act on they say why.

use serde_json::{Value, json};

use super::CommandSpec;
use crate::error::{EngineError, EngineResult};
use crate::session::{Clip, Session};

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        tooltip: "Take back the last change to the job",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+Z"),
        params: &[],
        returns: "{\"undo\": n, \"redo\": n}",
        enabled: always,
        run: |s, _| {
            let earlier =
                s.undo.pop().ok_or_else(|| EngineError::InvalidParams("there is nothing to undo yet".into()))?;
            let now = std::mem::replace(&mut s.job, earlier);
            restored(s, &now);
            s.redo.push(now);
            Ok(counts(s))
        },
    },
    CommandSpec {
        id: "edit.redo",
        label: "Redo",
        tooltip: "Make the last change undone again",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+Y"),
        params: &[],
        returns: "{\"undo\": n, \"redo\": n}",
        enabled: always,
        run: |s, _| {
            let later = s.redo.pop().ok_or_else(|| EngineError::InvalidParams("there is nothing to redo".into()))?;
            let now = std::mem::replace(&mut s.job, later);
            restored(s, &now);
            s.undo.push(now);
            Ok(counts(s))
        },
    },
    CommandSpec {
        id: "edit.cut",
        label: "Cut",
        tooltip: "Remove the selected cut, shape or model and keep it for Paste",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+X"),
        params: &[],
        returns: "{\"cut\": id}",
        enabled: always,
        run: |s, _| {
            let id = copy(s)?;
            if s.job.cuts.iter().any(|c| c.id == id) {
                s.job.cuts.retain(|c| c.id != id);
            } else {
                s.job.shapes.retain(|sh| sh.id != id);
                s.job.models.retain(|m| m.id != id);
                s.job.rebuild();
            }
            s.view.selection = None;
            s.job.touch();
            s.sim = None;
            Ok(json!({ "cut": id }))
        },
    },
    CommandSpec {
        id: "edit.duplicate",
        label: "Duplicate",
        tooltip: "Copy the selected cut (after it) or model (beside it)",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+D"),
        params: &[],
        returns: "the copy",
        enabled: always,
        run: |s, _| {
            let id = if s.selected_cut().is_some() {
                "cut.duplicate"
            } else if s.selected_model().is_some() {
                "model.duplicate"
            } else {
                return Err(EngineError::InvalidParams("select a cut or a model first".into()));
            };
            let spec = super::find(id).ok_or_else(|| EngineError::UnknownCommand(id.into()))?;
            (spec.run)(s, &Value::Null)
        },
    },
    CommandSpec {
        id: "edit.copy",
        label: "Copy",
        tooltip: "Keep the selected cut, shape or model for Paste",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+C"),
        params: &[],
        returns: "{\"copied\": id}",
        enabled: always,
        run: |s, _| Ok(json!({ "copied": copy(s)? })),
    },
    CommandSpec {
        id: "edit.paste",
        label: "Paste",
        tooltip: "Add a copy of what was cut or copied (a cut goes into the active operation)",
        placement: None,
        icon: None,
        shortcut: Some("Ctrl+V"),
        params: &[],
        returns: "{\"pasted\": id}",
        enabled: always,
        run: |s, _| {
            let clip = s.clipboard.clone().ok_or_else(|| {
                EngineError::InvalidParams("nothing to paste: cut or copy a cut, shape or model first".into())
            })?;
            let id = match clip {
                Clip::Cut(c) => {
                    let mut c = *c;
                    c.id = s.job.fresh_id("cut");
                    c.name = s.job.fresh_name(&c.name);
                    c.op = s.job.active_op.clone();
                    let id = c.id.clone();
                    s.job.cuts.push(c);
                    // Its shape or model may have changed (or gone) since it was copied.
                    s.job.rebuild();
                    id
                }
                Clip::Shape(sh) => {
                    let mut sh = *sh;
                    sh.id = s.job.fresh_id("shape");
                    sh.name = s.job.fresh_name(&sh.name);
                    let id = sh.id.clone();
                    s.job.shapes.push(sh);
                    id
                }
                // A pasted model stands beside where it was copied from, not on top of it.
                Clip::Model(m) => super::model::add_beside(s, *m),
            };
            s.view.selection = Some(id.clone());
            s.job.touch();
            s.sim = None;
            Ok(json!({ "pasted": id }))
        },
    },
];

fn always(_: &Session) -> bool {
    true
}

/// Puts the selected cut, shape or model on the clipboard; returns its id.
fn copy(s: &mut Session) -> EngineResult<String> {
    let (clip, id) = if let Some(c) = s.selected_cut() {
        (Clip::Cut(Box::new(c.clone())), c.id.clone())
    } else if let Some(sh) = s.selected_shape() {
        (Clip::Shape(Box::new(sh.clone())), sh.id.clone())
    } else if let Some(m) = s.selected_model() {
        (Clip::Model(Box::new(m.clone())), m.id.clone())
    } else {
        return Err(EngineError::InvalidParams("select a cut, shape or model in the explorer first".into()));
    };
    s.clipboard = Some(clip);
    Ok(id)
}

/// After the job was swapped for one from the history: drawn again, nothing stale left over.
fn restored(s: &mut Session, replaced: &crate::session::JobState) {
    s.job.revision = replaced.revision.wrapping_add(1);
    s.job.generation = replaced.generation.wrapping_add(1);
    s.sim = None;
    if let Some(sel) = s.view.selection.clone()
        && s.selection_node(&sel).is_none()
    {
        s.view.selection = None;
    }
}

fn counts(s: &Session) -> Value {
    json!({ "undo": s.undo.len(), "redo": s.redo.len() })
}
