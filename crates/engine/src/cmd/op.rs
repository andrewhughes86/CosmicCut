//! Operation commands: add operations, set their machine stops and messages, move cuts between
//! them.

use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::{EngineError, EngineResult};
use crate::op::{Operation, Stop};
use crate::params;
use crate::session::Session;

const SETTINGS: &[Param] = &[
    Param::optional("name", ParamKind::Str, "What the operation is for, e.g. Turn the stock a quarter"),
    Param::optional(
        "stop",
        ParamKind::Choice(Stop::IDS),
        "The machine stop when the operation starts: none, m0 (stop until resumed) or m1 (optional stop)",
    ),
    Param::optional("message", ParamKind::Str, "Shown to the operator on the controller when the operation starts"),
    Param::optional("notes", ParamKind::Str, "For the operator: written into the program as comments"),
    Param::optional(
        "ignore",
        ParamKind::StrList { max: 64 },
        "Models taken off the table before this operation: its cuts' wire may pass through them",
    ),
];

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "op.add",
        label: "Add operation",
        tooltip: "Start a new operation: the cuts after a machine stop, e.g. once the operator has turned the stock \
                  by hand. New cuts go into it",
        placement: Some(Placement::large(RibbonTab::Cuts, "Operations")),
        icon: None,
        shortcut: None,
        params: SETTINGS,
        returns: "operation",
        enabled: always,
        run: |s, p| {
            let mut op = Operation { id: s.job.fresh_id("op"), stop: Stop::Pause, ..Operation::default() };
            op.apply(p)?;
            let out = op.to_json();
            s.job.active_op = op.id.clone();
            s.view.selection = Some(op.id.clone());
            s.job.ops.push(op);
            changed(s);
            Ok(out)
        },
    },
    CommandSpec {
        id: "op.edit",
        label: "Operation settings",
        tooltip: "The selected operation's machine stop (M0 or M1), message to the operator and notes",
        placement: Some(Placement::large(RibbonTab::Cuts, "Operations")),
        icon: Some("op_settings"),
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "The operation (default the one new cuts go into)"),
            SETTINGS[0],
            SETTINGS[1],
            SETTINGS[2],
            SETTINGS[3],
            SETTINGS[4],
        ],
        returns: "operation",
        enabled: always,
        run: |s, p| {
            let i = index(s, p)?;
            let mut settings = p.clone();
            if let Some(o) = settings.as_object_mut() {
                o.remove("id");
            }
            let op = s.job.ops.get_mut(i).ok_or_else(missing)?;
            op.apply(&settings)?;
            let out = op.to_json();
            changed(s);
            Ok(out)
        },
    },
    CommandSpec {
        id: "op.remove",
        label: "Delete operation",
        tooltip: "Delete an operation; its cuts join the operation before it",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("id", ParamKind::Str, "The operation")],
        returns: "{\"removed\": id}",
        enabled: |s| s.job.ops.len() > 1,
        run: |s, p| {
            let i = index(s, p)?;
            if s.job.ops.len() < 2 {
                return Err(EngineError::InvalidParams("a job has at least one operation".into()));
            }
            let gone = s.job.ops.remove(i);
            let into = s.job.ops.get(i.saturating_sub(1)).map(|o| o.id.clone()).unwrap_or_default();
            for c in s.job.cuts.iter_mut().filter(|c| c.op == gone.id) {
                c.op = into.clone();
            }
            if s.job.active_op == gone.id {
                s.job.active_op = into;
            }
            if s.view.selection.as_deref() == Some(gone.id.as_str()) {
                s.view.selection = None;
            }
            changed(s);
            Ok(json!({ "removed": gone.id }))
        },
    },
    CommandSpec {
        id: "op.activate",
        label: "Add cuts here",
        tooltip: "Make new cuts go into this operation",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("id", ParamKind::Str, "The operation")],
        returns: "operation",
        enabled: always,
        run: |s, p| {
            let i = index(s, p)?;
            let op = s.job.ops.get(i).ok_or_else(missing)?;
            s.job.active_op = op.id.clone();
            s.job.revision += 1;
            Ok(op.to_json())
        },
    },
    CommandSpec {
        id: "cut.op.set",
        label: "Move to operation",
        tooltip: "Move a cut into another operation",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The cut"),
            Param::required("op", ParamKind::Str, "The operation"),
        ],
        returns: "cut",
        enabled: super::has_cuts,
        run: |s, p| {
            let op = params::req_str(p, "op")?.to_owned();
            if s.job.op_index(&op).is_none() {
                return Err(EngineError::InvalidParams(format!("there is no operation `{op}`")));
            }
            let id = params::req_str(p, "id")?;
            let c = s
                .job
                .cuts
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| EngineError::InvalidParams(format!("there is no cut `{id}`")))?;
            c.op = op;
            let out = json!({ "id": c.id, "op": c.op });
            changed(s);
            Ok(out)
        },
    },
    CommandSpec {
        id: "cut.place",
        label: "Place cut",
        tooltip: "Put a cut into an operation, before another of its cuts or after the last (dragging it in the \
                  explorer)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::required("id", ParamKind::Str, "The cut"),
            Param::optional("op", ParamKind::Str, "The operation (default the one `before` is in, else its own)"),
            Param::optional("before", ParamKind::Str, "The cut it goes before (default: after the operation's last)"),
        ],
        returns: "[{id, op} in order]",
        enabled: super::has_cuts,
        run: |s, p| {
            let id = params::req_str(p, "id")?.to_owned();
            let before = params::opt_str(p, "before")?.map(str::to_owned).filter(|b| *b != id);
            let i = s
                .job
                .cuts
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| EngineError::InvalidParams(format!("there is no cut `{id}`")))?;
            let own = s.job.cuts.get(i).map(|c| c.op.clone()).unwrap_or_default();
            let next_op = before.as_deref().and_then(|b| s.job.cut(b)).map(|c| c.op.clone());
            if let Some(b) = &before
                && next_op.is_none()
            {
                return Err(EngineError::InvalidParams(format!("there is no cut `{b}`")));
            }
            let op = match params::opt_str(p, "op")? {
                Some(op) => op.to_owned(),
                None => next_op.clone().unwrap_or(own),
            };
            if s.job.op_index(&op).is_none() {
                return Err(EngineError::InvalidParams(format!("there is no operation `{op}`")));
            }
            if next_op.as_ref().is_some_and(|o| *o != op) {
                return Err(EngineError::InvalidParams("`before` is a cut in another operation".into()));
            }
            let mut cut = s.job.cuts.remove(i);
            cut.op = op.clone();
            let at = match &before {
                Some(b) => s.job.cuts.iter().position(|c| c.id == *b).unwrap_or(s.job.cuts.len()),
                // After the operation's last cut, else where the operations after it start.
                None => match s.job.cuts.iter().rposition(|c| c.op == op) {
                    Some(k) => k + 1,
                    None => {
                        let rank = |o: &str| s.job.op_index(o).unwrap_or(usize::MAX);
                        let mine = rank(&op);
                        s.job.cuts.iter().position(|c| rank(&c.op) > mine).unwrap_or(s.job.cuts.len())
                    }
                },
            };
            s.job.cuts.insert(at.min(s.job.cuts.len()), cut);
            changed(s);
            Ok(json!(s.job.cuts.iter().map(|c| json!({ "id": c.id, "op": c.op })).collect::<Vec<_>>()))
        },
    },
];

fn missing() -> EngineError {
    EngineError::InvalidParams("there is no such operation".into())
}

/// The operation `id` names, else the one new cuts go into.
fn index(s: &Session, p: &Value) -> EngineResult<usize> {
    let id = params::opt_str(p, "id")?.map_or_else(|| s.job.active_op.clone(), str::to_owned);
    s.job.op_index(&id).ok_or_else(|| EngineError::InvalidParams(format!("there is no operation `{id}`")))
}

fn changed(s: &mut Session) {
    // What an operation ignores changes what its cuts may do.
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
}
