//! Machine and post processor commands: the job's machine, the machine library, and the post
//! processor library.

use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::{EngineError, EngineResult};
use crate::library::{NamedPost, check_name};
use crate::machine::{Machine, Origin, PARAMS};
use crate::params;
use crate::post::PostSettings;
use crate::session::Session;

const NAME: Param = Param::required("id", ParamKind::Str, "The library entry, by name");

pub(super) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "machine.edit",
        label: "Machine",
        tooltip: "This job's machine: travel, where the turntable is, the wire, rates, and its post processor. \
                  The machine library keeps the changes too",
        placement: Some(Placement::large(RibbonTab::Machine, "Machine")),
        icon: Some("machine_edit"),
        shortcut: None,
        params: PARAMS,
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let old = s.job.machine.name.clone();
            let mut m = s.job.machine.clone();
            m.apply(p)?;
            if let Some(post) = params::opt_str(p, "post")? {
                let named = s
                    .library
                    .post(post)
                    .ok_or_else(|| EngineError::InvalidParams(format!("there is no post processor `{post}`")))?;
                m.post_name = named.name.clone();
                m.post = named.settings.clone();
            }
            if m.name != old && s.library.machine(&m.name).is_some() {
                return Err(EngineError::InvalidParams(format!("the library already has a machine `{}`", m.name)));
            }
            // The library's copy follows (it is added if the job's machine was not in it).
            match s.library.machines.iter_mut().find(|x| x.name == old) {
                Some(slot) => *slot = m.clone(),
                None => s.library.machines.push(m.clone()),
            }
            s.library.touch();
            set_machine(s, m);
            Ok(s.job.machine.to_json())
        },
    },
    CommandSpec {
        id: "origin.set",
        label: "Origin",
        tooltip: "Where the program's X0 Y0 is: machine zero, or the turntable's axis at the table top",
        placement: None,
        icon: Some("job_origin"),
        shortcut: None,
        params: &[
            Param::required("at", ParamKind::Choice(Origin::IDS), "machine (machine 0,0) or table (turntable 0,0)"),
            Param::optional("id", ParamKind::Str, "Ignored (the explorer sends its row's id)"),
        ],
        returns: "{at}",
        enabled: always,
        run: |s, p| {
            let at = params::req_str(p, "at")?;
            let origin = Origin::from_id(at)
                .ok_or_else(|| EngineError::InvalidParams(format!("`at` must be machine or table, not `{at}`")))?;
            if origin != s.job.origin {
                s.job.origin = origin;
                s.job.touch();
            }
            Ok(json!({ "at": origin.id() }))
        },
    },
    CommandSpec {
        id: "machine.library",
        label: "Machine library",
        tooltip: "The machines kept between jobs: choose the one this job runs on, add, copy, rename and delete them",
        placement: Some(Placement::large(RibbonTab::Machine, "Machine")),
        icon: Some("machine_library"),
        shortcut: None,
        params: &[],
        returns: "[machine]",
        enabled: always,
        run: |s, _| Ok(json!(s.library.machines.iter().map(Machine::to_json).collect::<Vec<_>>())),
    },
    CommandSpec {
        id: "machine.library.use",
        label: "Use machine",
        tooltip: "Run this job on a machine from the library",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME],
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let m = machine(s, p)?.clone();
            set_machine(s, m);
            Ok(s.job.machine.to_json())
        },
    },
    CommandSpec {
        id: "machine.library.new",
        label: "New machine",
        tooltip: "Add a machine to the library with the default settings",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::optional("name", ParamKind::Str, "Its name")],
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let name = match params::opt_str(p, "name")? {
                Some(n) => unique(s, check_name(n)?)?,
                None => s.library.fresh_name("New machine", false),
            };
            let mut m = Machine { name, ..Machine::default() };
            if let Some(post) = s.library.posts.first() {
                m.post_name = post.name.clone();
                m.post = post.settings.clone();
            }
            let out = m.to_json();
            s.library.machines.push(m);
            s.library.touch();
            Ok(out)
        },
    },
    CommandSpec {
        id: "machine.library.duplicate",
        label: "Copy machine",
        tooltip: "Copy a machine in the library",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME],
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let mut m = machine(s, p)?.clone();
            m.name = s.library.fresh_name(&format!("{} copy", m.name), false);
            let out = m.to_json();
            s.library.machines.push(m);
            s.library.touch();
            Ok(out)
        },
    },
    CommandSpec {
        id: "machine.library.rename",
        label: "Rename machine",
        tooltip: "Rename a machine in the library",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME, Param::required("name", ParamKind::Str, "New name")],
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let old = machine(s, p)?.name.clone();
            let new = check_name(params::req_str(p, "name")?)?;
            if new != old {
                unique(s, new.clone())?;
            }
            if let Some(m) = s.library.machines.iter_mut().find(|m| m.name == old) {
                m.name = new.clone();
            }
            if s.job.machine.name == old {
                s.job.machine.name = new.clone();
                s.job.touch();
            }
            s.library.touch();
            Ok(s.library.machine(&new).map(Machine::to_json).unwrap_or_default())
        },
    },
    CommandSpec {
        id: "machine.library.remove",
        label: "Delete machine",
        tooltip: "Delete a machine from the library (this job keeps its copy)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME],
        returns: "{\"removed\": name}",
        enabled: |s| s.library.machines.len() > 1,
        run: |s, p| {
            let name = machine(s, p)?.name.clone();
            if s.library.machines.len() < 2 {
                return Err(EngineError::InvalidParams("the library keeps at least one machine".into()));
            }
            s.library.machines.retain(|m| m.name != name);
            s.library.touch();
            Ok(json!({ "removed": name }))
        },
    },
    CommandSpec {
        id: "post.library",
        label: "Post processor library",
        tooltip: "The post processors kept between jobs: edit them, add your own, and choose the one each machine uses",
        placement: Some(Placement::large(RibbonTab::Machine, "Post")),
        icon: Some("post_library"),
        shortcut: None,
        params: &[],
        returns: "[post]",
        enabled: always,
        run: |s, _| Ok(json!(s.library.posts.iter().map(post_json).collect::<Vec<_>>())),
    },
    CommandSpec {
        id: "post.library.use",
        label: "Use post processor",
        tooltip: "Write this job's machine's programs with a post processor from the library",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME],
        returns: "machine",
        enabled: always,
        run: |s, p| {
            let post = post(s, p)?.name.clone();
            let params = json!({ "post": post });
            (super::find("machine.edit").ok_or_else(|| EngineError::Internal("no machine.edit".into()))?.run)(
                s, &params,
            )
        },
    },
    CommandSpec {
        id: "post.library.new",
        label: "New post processor",
        tooltip: "Add a post processor to the library, a copy of another (default this machine's)",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[
            Param::optional("name", ParamKind::Str, "Its name"),
            Param::optional("from", ParamKind::Str, "The post processor to copy"),
        ],
        returns: "post",
        enabled: always,
        run: |s, p| {
            let settings = match params::opt_str(p, "from")? {
                Some(from) => s
                    .library
                    .post(from)
                    .map(|n| n.settings.clone())
                    .ok_or_else(|| EngineError::InvalidParams(format!("there is no post processor `{from}`")))?,
                None => s.job.machine.post.clone(),
            };
            let name = match params::opt_str(p, "name")? {
                Some(n) => {
                    let n = check_name(n)?;
                    if s.library.post(&n).is_some() {
                        return Err(EngineError::InvalidParams(format!("there is already a post processor `{n}`")));
                    }
                    n
                }
                None => s.library.fresh_name("My post", true),
            };
            let named = NamedPost { name, settings };
            let out = post_json(&named);
            s.library.posts.push(named);
            s.library.touch();
            Ok(out)
        },
    },
    CommandSpec {
        id: "post.library.rename",
        label: "Rename post processor",
        tooltip: "Rename a post processor; machines using it follow",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME, Param::required("name", ParamKind::Str, "New name")],
        returns: "post",
        enabled: always,
        run: |s, p| {
            let old = post(s, p)?.name.clone();
            let new = check_name(params::req_str(p, "name")?)?;
            if new != old && s.library.post(&new).is_some() {
                return Err(EngineError::InvalidParams(format!("there is already a post processor `{new}`")));
            }
            if let Some(n) = s.library.posts.iter_mut().find(|n| n.name == old) {
                n.name = new.clone();
            }
            for m in s.library.machines.iter_mut().filter(|m| m.post_name == old) {
                m.post_name = new.clone();
            }
            if s.job.machine.post_name == old {
                s.job.machine.post_name = new.clone();
                s.job.touch();
            }
            s.library.touch();
            Ok(s.library.post(&new).map(post_json).unwrap_or_default())
        },
    },
    CommandSpec {
        id: "post.library.remove",
        label: "Delete post processor",
        tooltip: "Delete a post processor; machines using it change to the first in the library",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[NAME],
        returns: "{\"removed\": name}",
        enabled: |s| s.library.posts.len() > 1,
        run: |s, p| {
            let name = post(s, p)?.name.clone();
            if s.library.posts.len() < 2 {
                return Err(EngineError::InvalidParams("the library keeps at least one post processor".into()));
            }
            s.library.posts.retain(|n| n.name != name);
            let first = s.library.posts.first().cloned().ok_or_else(|| EngineError::Internal("no posts".into()))?;
            for m in s.library.machines.iter_mut().filter(|m| m.post_name == name) {
                m.post_name = first.name.clone();
                m.post = first.settings.clone();
            }
            if s.job.machine.post_name == name {
                s.job.machine.post_name = first.name.clone();
                s.job.machine.post = first.settings.clone();
                s.job.touch();
            }
            s.library.touch();
            Ok(json!({ "removed": name }))
        },
    },
];

/// The library machine `id` names.
fn machine<'a>(s: &'a Session, p: &Value) -> EngineResult<&'a Machine> {
    let id = params::req_str(p, "id")?;
    s.library
        .machine(id)
        .ok_or_else(|| EngineError::InvalidParams(format!("there is no machine `{id}` in the library")))
}

/// The library post `id` names.
fn post<'a>(s: &'a Session, p: &Value) -> EngineResult<&'a NamedPost> {
    let id = params::req_str(p, "id")?;
    s.library.post(id).ok_or_else(|| EngineError::InvalidParams(format!("there is no post processor `{id}`")))
}

fn unique(s: &Session, name: String) -> EngineResult<String> {
    if s.library.machine(&name).is_some() {
        return Err(EngineError::InvalidParams(format!("the library already has a machine `{name}`")));
    }
    Ok(name)
}

/// Makes `m` the job's machine and remakes the cuts for it.
fn set_machine(s: &mut Session, m: Machine) {
    s.job.machine = m;
    s.job.rebuild();
    s.job.touch();
    s.sim = None;
}

pub fn post_json(p: &NamedPost) -> Value {
    json!({ "name": p.name, "settings": p.settings.to_json() })
}

/// Changes post processor `name`'s settings (in the library, and on every machine using it).
pub fn set_post(s: &mut Session, name: &str, p: &Value) -> EngineResult<PostSettings> {
    let mut settings = s
        .library
        .post(name)
        .map(|n| n.settings.clone())
        .ok_or_else(|| EngineError::InvalidParams(format!("there is no post processor `{name}`")))?;
    settings.apply(p)?;
    if let Some(n) = s.library.posts.iter_mut().find(|n| n.name == name) {
        n.settings = settings.clone();
    }
    for m in s.library.machines.iter_mut().filter(|m| m.post_name == name) {
        m.post = settings.clone();
    }
    if s.job.machine.post_name == name {
        s.job.machine.post = settings.clone();
        s.job.touch();
    }
    s.library.touch();
    Ok(settings)
}
