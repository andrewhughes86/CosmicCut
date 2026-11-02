//! Post commands: the program as G-code, to look at or to write out.

use serde_json::json;

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab};
use crate::error::EngineError;
use crate::params;
use crate::post;
use crate::session::{Pane, UiRequest};

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "post.settings",
        label: "Post processor",
        tooltip: "How this machine's programs are written: start (safety) block, wire on and off codes, end code, \
                  G words on every line, line numbers, units and more. Changes the post in the library, for every \
                  machine that uses it",
        placement: Some(Placement::large(RibbonTab::Machine, "Post")),
        icon: Some("post_settings"),
        shortcut: None,
        params: post::PARAMS,
        returns: "post settings",
        enabled: |_| true,
        run: |s, p| {
            let name = match params::opt_str(p, "name")? {
                Some(n) => n.to_owned(),
                None => {
                    // A job from elsewhere may name a post this library does not have: keep it.
                    let m = s.job.machine.clone();
                    if s.library.post(&m.post_name).is_none() {
                        s.library.posts.push(crate::library::NamedPost { name: m.post_name.clone(), settings: m.post });
                    }
                    m.post_name
                }
            };
            let mut settings = p.clone();
            if let Some(o) = settings.as_object_mut() {
                o.remove("name");
            }
            Ok(super::machine::set_post(s, &name, &settings)?.to_json())
        },
    },
    CommandSpec {
        id: "post.gcode.preview",
        label: "View G-code",
        tooltip: "Post the program and show the G-code",
        placement: Some(Placement::large(RibbonTab::Machine, "Post")),
        icon: None,
        shortcut: None,
        params: &[],
        returns: "{text, lines, seconds, warnings}",
        enabled: super::has_cuts,
        run: |s, _| {
            let posted = post::post(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops, &s.job.name);
            s.ui.requests.push(UiRequest::FocusPane(Pane::Gcode));
            let mut v = posted.summary_json();
            v["text"] = json!(posted.text);
            Ok(v)
        },
    },
    CommandSpec {
        id: "post.gcode.export",
        label: "Post G-code",
        tooltip: "Write the program to a G-code file for the machine",
        placement: Some(Placement::large(RibbonTab::Machine, "Post")),
        icon: None,
        shortcut: Some("Ctrl+P"),
        params: &[Param::required("path", ParamKind::Str, "Where to write it (.ngc, .nc, .tap…)")],
        returns: "{path, lines, seconds, warnings}",
        enabled: super::has_cuts,
        run: |s, p| {
            let path = params::req_str(p, "path")?.to_owned();
            let ext = s.job.machine.post.extension.clone();
            let path = if std::path::Path::new(&path).extension().is_none() { format!("{path}.{ext}") } else { path };
            let posted = post::post(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops, &s.job.name);
            CosmicCut_io::files::write(&path, posted.text.as_bytes())
                .map_err(|e| EngineError::InvalidParams(format!("could not write {path}: {e}")))?;
            let mut v = posted.summary_json();
            v["path"] = json!(path);
            Ok(v)
        },
    },
    CommandSpec {
        id: "post.limits.check",
        label: "Check limits",
        tooltip: "Check the program stays inside the machine's travel",
        placement: None,
        icon: Some("post_limits_check"),
        shortcut: None,
        params: &[],
        returns: "{problems: [text]}",
        enabled: super::has_cuts,
        run: |s, _| {
            let posted = post::post(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops, &s.job.name);
            Ok(json!({ "problems": posted.warnings }))
        },
    },
];
