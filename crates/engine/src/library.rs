//! The libraries the operator keeps between jobs: their machines, and the post processors that
//! write programs for them. A job holds a copy of its machine (with its post's settings), so a
//! job file always posts the same way.

use serde_json::{Value, json};

use crate::error::{EngineError, EngineResult};
use crate::machine::Machine;
use crate::post::PostSettings;
use crate::units::DisplayUnits;

/// A post processor in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedPost {
    pub name: String,
    pub settings: PostSettings,
}

/// Most entries either list holds.
pub const MAX_ENTRIES: usize = 500;

/// The post processor every machine starts with.
pub const DEFAULT_POST: &str = "HotwireCNC";

#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub machines: Vec<Machine>,
    pub posts: Vec<NamedPost>,
    /// Bumped on every change, so the app knows to save it.
    pub revision: u64,
}

impl Default for Library {
    fn default() -> Self {
        Self { machines: vec![Machine::default()], posts: builtin_posts(), revision: 0 }
    }
}

/// The post processors CosmicCut comes with.
pub fn builtin_posts() -> Vec<NamedPost> {
    let hotwire = PostSettings::default();
    let linuxcnc = PostSettings {
        units: DisplayUnits::Millimetres,
        header: "G17 G90 G91.1 G40 G49 G94 G64 P0.05\n{units}".into(),
        ..PostSettings::default()
    };
    let mach = PostSettings {
        header: "G17 G90 G91.1 G40 G49 G94\n{units}".into(),
        footer: "M30".into(),
        ..PostSettings::default()
    };
    let generic = PostSettings {
        header: "G17 G90\n{units}".into(),
        motion_every_line: true,
        feed_every_line: true,
        arcs: false,
        line_numbers: true,
        footer: "M30".into(),
        extension: "nc".into(),
        ..PostSettings::default()
    };
    vec![
        NamedPost { name: DEFAULT_POST.into(), settings: hotwire },
        NamedPost { name: "LinuxCNC (mm)".into(), settings: linuxcnc },
        NamedPost { name: "Mach3 / Mach4".into(), settings: mach },
        NamedPost { name: "Generic (G word every line, no arcs)".into(), settings: generic },
    ]
}

impl Library {
    pub fn machine(&self, name: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.name == name)
    }

    pub fn post(&self, name: &str) -> Option<&NamedPost> {
        self.posts.iter().find(|p| p.name == name)
    }

    /// `base`, else `base 2`, `base 3`… not yet used by a machine (or a post, with `posts`).
    pub fn fresh_name(&self, base: &str, posts: bool) -> String {
        let taken = |n: &str| if posts { self.post(n).is_some() } else { self.machine(n).is_some() };
        if !taken(base) {
            return base.to_owned();
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_owned())
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn to_json(&self) -> Value {
        json!({
            "machines": self.machines.iter().map(Machine::to_json).collect::<Vec<_>>(),
            "posts": self.posts.iter().map(|p| json!({ "name": p.name, "settings": p.settings.to_json() })).collect::<Vec<_>>(),
        })
    }

    /// Reads a saved library. Entries that cannot be read are left out and named in the result;
    /// an empty list falls back to the defaults.
    pub fn from_json(v: &Value) -> EngineResult<(Self, Vec<String>)> {
        if !v.is_object() {
            return Err(EngineError::InvalidParams("the library is not a JSON object".into()));
        }
        let mut dropped = Vec::new();
        let mut lib = Self { machines: Vec::new(), posts: Vec::new(), revision: 0 };
        for m in v.get("machines").and_then(Value::as_array).into_iter().flatten().take(MAX_ENTRIES) {
            match Machine::from_json(m) {
                Ok(m) if lib.machine(&m.name).is_none() => lib.machines.push(m),
                Ok(m) => dropped.push(format!("a second machine called {}", m.name)),
                Err(e) => dropped.push(format!("a machine ({e})")),
            }
        }
        for p in v.get("posts").and_then(Value::as_array).into_iter().flatten().take(MAX_ENTRIES) {
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
            let mut settings = PostSettings::default();
            match p.get("settings").map(|s| settings.apply(s)) {
                Some(Ok(())) if !name.is_empty() && lib.post(&name).is_none() => {
                    lib.posts.push(NamedPost { name, settings })
                }
                Some(Err(e)) => dropped.push(format!("the post {name} ({e})")),
                _ => dropped.push(format!("a post called {name:?}")),
            }
        }
        if lib.machines.is_empty() {
            lib.machines.push(Machine::default());
        }
        if lib.posts.is_empty() {
            lib.posts = builtin_posts();
        }
        Ok((lib, dropped))
    }
}

/// Checks a library entry's name: not blank, not too long.
pub fn check_name(name: &str) -> EngineResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(EngineError::InvalidParams("the name must not be empty".into()));
    }
    Ok(name.chars().take(120).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraries_round_trip_and_bad_entries_are_dropped() -> EngineResult<()> {
        let mut lib = Library::default();
        lib.machines.push(Machine { name: "Big table".into(), ..Machine::default() });
        let mut v = lib.to_json();
        if let Some(a) = v["posts"].as_array_mut() {
            a.push(json!({ "name": "Broken", "settings": { "units": "furlongs" } }));
        }
        let (back, dropped) = Library::from_json(&v)?;
        assert_eq!(back.machines, lib.machines);
        assert_eq!(back.posts, lib.posts);
        assert_eq!(dropped.len(), 1);
        assert_eq!(lib.fresh_name("Big table", false), "Big table 2");
        Ok(())
    }
}
