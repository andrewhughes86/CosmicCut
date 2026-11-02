//! Simulation commands: play the program and watch the foam come away.

use serde_json::{Value, json};

use super::{CommandSpec, Param, ParamKind, Placement, RibbonTab, always};
use crate::error::{EngineError, EngineResult};
use crate::params;
use crate::session::Session;
use crate::sim::{SPEEDS, Sim};

/// The simulation, started if it is not running yet.
fn sim(s: &mut Session) -> EngineResult<&mut Sim> {
    if s.sim.is_none() {
        if s.job.cuts.iter().all(|c| c.error.is_some() || c.path.moves.is_empty()) {
            return Err(EngineError::InvalidParams("there are no cuts to simulate".into()));
        }
        s.sim = Some(Box::new(Sim::new(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops)));
    }
    s.sim.as_deref_mut().ok_or_else(|| EngineError::Internal("the simulation did not start".into()))
}

fn state(s: &mut Session) -> EngineResult<Value> {
    Ok(s.sim.as_deref().map_or(json!({ "running": false }), Sim::to_json))
}

fn running(s: &Session) -> bool {
    s.sim.is_some()
}

macro_rules! transport {
    ($id:literal, $label:literal, $tooltip:literal, $shortcut:expr, $panel:literal, $body:expr) => {
        CommandSpec {
            id: $id,
            label: $label,
            tooltip: $tooltip,
            // Every button on the Simulate tab is large: nothing else shares it.
            placement: Some(Placement::large(RibbonTab::Simulate, $panel)),
            icon: None,
            shortcut: $shortcut,
            params: &[],
            returns: "simulation state",
            enabled: super::has_cuts,
            run: |s, _| {
                let f: fn(&mut Sim) = $body;
                f(sim(s)?);
                state(s)
            },
        }
    };
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    transport!(
        "sim.play",
        "Simulation",
        "Play the program and watch the foam being cut",
        Some("Space"),
        "Playback",
        |sim| {
            if sim.time >= sim.duration() - 1.0e-9 {
                sim.seek(0.0);
            }
            sim.playing = true;
        }
    ),
    transport!("sim.pause", "Pause", "Pause the simulation", None, "Playback", |sim| sim.playing = false),
    transport!("sim.jump.start", "Start", "Back to the start of the program", Some("Home"), "Step", |sim| {
        sim.playing = false;
        sim.seek(0.0);
    }),
    transport!(
        "sim.jump.previous",
        "Previous cut",
        "Back to the start of this cut, or the one before",
        None,
        "Step",
        |sim| { sim.jump_cut(false) }
    ),
    transport!("sim.jump.next", "Next cut", "On to the start of the next cut", None, "Step", |sim| sim.jump_cut(true)),
    transport!("sim.jump.end", "End", "On to the end: the finished part", Some("End"), "Step", |sim| {
        sim.playing = false;
        let end = sim.duration();
        sim.seek(end);
    }),
    CommandSpec {
        id: "sim.speed",
        label: "Speed",
        tooltip: "Playback speed: program seconds per second (omit to step through the speeds)",
        placement: Some(Placement::large(RibbonTab::Simulate, "Playback")),
        icon: None,
        shortcut: None,
        params: &[Param::optional("speed", ParamKind::Number { min: 0.01, max: 100_000.0 }, "Speed factor")],
        returns: "simulation state",
        enabled: super::has_cuts,
        run: |s, p| {
            let want = params::opt_f64(p, "speed", 0.01, 100_000.0)?;
            let sim = sim(s)?;
            sim.speed = match want {
                Some(v) => v,
                None => SPEEDS.iter().copied().find(|v| *v > sim.speed + 1.0e-9).unwrap_or(1.0),
            };
            state(s)
        },
    },
    CommandSpec {
        id: "sim.seek",
        label: "Seek",
        tooltip: "Jump to a time in the program",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[Param::required("time", ParamKind::Number { min: 0.0, max: 1.0e9 }, "Seconds from the start")],
        returns: "simulation state",
        enabled: super::has_cuts,
        run: |s, p| {
            let t = params::opt_f64(p, "time", 0.0, 1.0e9)?.unwrap_or(0.0);
            sim(s)?.seek(t);
            state(s)
        },
    },
    CommandSpec {
        id: "sim.stop",
        label: "Stop",
        tooltip: "Close the simulation",
        placement: Some(Placement::large(RibbonTab::Simulate, "Playback")),
        icon: Some("sim_stop"),
        shortcut: Some("Escape"),
        params: &[],
        returns: "{\"running\": false}",
        enabled: running,
        run: |s, _| {
            s.sim = None;
            Ok(json!({ "running": false }))
        },
    },
    CommandSpec {
        id: "sim.get",
        label: "Simulation",
        tooltip: "Where the simulation is",
        placement: None,
        icon: None,
        shortcut: None,
        params: &[],
        returns: "simulation state",
        enabled: always,
        run: |s, _| state(s),
    },
];
