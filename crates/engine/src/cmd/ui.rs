//! Ribbon, panes, selection and preferences.

use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::EngineError;
use crate::params;
use crate::session::{Pane, Session, UiRequest};
use crate::units::DisplayUnits;

macro_rules! pane_command {
    ($id:literal, $label:literal, $tooltip:literal, $request:path) => {
        CommandSpec {
            id: $id,
            label: $label,
            tooltip: $tooltip,
            placement: None,
            icon: None,
            shortcut: None,
            params: &[Param::required("pane", ParamKind::Choice(Pane::IDS), "Pane id")],
            returns: "{\"pane\": id}",
            enabled: always,
            run: |s, p| {
                let id = params::req_str(p, "pane")?;
                let pane =
                    Pane::from_id(id).ok_or_else(|| EngineError::InvalidParams(format!("unknown pane `{id}`")))?;
                s.ui.requests.push($request(pane));
                Ok(json!({ "pane": pane.id() }))
            },
        }
    };
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "ui.ribbon.tab.set",
        label: "Ribbon tab",
        tooltip: "Show a ribbon tab",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("tab", ParamKind::Choice(RibbonTab::IDS), "Tab id")],
        returns: "{\"tab\": id}",
        enabled: always,
        run: |s, p| {
            let id = params::req_str(p, "tab")?;
            let tab =
                RibbonTab::from_id(id).ok_or_else(|| EngineError::InvalidParams(format!("unknown tab `{id}`")))?;
            s.ui.ribbon_tab = tab;
            Ok(json!({ "tab": tab.id() }))
        },
    },
    pane_command!("ui.pane.show", "Show pane", "Open a workspace pane", UiRequest::ShowPane),
    pane_command!("ui.pane.hide", "Hide pane", "Close a workspace pane", UiRequest::HidePane),
    pane_command!("ui.pane.focus", "Focus pane", "Open a pane and bring it to the front", UiRequest::FocusPane),
    CommandSpec {
        id: "selection.set",
        label: "Select",
        tooltip: "Select an item of the job by id (none clears the selection)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("id", ParamKind::Str, "Item id, e.g. cut.3; omit to clear"),
            Param::optional(
                "add",
                ParamKind::Bool,
                "Keep what is selected and add the item, or take it away if it is selected (Ctrl-click)",
            ),
            Param::optional(
                "ids",
                ParamKind::StrList { max: 10_000 },
                "Select these items too, with `id` (Shift-click: the rows from the last one selected)",
            ),
        ],
        returns: "{\"selection\": id or null, \"selected\": [ids]}",
        enabled: always,
        run: |s, p| {
            let mut id = params::opt_str(p, "id")?.map(str::to_owned);
            let extra: Vec<String> = p
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            for i in id.iter().chain(&extra) {
                if s.selection_node(i).is_none() {
                    return Err(EngineError::InvalidParams(format!("nothing has the id `{i}`")));
                }
            }
            let mut all = if params::opt_bool(p, "add")?.unwrap_or(false) { s.selected_ids() } else { Vec::new() };
            match &id {
                Some(i) if all.contains(i) && extra.is_empty() => {
                    // Ctrl-click on a selected item lets it go.
                    all.retain(|x| x != i);
                    id = all.last().cloned();
                }
                Some(i) => {
                    for x in extra.iter().chain(std::iter::once(i)) {
                        if !all.contains(x) {
                            all.push(x.clone());
                        }
                    }
                }
                None => all.clear(),
            }
            s.view.selected = all;
            // Selecting an operation, or a cut in one, makes it the one new cuts go into.
            if let Some(id) = &id {
                let op = if s.job.op_index(id).is_some() {
                    Some(id.clone())
                } else {
                    s.job.cuts.iter().find(|c| &c.id == id).map(|c| c.op.clone())
                };
                if let Some(op) = op.filter(|op| *op != s.job.active_op) {
                    s.job.active_op = op;
                    s.job.revision += 1;
                }
            }
            s.view.selection = id;
            Ok(json!({ "selection": s.view.selection, "selected": s.selected_ids() }))
        },
    },
    CommandSpec {
        id: "command.list",
        label: "Commands",
        tooltip: "Every command, with its parameters",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "[command]",
        enabled: always,
        run: |s, _| Ok(Value::Array(super::all().map(|c| c.to_json(s)).collect())),
    },
    CommandSpec {
        id: "settings.open",
        label: "Preferences",
        tooltip: "Units, theme and layout",
        placement: Some(Placement::large(RibbonTab::Home, "Preferences")),
        icon: Some("settings_open"),
        shortcut: Some("Ctrl+,"),
        params: &[],
        returns: "preferences {units, theme}",
        enabled: always,
        run: |s, _| Ok(prefs_json(s)),
    },
    CommandSpec {
        id: "settings.units.set",
        label: "Units",
        tooltip: "Show and type lengths in inches or millimetres (commands always take millimetres)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("units", ParamKind::Choice(DisplayUnits::IDS), "in or mm")],
        returns: "preferences {units, theme}",
        enabled: always,
        run: |s, p| {
            let id = params::req_str(p, "units")?;
            s.prefs.units =
                DisplayUnits::from_id(id).ok_or_else(|| EngineError::InvalidParams(format!("unknown units `{id}`")))?;
            s.prefs.revision = s.prefs.revision.wrapping_add(1);
            s.job.revision += 1;
            Ok(prefs_json(s))
        },
    },
];

fn prefs_json(s: &Session) -> Value {
    json!({ "units": s.prefs.units.id(), "theme": s.prefs.theme.id(), "display": s.prefs.display.to_json() })
}
