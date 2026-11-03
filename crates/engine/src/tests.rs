//! Engine tests across commands: whole jobs from the console's point of view.
#![allow(clippy::panic)]

use serde_json::{Value, json};

use crate::Session;
use crate::cmd;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn a_profile_job_posts_and_simulates() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 200.0 }));
    let shape = run(&mut s, "shape.star", json!({ "points": 5, "outer": 180.0, "inner": 80.0 }));
    let id = shape["id"].as_str().unwrap_or_default().to_owned();
    let cut = run(&mut s, "cut.profile", json!({ "shape": id, "copies": 2, "step": 90.0 }));
    assert!(cut["moves"].as_u64().unwrap_or(0) > 20, "{cut}");
    let g = run(&mut s, "post.gcode.preview", json!({}));
    let text = g["text"].as_str().unwrap_or_default();
    assert!(text.contains("G20") && text.contains("M3") && text.contains("M5") && text.contains("B90"), "{text}");
    assert!(text.is_ascii(), "G-code must be plain ASCII");
    assert!(g["warnings"].as_array().is_some_and(Vec::is_empty), "{g}");
    let start = s.sim.as_ref().map(|_| 0.0);
    assert!(start.is_none());
    run(&mut s, "sim.jump.end", json!({}));
    let state = run(&mut s, "sim.get", json!({}));
    let left = state["foam_volume"].as_f64().unwrap_or(0.0);
    let block = 300.0 * 300.0 * 200.0;
    // Two star prisms at right angles keep much less than the block.
    assert!(left > 0.05 * block && left < 0.6 * block, "{left}");
}

#[test]
fn the_sample_job_cuts_a_penguin() {
    let mut s = Session::with_sample();
    assert_eq!(s.job.models.len(), 1);
    assert_eq!(s.job.cuts.len(), 1);
    assert!(!s.job.is_modified());
    let cut = &s.job.cuts[0];
    assert!(cut.error.is_none(), "{:?}", cut.error);
    assert!(cut.warnings.is_empty(), "{:?}", cut.warnings);
    assert!(cut.release.is_some());
    run(&mut s, "sim.jump.end", json!({}));
    let state = run(&mut s, "sim.get", json!({}));
    let left = state["foam_volume"].as_f64().unwrap_or(0.0);
    // The penguin and the inch of foam under it, much less than the block.
    assert!(left > 0.15 * s.job.stock.volume() && left < 0.6 * s.job.stock.volume(), "{left}");
    // Back to the start puts the foam back.
    run(&mut s, "sim.jump.start", json!({}));
    let state = run(&mut s, "sim.get", json!({}));
    assert!(state["foam_volume"].as_f64().unwrap_or(0.0) > 0.95 * s.job.stock.volume());
}

#[test]
fn edits_rebuild_cuts_and_bad_ones_change_nothing() {
    let mut s = Session::new();
    let shape = run(&mut s, "shape.rectangle", json!({ "width": 100.0, "height": 100.0 }));
    let sid = shape["id"].as_str().unwrap_or_default().to_owned();
    let cut = run(&mut s, "cut.profile", json!({ "shape": sid }));
    let cid = cut["id"].as_str().unwrap_or_default().to_owned();
    // Edit in place.
    run(&mut s, "cut.profile", json!({ "id": cid, "shape": sid, "angle": 45.0 }));
    assert_eq!(s.job.cuts.len(), 1);
    // Removing the shape leaves the cut with an error, not a crash.
    run(&mut s, "shape.remove", json!({ "id": sid }));
    assert!(s.job.cuts[0].error.is_some());
    let before = s.job.edits;
    assert!(s.execute("stock.block", &json!({ "width": -1.0, "depth": 1.0, "height": 1.0 })).is_err());
    assert!(s.execute("test.panic", &json!({})).is_err());
    assert_eq!(s.job.edits, before);
    assert_ne!(s.job.name, "half-edited");
}

#[test]
fn jobs_save_and_open() {
    let mut s = Session::with_sample();
    let dir = std::env::temp_dir().join(format!("cosmiccut-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("vase.ccut").to_string_lossy().into_owned();
    run(&mut s, "job.save.as", json!({ "path": path }));
    let moves = s.job.cuts[0].path.moves.len();
    let mut t = Session::new();
    run(&mut t, "job.open", json!({ "path": path }));
    assert_eq!(t.job.shapes.len(), s.job.shapes.len());
    assert_eq!(t.job.cuts[0].path.moves.len(), moves);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_params_never_panic() {
    let junk = [
        json!(null),
        json!({}),
        json!([1, 2]),
        json!({ "id": "nope" }),
        json!({ "shape": "nope", "thickness": 1e9 }),
        json!({ "width": 1e300, "depth": -0.0, "height": f64::MAX }),
        json!({ "path": "/nonexistent/\u{0}/x.dxf" }),
        json!({ "code": "9999", "chord": 0.01 }),
        json!({ "points": [[1e9, 1e9, 0], [0, 0, 0]] }),
    ];
    for spec in cmd::all() {
        if spec.id == "test.panic" {
            continue;
        }
        for p in &junk {
            let mut s = Session::with_sample();
            let _ = s.execute(spec.id, p);
        }
    }
}

#[test]
fn every_ribbon_command_is_reachable_and_unique() {
    let mut ids: Vec<&str> = cmd::all().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    for tab in cmd::RibbonTab::ALL {
        assert!(!cmd::ribbon(tab).is_empty(), "{tab:?} is empty");
    }
}

/// An ASCII STL of a box from the origin to `size`.
fn box_stl(size: [f64; 3]) -> String {
    let c = |i: usize| {
        [
            if i & 1 == 0 { 0.0 } else { size[0] },
            if i & 2 == 0 { 0.0 } else { size[1] },
            if i & 4 == 0 { 0.0 } else { size[2] },
        ]
    };
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [1, 3, 7, 5], [0, 4, 6, 2]];
    let mut out = String::from("solid box\n");
    for q in quads {
        for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
            out.push_str("facet normal 0 0 0\nouter loop\n");
            for v in t {
                let p = c(v);
                out.push_str(&format!("vertex {} {} {}\n", p[0], p[1], p[2]));
            }
            out.push_str("endloop\nendfacet\n");
        }
    }
    out.push_str("endsolid box\n");
    out
}

#[test]
fn models_lie_flat_centred_and_make_cuts() {
    let dir = std::env::temp_dir().join(format!("cosmiccut-model-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("brick.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([200.0, 100.0, 50.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 400.0, "depth": 400.0, "height": 400.0, "base": 25.4 }));
    let m = run(&mut s, "model.import", json!({ "path": path, "units": "mm" }));
    let id = m["id"].as_str().unwrap_or_default().to_owned();
    // Centred on the axis, on the stock's bottom.
    assert!((m["min"][0].as_f64().unwrap_or(0.0) + 100.0).abs() < 1e-3, "{m}");
    assert!((m["min"][2].as_f64().unwrap_or(0.0) - 25.4).abs() < 1e-3, "{m}");
    // Stand it on its 100 × 50 end, then turn it a quarter: still centred.
    let m = run(&mut s, "model.lay_flat", json!({ "id": id, "down": [1.0, 0.0, 0.0] }));
    assert!((m["max"][2].as_f64().unwrap_or(0.0) - 225.4).abs() < 1e-3, "{m}");
    let m = run(&mut s, "model.place", json!({ "id": id, "rotate": 90.0 }));
    assert!((m["min"][0].as_f64().unwrap_or(0.0) + m["max"][0].as_f64().unwrap_or(0.0)).abs() < 1e-3);
    // One cut: a profile every 45° over half a turn (4 rotations).
    let c = run(&mut s, "cut.model", json!({ "spacing": "degrees", "degrees": 45.0 }));
    assert_eq!(s.job.cuts.len(), 1, "{c}");
    assert_eq!(s.job.cuts[0].keeps.len(), 4);
    let angles: Vec<f64> = s.job.cuts[0].keeps.iter().map(|k| k.b).collect();
    assert_eq!(angles, vec![0.0, 45.0, 90.0, 135.0]);
    // Dividing half a turn by 4 is the same.
    let p = json!({ "spacing": "divide", "divisions": 4 });
    assert_eq!(crate::cut::model_angles(&p).unwrap_or_default(), angles);
    // Turning the model rebuilds its cut from the new outlines.
    let before = s.job.cuts[0].path.clone();
    run(&mut s, "model.place", json!({ "id": id, "rotate": 30.0 }));
    assert!(s.job.cuts[0].error.is_none());
    assert_ne!(s.job.cuts[0].path, before);
    assert!(
        s.job.cuts.iter().all(|c| c.error.is_none()),
        "{:?}",
        s.job.cuts.iter().map(|c| &c.error).collect::<Vec<_>>()
    );
    run(&mut s, "sim.jump.end", json!({}));
    let left = run(&mut s, "sim.get", json!({}))["foam_volume"].as_f64().unwrap_or(0.0);
    // About the brick: 200 × 100 × 50 mm.
    // About the brick (200 × 100 × 50 mm, stood on end), cut from four sides.
    assert!(left > 0.6e6 && left < 2.0e6, "{left}");
    // Saved and opened again, the model is read back from its file.
    let job = dir.join("brick.ccut").to_string_lossy().into_owned();
    run(&mut s, "job.save.as", json!({ "path": job }));
    let mut t = Session::new();
    run(&mut t, "job.open", json!({ "path": job }));
    assert_eq!(t.job.models.len(), 1);
    assert!((t.job.models[0].bounds().max.z - 225.4).abs() < 1e-3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn post_settings_shape_the_program() {
    let mut s = Session::with_sample();
    run(
        &mut s,
        "post.settings",
        json!({
            "wire_on": "M64 P0\nM3 S55",
            "wire_off": "M65 P0",
            "header": "(SAFETY)\nG17 G90 G40\n{units}",
            "footer": "G0 X0 Y0\nM30",
            "motion_every_line": true,
            "line_numbers": true,
            "percent": false,
            "units": "mm",
        }),
    );
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    assert!(text.contains("N10 G17 G90 G40") && text.contains("G21"), "{text}");
    assert!(text.contains("M64 P0") && text.contains("M3 S55") && text.contains("M65 P0"));
    assert!(text.trim_end().ends_with("M30"), "{text}");
    assert!(!text.contains('%'));
    // Every move line starts (after its number) with its G word.
    for line in text.lines() {
        let code = if line.starts_with('N') { line.split_once(' ').map_or(line, |(_, rest)| rest) } else { line };
        if !code.starts_with('(') && (code.contains('X') || code.contains('Y') || code.contains('B')) {
            assert!(code.starts_with('G'), "move line without a G word: {line}");
        }
    }
}

#[test]
fn models_scale_to_fit_to_height_and_by_factor() {
    let dir = std::env::temp_dir().join(format!("cosmiccut-scale-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("brick.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([200.0, 100.0, 50.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 400.0, "depth": 300.0, "height": 500.0 }));
    run(&mut s, "model.import", json!({ "path": path, "units": "mm" }));
    // Fit: the 200 mm width meets the 400 mm block less 10 mm a side.
    let m = run(&mut s, "model.scale", json!({ "mode": "fit", "margin": 10.0 }));
    let width = m["max"][0].as_f64().unwrap_or(0.0) - m["min"][0].as_f64().unwrap_or(0.0);
    assert!((width - 380.0).abs() < 0.01, "{m}");
    let m = run(&mut s, "model.scale", json!({ "mode": "height", "height": 25.0 }));
    assert!((m["max"][2].as_f64().unwrap_or(0.0) - 25.0).abs() < 1e-3, "{m}");
    let m = run(&mut s, "model.scale", json!({ "mode": "factor", "factor": 0.5 }));
    assert!((m["max"][2].as_f64().unwrap_or(0.0) - 25.0).abs() < 1e-3 && m["scale"] == json!(0.5), "{m}");
    assert!(s.execute("model.scale", &json!({ "mode": "height" })).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parts_stay_attached_until_the_release_at_the_end() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 300.0, "base": 25.4 }));
    let sq = run(&mut s, "shape.rectangle", json!({ "width": 100.0, "height": 150.0 }));
    let id = sq["id"].as_str().unwrap_or_default().to_owned();
    run(&mut s, "cut.twoview", json!({ "front": id, "side": id, "anchor": "bottom_centre", "y": 76.2, "kerf": 1.0 }));
    let c = &s.job.cuts[0];
    // No cut move runs along the bottom (h = 25.4 less half a kerf) under the part.
    let bottom = 76.2 - 0.5;
    let mut at = c.path.start;
    for m in &c.path.moves {
        if m.kind == crate::wire::MoveKind::Cut {
            let under = (at.h - bottom).abs() < 1e-3
                && (m.to.h - bottom).abs() < 1e-3
                && at.u.abs() < 50.0
                && m.to.u.abs() < 50.0;
            assert!(!under, "cuts under the part: {at:?} -> {:?}", m.to);
        }
        at = m.to;
    }
    // The program ends with the release along the bottom.
    let prog = crate::program::Program::build(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops);
    let start = prog.release_at.unwrap_or(usize::MAX);
    assert!(start < prog.moves.len());
    let releases_bottom =
        prog.moves[start..].iter().any(|(_, m)| (m.to.h - bottom).abs() < 1e-3 && m.to.u.abs() < 50.5);
    assert!(releases_bottom);
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    assert!(text.contains("(Release - cut the part free"), "{text}");
    // Simulated, the block under the part is still there, joined to it.
    run(&mut s, "sim.jump.end", json!({}));
    let left = run(&mut s, "sim.get", json!({}))["foam_volume"].as_f64().unwrap_or(0.0);
    // The part and the 2 in of foam under it, which the wire never crossed.
    let part = 100.0 * 100.0 * 150.0;
    let base = 300.0 * 300.0 * 50.8;
    assert!(left > part + 0.9 * base && left < part + 1.1 * base + 0.2 * part, "{left}");
}

#[test]
fn every_new_or_opened_job_is_drawn_afresh() {
    let mut s = Session::with_sample();
    let mut seen = vec![s.job.revision];
    for (id, p) in [("job.new", json!({})), ("job.sample", json!({})), ("job.new", json!({})), ("job.new", json!({}))] {
        run(&mut s, id, p);
        assert!(!seen.contains(&s.job.revision), "{id} reused revision {}", s.job.revision);
        seen.push(s.job.revision);
    }
    assert!(s.job.cuts.is_empty() && s.job.models.is_empty() && s.job.shapes.is_empty());
}

#[test]
fn jobs_open_in_tabs_each_with_its_own_history() {
    let mut s = Session::with_sample();
    assert_eq!(s.tabs().len(), 1);
    // New opens a second tab; an edit there leaves the sample alone.
    run(&mut s, "job.new", json!({}));
    assert_eq!((s.tabs().len(), s.active_tab()), (2, 1));
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 300.0 }));
    let undos = s.undo.len();
    assert_eq!(undos, 1, "opening a tab is not an edit");
    // Back to the sample: its cut, its (empty) history; then forward again to the edit.
    run(&mut s, "job.tab.set", json!({ "index": 0 }));
    assert_eq!(s.job.name, "Sample penguin");
    assert_eq!(s.job.cuts.len(), 1);
    assert!(s.undo.is_empty());
    run(&mut s, "job.tab.next", json!({}));
    assert_eq!(s.undo.len(), undos);
    assert!(s.job.cuts.is_empty());
    // A file already open comes to the front instead of opening twice.
    let path = std::env::temp_dir().join(format!("cosmiccut-tabs-{}.ccut", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    run(&mut s, "job.save", json!({ "path": path }));
    run(&mut s, "job.tab.set", json!({ "index": 0 }));
    run(&mut s, "job.open", json!({ "path": path }));
    assert_eq!((s.tabs().len(), s.active_tab()), (2, 1));
    let _ = std::fs::remove_file(&path);
    // Closing a tab brings in its neighbour; closing the last leaves one empty job.
    run(&mut s, "job.close", json!({}));
    assert_eq!((s.tabs().len(), s.job.name.as_str()), (1, "Sample penguin"));
    run(&mut s, "job.close", json!({}));
    assert_eq!(s.tabs().len(), 1);
    assert!(s.job.cuts.is_empty());
    assert!(s.execute("job.tab.set", &json!({ "index": 3 })).is_err());
}

#[test]
fn operations_stop_the_machine_for_the_operator() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 300.0, "base": 25.4 }));
    let sq = run(&mut s, "shape.rectangle", json!({ "width": 100.0, "height": 100.0 }));
    let id = sq["id"].as_str().unwrap_or_default().to_owned();
    run(&mut s, "cut.profile", json!({ "shape": id, "name": "First" }));
    // Op 2: the operator turns the stock; its cut comes after the stop.
    run(&mut s, "op.add", json!({ "name": "Turn the stock", "stop": "m0", "message": "Turn the stock a quarter" }));
    run(&mut s, "cut.profile", json!({ "shape": id, "name": "Second" }));
    let ops: Vec<String> = s.job.cuts.iter().map(|c| c.op.clone()).collect();
    assert_ne!(ops[0], ops[1]);
    // Listing the second cut first does not change the program's order.
    let second = s.job.cuts[1].id.clone();
    run(&mut s, "cut.move", json!({ "id": second, "to": 0 }));
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    let pos = |needle: &str| text.find(needle).unwrap_or(usize::MAX);
    assert!(pos("(First") < pos("(MSG, Turn the stock a quarter)"), "{text}");
    assert!(pos("(MSG, Turn the stock a quarter)") < pos("\nM0\n") && pos("\nM0\n") < pos("(Second"), "{text}");
    // The wire is off for the stop and heats again after.
    let stop = pos("\nM0\n");
    assert!(text[..stop].rfind("M5").unwrap_or(0) > pos("(First"));
    assert!(text[stop..].contains("M3"));
    // The simulation waits at the stop.
    run(&mut s, "sim.play", json!({}));
    if let Some(sim) = s.sim.as_deref_mut() {
        for _ in 0..10_000 {
            sim.tick(0.25);
            if !sim.playing {
                break;
            }
        }
        assert!(sim.notice.as_deref().is_some_and(|n| n.contains("Turn the stock a quarter")), "{:?}", sim.notice);
        assert!(sim.time < sim.duration());
    }
    // Saved and opened, the operations stay.
    let dir = std::env::temp_dir().join(format!("cosmiccut-ops-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("ops.ccut").to_string_lossy().into_owned();
    run(&mut s, "job.save.as", json!({ "path": path }));
    let mut t = Session::new();
    run(&mut t, "job.open", json!({ "path": path }));
    assert_eq!(t.job.ops.len(), 2);
    assert_eq!(t.job.ops[1].message, "Turn the stock a quarter");
    assert_eq!(t.job.cuts.iter().filter(|c| c.op == t.job.ops[1].id).count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_model_can_be_freed_from_the_table_centre() {
    let mut s = Session::with_sample();
    let id = s.job.models[0].id.clone();
    let m = run(&mut s, "model.place", json!({ "id": id, "x": 100.0 }));
    assert!((m["min"][0].as_f64().unwrap_or(0.0) + m["max"][0].as_f64().unwrap_or(0.0) - 200.0).abs() < 1e-3, "{m}");
    assert_eq!(m["centred"], json!(false));
    let m = run(&mut s, "model.centre", json!({ "id": id, "on": true }));
    assert!((m["min"][0].as_f64().unwrap_or(0.0) + m["max"][0].as_f64().unwrap_or(0.0)).abs() < 1e-3, "{m}");
    // Freeing it keeps it where it stands.
    let before = s.job.models[0].bounds();
    run(&mut s, "model.centre", json!({ "id": id, "on": false }));
    assert_eq!(s.job.models[0].bounds(), before);
    // Choosing its bottom face centres it again.
    run(&mut s, "model.place", json!({ "id": id, "x": 50.0 }));
    let m = run(&mut s, "model.lay_flat", json!({ "id": id, "down": [0.0, 0.0, -1.0] }));
    assert_eq!(m["centred"], json!(true));
}

#[test]
fn machines_and_post_processors_come_from_the_libraries() {
    let mut s = Session::with_sample();
    assert_eq!(s.job.machine.post_name, "HotwireCNC");
    // A second machine that writes millimetres with the LinuxCNC post.
    run(&mut s, "machine.library.new", json!({ "name": "Big table" }));
    run(&mut s, "machine.library.use", json!({ "id": "Big table" }));
    run(&mut s, "machine.edit", json!({ "post": "LinuxCNC (mm)", "travel_x": 3000.0 }));
    assert_eq!(s.job.machine.post.units, crate::units::DisplayUnits::Millimetres);
    // The library kept the edit.
    assert_eq!(s.library.machine("Big table").map(|m| m.travel_x), Some(3000.0));
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    assert!(text.contains("G21") && text.contains("G64 P0.05"), "{text}");
    // Editing the post changes it for the machines that use it.
    run(&mut s, "post.settings", json!({ "wire_on": "M3 S70" }));
    assert_eq!(s.library.post("LinuxCNC (mm)").map(|p| p.settings.wire_on.clone()), Some("M3 S70".into()));
    assert_eq!(s.job.machine.post.wire_on, "M3 S70");
    // Renaming it renames it on the machines too; a copy is its own post.
    run(&mut s, "post.library.rename", json!({ "id": "LinuxCNC (mm)", "name": "Shop mm" }));
    assert_eq!(s.job.machine.post_name, "Shop mm");
    assert_eq!(s.library.machine("Big table").map(|m| m.post_name.clone()), Some("Shop mm".into()));
    run(&mut s, "post.library.new", json!({ "name": "Shop copy", "from": "Shop mm" }));
    run(&mut s, "post.library.remove", json!({ "id": "Shop mm" }));
    assert_eq!(s.job.machine.post_name, s.library.posts[0].name);
    // Back to the first machine.
    run(&mut s, "machine.library.use", json!({ "id": "Hot-wire XYB" }));
    assert_eq!(s.job.machine.travel_x, 2540.0);
    // The libraries survive a save and load.
    let (back, dropped) = crate::library::Library::from_json(&s.library.to_json()).unwrap_or_default();
    assert!(dropped.is_empty());
    assert_eq!(back.machines.len(), 2);
}

#[test]
fn the_program_ends_clear_of_the_part_and_goes_home() {
    let mut s = Session::with_sample();
    let prog = crate::program::Program::build(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops);
    // After the part-off, every move across the stock is above it.
    let start = prog.release_at.unwrap_or(usize::MAX);
    let release_end = prog.moves.iter().skip(start).position(|(c, _)| c.is_none()).map_or(usize::MAX, |k| start + k);
    let top = s.job.stock.top();
    let (u0, u1) = s.job.stock.u_range(prog.moves[release_end].1.to.b);
    for k in release_end..prog.moves.len() {
        let (a, b) = (prog.from(k), prog.moves[k].1.to);
        let crosses = (a.u - b.u).abs() > 1e-6 && a.u.max(b.u) > u0 && a.u.min(b.u) < u1;
        assert!(!crosses || (a.h > top && b.h > top), "move {k} crosses the part low: {a:?} -> {b:?}");
    }
    // Between the profiles and before the release the wire stays beside the foam: it never
    // climbs over the stock, and the table turns with the wire out past the stock's reach.
    let reach = s.job.stock.reach();
    for k in 0..start.min(prog.moves.len()) {
        let (a, m) = (prog.from(k), prog.moves[k].1);
        assert!(m.to.h <= top, "move {k} climbs over the stock: {a:?} -> {:?}", m.to);
        if (m.to.b - a.b).abs() > 1e-9 {
            assert!(a.u.abs() >= reach, "the table turns at {a:?}");
        }
    }
    // Then nothing but the trip home: no backing up after the release.
    assert_eq!(prog.home_at, Some(release_end), "moves between the release and home");
    // It ends at machine X0 Y0.
    let end = prog.moves.last().map(|(_, m)| m.to).unwrap_or_default();
    assert_eq!(s.job.machine.machine_xy(end.u, end.h), (0.0, 0.0));
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    let home = text.find("and return home)").unwrap_or(0);
    assert!(home > 0 && text[home..].contains("X0") && text[home..].contains("Y0"), "{text}");
    // After the last cut the table turns to B0 or B180, whichever is nearer, clear of the foam,
    // before going home.
    let from = prog.from(prog.home_at.unwrap_or(0)).b;
    let nearer = if from.abs() <= (from - 180.0).abs() { 0.0 } else { 180.0 };
    let turned = prog.moves[prog.home_at.unwrap_or(0)..].iter().find(|(_, m)| m.kind == crate::wire::MoveKind::Rotate);
    assert!(turned.is_some_and(|(_, m)| m.to.b == nearer), "from B{from}: {turned:?}");
    assert!(turned.is_some_and(|(_, m)| m.to.u.abs() >= s.job.stock.reach() || m.to.h > s.job.stock.top()));
    let turn = text[home..].find(&format!("B{nearer}\n")).map(|k| k + home).unwrap_or(0);
    assert!(turn > home && text[turn..].contains("X0"), "{text}");
    // A table left nearer B180 turns to B180 instead.
    for (last, want) in [(170.0, 180.0), (80.0, 0.0), (300.0, 180.0)] {
        let mut cuts = s.job.cuts.clone();
        for c in &mut cuts {
            c.release = None;
        }
        if let Some(c) = cuts.last_mut() {
            let at = c.path.end();
            c.path.moves.push(crate::wire::Move {
                kind: crate::wire::MoveKind::Rotate,
                to: crate::wire::WirePos::new(at.u, at.h, last),
                feed: 0.0,
            });
        }
        let prog = crate::program::Program::build(&s.job.machine, &s.job.stock, &cuts, &s.job.ops);
        assert_eq!(prog.end_turn, Some(want), "from B{last}");
    }
    // The wire is off before it goes home.
    assert!(text[..home].rfind("M5").is_some_and(|m| m > text.find("(Release").unwrap_or(0)));
    // From the right of the stock, home goes over the top (no end turn, to look at just that).
    s.job.machine.post.square_b = false;
    let at = crate::wire::WirePos::new(400.0, 50.0, 0.0);
    let mut cuts = s.job.cuts.clone();
    if let Some(c) = cuts.first_mut() {
        c.release = None;
        c.path.moves.push(crate::wire::Move { kind: crate::wire::MoveKind::Rapid, to: at, feed: 0.0 });
    }
    let prog = crate::program::Program::build(&s.job.machine, &s.job.stock, &cuts, &s.job.ops);
    let home: Vec<_> = prog.moves[prog.home_at.unwrap_or(0)..].iter().map(|(_, m)| m.to).collect();
    let ymax = s.job.machine.travel_y + s.job.machine.wire_y0;
    assert_eq!(home.len(), 3, "{home:?}");
    assert!((home[0].h - ymax).abs() < 1e-9 && (home[0].u - 400.0).abs() < 1e-9);
    assert!((home[1].u + s.job.machine.table_x).abs() < 1e-9);
}

#[test]
fn decimals_the_user_sets_keep_their_zeros() {
    let mut s = Session::with_sample();
    run(&mut s, "post.settings", json!({ "decimals": 4 }));
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    let home = text.find("(Return home)").unwrap_or(0);
    assert!(text[home..].contains("X0.0000") && text[home..].contains("Y0.0000"), "{text}");
    let coordinate = |w: &&str| {
        (w.starts_with('X') || w.starts_with('Y'))
            && w.chars().nth(1).is_some_and(|c| c.is_ascii_digit() || c == '-')
            && !w.contains(')')
    };
    for word in text.split_whitespace().filter(coordinate) {
        let digits = word.split('.').nth(1).map_or(0, str::len);
        assert_eq!(digits, 4, "{word}");
    }
}

#[test]
fn changes_undo_and_redo_and_cuts_copy_and_paste() {
    let mut s = Session::with_sample();
    let first = s.job.cuts[0].clone();
    // Copy and paste the cut: a second one, then undo takes it back and redo brings it again.
    run(&mut s, "selection.set", json!({ "id": first.id }));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.job.cuts.len(), 2);
    assert_ne!(s.job.cuts[1].id, first.id);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.job.cuts.len(), 1);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.job.cuts.len(), 2);
    // Cut removes it; paste puts it back.
    let pasted = s.job.cuts[1].id.clone();
    run(&mut s, "selection.set", json!({ "id": pasted }));
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.job.cuts.len(), 1);
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.job.cuts.len(), 2);
    // An edit can be undone; looking around (selection, view) is not an edit.
    let before = s.job.stock;
    run(&mut s, "stock.block", json!({ "width": 400.0, "depth": 400.0, "height": 400.0 }));
    let undos = s.undo.len();
    run(&mut s, "selection.set", json!({ "id": first.id }));
    run(&mut s, "view.grid.toggle", json!({}));
    assert_eq!(s.undo.len(), undos);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.job.stock, before);
    // Closing the job can be undone too.
    run(&mut s, "job.close", json!({}));
    assert!(s.job.cuts.is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.job.cuts.len(), 2);
    // A new change clears what was undone.
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 300.0 }));
    assert!(s.execute("edit.redo", &json!({})).is_err());
    // A model copies and pastes too; with nothing selected, Copy says what to do.
    let model = s.job.models[0].id.clone();
    run(&mut s, "selection.set", json!({ "id": model }));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.job.models.len(), 2);
    run(&mut s, "selection.set", json!({}));
    let Err(e) = s.execute("edit.copy", &json!({})) else { panic!("copy with nothing selected") };
    assert!(e.to_string().contains("select a cut, shape or model"), "{e}");
}

#[test]
fn a_sketch_is_a_shape_the_cuts_follow() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 400.0, "depth": 300.0, "height": 400.0 }));
    let id = run(&mut s, "sketch.new", json!({ "plane": "front" }))["id"].as_str().unwrap_or_default().to_owned();
    assert_eq!(s.view.sketch.as_deref(), Some(id.as_str()));
    // Drawing tools with no points pick the tool for the mouse.
    assert_eq!(run(&mut s, "sketch.circle", json!({}))["tool"], "circle");
    let lines = run(&mut s, "sketch.rectangle", json!({ "from": [-100.0, 0.0], "to": [100.0, 150.0] }))["ids"].clone();
    let bottom = lines[0].as_str().unwrap_or_default().to_owned();
    let dim =
        run(&mut s, "sketch.dimension", json!({ "items": [bottom] }))["id"].as_str().unwrap_or_default().to_owned();
    // A profile cut from the sketch's shape.
    run(&mut s, "cut.profile", json!({ "shape": id }));
    let width = |s: &Session| {
        let c = &s.job.cuts[0];
        let us: Vec<f64> =
            c.path.moves.iter().filter(|m| m.kind == crate::wire::MoveKind::Cut).map(|m| m.to.u).collect();
        us.iter().copied().fold(f64::NEG_INFINITY, f64::max) - us.iter().copied().fold(f64::INFINITY, f64::min)
    };
    let before = width(&s);
    assert!((before - 200.0).abs() < 2.0, "{before}");
    // Changing the dimension changes the cut; undo puts it back.
    s.view.sketch = Some(id.clone());
    run(&mut s, "sketch.dimension.set", json!({ "id": dim, "value": 300.0 }));
    assert!((width(&s) - 300.0).abs() < 2.0, "{}", width(&s));
    run(&mut s, "edit.undo", json!({}));
    assert!((width(&s) - before).abs() < 1e-6);
    // Constraints by selection; a conflicting one is refused and changes nothing.
    run(&mut s, "sketch.select", json!({ "items": [bottom] }));
    let saved = s.job.shapes[0].sketch.clone();
    assert!(s.execute("sketch.vertical", &json!({})).is_err());
    assert_eq!(s.job.shapes[0].sketch, saved);
    // Saved and opened again, the sketch is still a sketch.
    let path = std::env::temp_dir().join(format!("cosmiccut-sketch-{}.ccut", std::process::id()));
    let path = path.to_string_lossy().into_owned();
    run(&mut s, "sketch.finish", json!({}));
    assert!(s.view.sketch.is_none());
    run(&mut s, "job.save", json!({ "path": path }));
    let mut t = Session::new();
    run(&mut t, "job.open", json!({ "path": path }));
    let _ = std::fs::remove_file(&path);
    assert_eq!(t.job.shapes[0].sketch, s.job.shapes[0].sketch);
    run(&mut t, "sketch.edit", json!({ "id": id }));
    assert!(t.execute("sketch.line", &json!({ "from": [0.0, 200.0], "to": [50.0, 260.0] })).is_ok());
}

#[test]
fn the_dimension_key_picks_the_tool_even_with_a_selection() {
    let mut s = Session::new();
    run(&mut s, "sketch.new", json!({ "plane": "front" }));
    let line = run(&mut s, "sketch.line", json!({ "from": [10.0, 10.0], "to": [100.0, 40.0] }))["id"].clone();
    run(&mut s, "sketch.select", json!({ "items": [line] }));
    assert_eq!(run(&mut s, "sketch.dimension", Value::Null)["tool"], "dimension");
    assert_eq!(run(&mut s, "sketch.move", Value::Null)["tool"], "move");
    run(&mut s, "sketch.move", json!({ "by": [5.0, 0.0] }));
}

#[test]
fn constraints_that_do_not_suit_the_selection_are_not_offered() {
    let mut s = Session::new();
    run(&mut s, "sketch.new", json!({ "plane": "front" }));
    let a = run(&mut s, "sketch.line", json!({ "from": [10.0, 10.0], "to": [100.0, 40.0] }))["id"].clone();
    let b = run(&mut s, "sketch.line", json!({ "from": [10.0, 80.0], "to": [90.0, 140.0] }))["id"].clone();
    let fits = |s: &Session, id: &str| cmd::sketch::fits_selection(s, id);
    // Nothing selected: no constraint.
    assert!(!fits(&s, "sketch.parallel") && !fits(&s, "sketch.coincident"));
    // Two lines (Ctrl-click): parallel, perpendicular, equal; not coincident or tangent.
    run(&mut s, "sketch.select", json!({ "items": [a, b] }));
    assert!(fits(&s, "sketch.parallel") && fits(&s, "sketch.perpendicular") && fits(&s, "sketch.equal"));
    assert!(!fits(&s, "sketch.coincident") && !fits(&s, "sketch.tangent") && !fits(&s, "sketch.midpoint"));
    // The origin and a line: midpoint and coincident (point on it), not parallel.
    run(&mut s, "sketch.select", json!({ "items": ["p1", a] }));
    assert!(fits(&s, "sketch.midpoint") && fits(&s, "sketch.coincident") && !fits(&s, "sketch.parallel"));
    // Other commands are not affected.
    assert!(fits(&s, "sketch.line"));
}

#[test]
fn imported_and_made_shapes_are_sketches_that_take_dimensions() {
    let mut s = Session::new();
    let id = run(&mut s, "shape.airfoil", json!({ "code": "0012", "chord": 300.0 }))["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(s.job.shapes[0].sketch.is_some(), "an airfoil is drawn as a sketch");
    run(&mut s, "sketch.edit", json!({ "id": id }));
    assert_eq!(s.ui.ribbon_tab, crate::cmd::RibbonTab::Sketch);
    let line = s.job.shapes[0].sketch.as_ref().and_then(|k| k.entities.first()).map(|e| e.id).unwrap_or_default();
    run(&mut s, "sketch.dimension", json!({ "items": [format!("e{line}")] }));
    run(&mut s, "sketch.finish", json!({}));
    assert_eq!(s.ui.ribbon_tab, crate::cmd::RibbonTab::Home);
    // A plain shape from an older job becomes a sketch when it is edited.
    s.job.shapes[0].sketch = None;
    run(&mut s, "sketch.edit", json!({ "id": id }));
    assert!(s.job.shapes[0].sketch.is_some());
}

#[test]
fn everything_selected_moves_together_even_when_not_fully_defined() {
    let mut s = Session::new();
    run(&mut s, "sketch.new", json!({ "plane": "front" }));
    run(&mut s, "sketch.rectangle", json!({ "from": [50.0, 50.0], "to": [150.0, 120.0] }));
    run(&mut s, "sketch.arc", json!({ "start": [200.0, 50.0], "end": [260.0, 50.0], "through": [230.0, 80.0] }));
    run(&mut s, "sketch.circle", json!({ "centre": [300.0, 200.0], "radius": 25.0 }));
    let before: Vec<glam::DVec2> =
        s.job.shapes[0].sketch.as_ref().map(|k| k.points.iter().map(|p| p.at).collect()).unwrap_or_default();
    // Ctrl+A, then the selection dragged: every point moves by the same amount.
    run(&mut s, "sketch.select.all", Value::Null);
    run(&mut s, "sketch.move", json!({ "by": [-30.0, 40.0] }));
    let after: Vec<glam::DVec2> =
        s.job.shapes[0].sketch.as_ref().map(|k| k.points.iter().map(|p| p.at).collect()).unwrap_or_default();
    for (k, (a, b)) in before.iter().zip(&after).enumerate().skip(1) {
        assert!((*b - *a - glam::DVec2::new(-30.0, 40.0)).length() < 1e-6, "point {k}: {a:?} -> {b:?}");
    }
    assert_eq!(after.first(), Some(&glam::DVec2::ZERO), "the origin stays");
}

#[test]
fn the_program_starts_from_home_over_the_part_only_when_it_is_in_the_way() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 300.0 }));
    let id = run(&mut s, "shape.rectangle", json!({ "width": 120.0, "height": 90.0 }))["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let top = s.job.stock.top();
    let build = |s: &Session| crate::program::Program::build(&s.job.machine, &s.job.stock, &s.job.cuts, &s.job.ops);
    // From the left (the default): straight from home to the start, no climb.
    run(&mut s, "cut.profile", json!({ "shape": id, "bottom": "through" }));
    let prog = build(&s);
    assert_eq!(
        prog.start,
        crate::wire::WirePos::new(-s.job.machine.table_x, s.job.machine.wire_y0, 0.0),
        "starts at home"
    );
    assert!(prog.approach_end >= 1);
    assert!(prog.moves[..prog.approach_end].iter().all(|(_, m)| m.to.h < top), "no climb over the stock");
    let first_lead =
        prog.moves.iter().find(|(_, m)| m.kind == crate::wire::MoveKind::Lead).map(|(_, m)| m.to).unwrap_or_default();
    assert!(first_lead.u < 0.0, "enters from the left");
    // A start point on the right: in from the right, over the stock to get there.
    s.job.cuts.clear();
    run(&mut s, "cut.profile", json!({ "shape": id, "bottom": "through", "start_x": 100.0, "start_y": 150.0 }));
    let prog = build(&s);
    assert!(prog.moves[..prog.approach_end].iter().any(|(_, m)| m.to.h > top), "over the stock");
    let entry = prog.moves.get(prog.approach_end).map(|(_, m)| m.to).unwrap_or_default();
    assert!(entry.u > 0.0, "enters from the right: {entry:?}");
    // The post heats the wire once it is there, not at home.
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    let wire_on = text.find("M3").unwrap_or(0);
    assert!(
        text[..wire_on].matches("G0").count()
            + text[..wire_on].lines().filter(|l| l.starts_with('X') || l.starts_with('Y')).count()
            >= 2,
        "{text}"
    );
}

#[test]
fn a_swarf_cut_turns_the_table_as_the_wire_goes_down_and_simulates() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 300.0, "depth": 300.0, "height": 200.0 }));
    let mesh = crate::swarf::tests::twisted_block(30.0, 80);
    let mut model = crate::model::Model::new("model.1".into(), "twist".into(), String::new(), None, mesh)
        .unwrap_or_else(|e| panic!("{e}"));
    model.place(s.job.stock.bottom());
    // Its sides: the faces that rise.
    let tris = model.mesh.triangles().to_vec();
    let sides: Vec<usize> = (0..model.faces.faces.len())
        .filter(|&f| {
            let zs: Vec<f32> = model
                .faces
                .triangles_of(f)
                .filter_map(|t| tris.get(t))
                .flat_map(|t| model.mesh.corners(*t).map(|c| c.z))
                .collect();
            zs.iter().copied().fold(f32::MIN, f32::max) - zs.iter().copied().fold(f32::MAX, f32::min) > 1.0
        })
        .collect();
    s.job.models.push(model);
    let cut = run(&mut s, "cut.swarf", json!({ "faces": sides }));
    assert_eq!(s.job.cuts[0].keeps.len(), 4, "{cut}");
    assert!(s.job.cuts[0].warnings.is_empty(), "{:?}", s.job.cuts[0].warnings);
    // X, Y and B move together on the cutting moves.
    let text = run(&mut s, "post.gcode.preview", json!({}))["text"].as_str().unwrap_or_default().to_owned();
    let together =
        text.lines().filter(|l| !l.starts_with("G0") && l.contains('X') && l.contains('Y') && l.contains('B'));
    assert!(together.count() > 50, "{text}");
    // The waste falls away round the twisted block (and the column of foam above it).
    run(&mut s, "sim.jump.end", json!({}));
    let left = run(&mut s, "sim.get", json!({}))["foam_volume"].as_f64().unwrap_or(0.0);
    assert!(left > 0.9e6 && left < 1.5e6, "{left}");
    // The cut is saved with its faces and comes back the same.
    let saved = crate::jobfile::to_json(&s.job);
    let back = crate::jobfile::from_json(&saved).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.cuts.first().map(|c| c.kind), Some(crate::cut::CutKind::Swarf));
}

#[test]
fn several_models_cut_together_and_the_wire_keeps_out_of_the_others() {
    let dir = std::env::temp_dir().join(format!("cosmiccut-two-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("brick.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([80.0, 60.0, 100.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 400.0, "depth": 400.0, "height": 200.0 }));
    let a = run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Left" }));
    let b = run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Right" }));
    let (a, b) = (a["id"].as_str().unwrap_or_default().to_owned(), b["id"].as_str().unwrap_or_default().to_owned());
    // Move one across, and turn it: where it stands and how it is turned are kept.
    let moved = run(&mut s, "model.move", json!({ "id": b, "x": 150.0, "rz": 30.0, "rx": 0.0 }));
    assert!((moved["rotate"].as_f64().unwrap_or(0.0) - 30.0).abs() < 1e-9, "{moved}");
    let at = s.job.model(&b).map(|m| m.position(s.job.stock.bottom())).unwrap_or_default();
    assert!((at.x - 150.0).abs() < 1e-3 && at.z.abs() < 1e-3, "{at}");
    // Cutting the left one alone would take the wire through the right one at most angles.
    let e = s.execute("cut.model", &json!({ "models": [a], "divisions": 4 }));
    assert!(e.as_ref().err().is_some_and(|e| e.to_string().contains("Right")), "{e:?}");
    // Both together: the wire follows the outline of both.
    let c = run(&mut s, "cut.model", json!({ "models": [a, b], "divisions": 4 }));
    assert!(c["moves"].as_u64().unwrap_or(0) > 10, "{c}");
    // A profile through the middle of the table would cut the models.
    let shape = run(&mut s, "shape.rectangle", json!({ "width": 50.0, "height": 50.0 }));
    let e = s.execute("cut.profile", &json!({ "shape": shape["id"] }));
    assert!(e.as_ref().err().is_some_and(|e| e.to_string().contains("would cut into")), "{e:?}");
    // Saved and read back, the moved model stands where it was.
    let back = crate::jobfile::from_json(&crate::jobfile::to_json(&s.job)).unwrap_or_else(|e| panic!("{e}"));
    let m = back.models.iter().find(|m| m.id == b);
    assert!(m.is_some_and(|m| (m.rotate - 30.0).abs() < 1e-9 && !m.centred && (m.at.x - 150.0).abs() < 1e-9));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_later_operation_can_pass_through_parts_already_taken_off() {
    let dir = std::env::temp_dir().join(format!("cosmiccut-ops-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("part.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([60.0, 60.0, 100.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 600.0, "depth": 200.0, "height": 200.0 }));
    let id = |v: Value| v["id"].as_str().unwrap_or_default().to_owned();
    // Two parts at the ends of the block, cut in Op 1.
    let left = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Left" })));
    run(&mut s, "model.move", json!({ "id": left, "x": -220.0 }));
    // Ctrl+D puts a copy beside it; moved to the other end.
    run(&mut s, "selection.set", json!({ "id": left }));
    let right = id(run(&mut s, "edit.duplicate", json!({})));
    assert_ne!(right, left);
    let beside = s.job.model(&right).map(|m| m.position(s.job.stock.bottom()).x).unwrap_or_default();
    assert!((beside - (-220.0 + 60.0 + 25.4)).abs() < 1e-3, "{beside}");
    run(&mut s, "model.move", json!({ "id": right, "x": 220.0 }));
    // With the centre part still on the table, the ends cannot be cut from the model: away from
    // 0° the wire runs along the row through it, and at 0° a pass held at the bottom leaves level
    // out of the far side, through it.
    let centre = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Centre" })));
    assert!(s.execute("cut.model", &json!({ "models": [left, right], "divisions": 2 })).is_err());
    let ends = s.execute("cut.model", &json!({ "models": [left, right], "spacing": "degrees", "degrees": 180.0 }));
    assert!(ends.is_err());
    // In Op 2 the ends are gone. The cut alone fails...
    run(&mut s, "op.add", json!({ "name": "Centre parts" }));
    assert!(s.execute("cut.model", &json!({ "models": [centre], "divisions": 2 })).is_err());
    // ...unless the cut ignores them,
    run(&mut s, "cut.model", json!({ "models": [centre], "divisions": 2, "ignore": [left, right] }));
    // or its operation does.
    let op = s.job.active_op.clone();
    run(&mut s, "op.edit", json!({ "id": op, "ignore": [left, right] }));
    run(&mut s, "cut.model", json!({ "models": [centre], "divisions": 2 }));
    assert!(s.job.cuts.iter().all(|c| c.error.is_none()));
    // Taking the operation's list away leaves only the cut that ignores them itself working.
    run(&mut s, "op.edit", json!({ "id": op, "ignore": [] }));
    assert_eq!(s.job.cuts.iter().filter(|c| c.error.is_some()).count(), 1);
    // The list is saved with the job.
    run(&mut s, "op.edit", json!({ "id": op, "ignore": [left] }));
    let back = crate::jobfile::from_json(&crate::jobfile::to_json(&s.job)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.ops.iter().find(|o| o.id == op).map(|o| o.ignore.clone()), Some(vec![left.clone()]));
    // Copy and paste a model: the copy stands beside it too.
    run(&mut s, "selection.set", json!({ "id": centre }));
    run(&mut s, "edit.copy", json!({}));
    let pasted = id(json!({ "id": run(&mut s, "edit.paste", json!({}))["pasted"] }));
    let x = s.job.model(&pasted).map(|m| m.position(s.job.stock.bottom()).x).unwrap_or_default();
    assert!((x - (60.0 + 25.4)).abs() < 1e-3, "{x}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_profile_can_leave_lines_out_and_start_from_the_end_chosen() {
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 600.0, "depth": 200.0, "height": 200.0 }));
    let shape = run(&mut s, "shape.rectangle", json!({ "width": 100.0, "height": 100.0 }));
    let id = shape["id"].as_str().unwrap_or_default().to_owned();
    // A part at the left end of the block, standing on its bottom.
    let at = json!({ "shape": id, "anchor": "bottom_left", "x": -280.0, "y": 0.0, "bottom": "through" });
    let lines = {
        let cx = s.job.context();
        crate::cut::profile_lines(&cx, &at).unwrap_or_else(|e| panic!("{e}")).1
    };
    assert_eq!(lines.len(), 4, "{lines:?}");
    // Leave out its bottom, and start at the bottom-left corner.
    let bottom =
        lines.iter().find(|(_, pts)| pts.iter().all(|p| p.y.abs() < 1e-6)).map(|(id, _)| *id).unwrap_or_default();
    let mut p = at.clone();
    p["skip"] = json!([bottom]);
    p["start_x"] = json!(-280.0);
    p["start_y"] = json!(0.0);
    run(&mut s, "cut.profile", p);
    let cut = &s.job.cuts[0];
    assert!(cut.error.is_none(), "{:?}", cut.error);
    let moves: Vec<_> = cut.path.moves.iter().filter(|m| m.kind != crate::wire::MoveKind::Rapid).collect();
    // In from the left at the bottom, round the part, and out to the left again along the bottom:
    // the part-off goes to the outside, never towards the middle of the block.
    let first = moves.first().map(|m| m.to).unwrap_or_default();
    let last = moves.last().map(|m| m.to).unwrap_or_default();
    assert!((first.u + 280.0).abs() < 1e-6 && first.h.abs() < 1e-6, "{first:?}");
    assert!(last.u < -300.0 && last.h.abs() < 1e-6, "{last:?}");
    assert!(moves.iter().all(|m| m.to.u <= -180.0 + 1e-6), "the wire stays at the left end");
    // The simulation keeps the part.
    let keep = cut.keeps.first().map(|k| k.regions.clone()).unwrap_or_default();
    assert!(CosmicCut_geom::region::contains(&keep, glam::DVec2::new(-230.0, 50.0)));
    assert!(!CosmicCut_geom::region::contains(&keep, glam::DVec2::new(0.0, 50.0)));
}

#[test]
fn swarf_backs_out_rather_than_cut_a_neighbour_and_sweeps_level_faces() {
    use CosmicCut_geom::faces::FaceKind;
    let dir = std::env::temp_dir().join(format!("cosmiccut-swarf-row-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("block.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([60.0, 60.0, 100.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 600.0, "depth": 200.0, "height": 200.0 }));
    let id = |v: Value| v["id"].as_str().unwrap_or_default().to_owned();
    // Three blocks in a row; the left one stands 20 mm up, on foam.
    let left = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Left" })));
    run(&mut s, "model.move", json!({ "id": left, "x": -200.0, "z": 20.0 }));
    let centre = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Centre" })));
    let right = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "Right" })));
    run(&mut s, "model.move", json!({ "id": right, "x": 200.0 }));
    // The left block's faces by the way they face.
    let face = |dir: glam::Vec3| {
        s.job.model(&left).and_then(|m| {
            m.faces.faces.iter().position(|f| match f.kind {
                FaceKind::Plane { normal, .. } => normal.dot(dir) > 0.99,
                _ => false,
            })
        })
    };
    let (inner, bottom) = (face(glam::Vec3::X).unwrap_or(0), face(glam::Vec3::NEG_Z).unwrap_or(0));
    // The inner face: out level through the waste would run into the centre block, so the wire
    // backs up its own kerf and leaves above the foam.
    let cut = s.execute("cut.swarf", &json!({ "model": left, "faces": [inner] }));
    assert!(cut.is_ok(), "{cut:?}");
    let c = s.job.cuts.last().map(|c| c.path.clone()).unwrap_or_default();
    let centre_edge = -30.0 - 1.0;
    assert!(c.moves.iter().all(|m| m.to.u < centre_edge || m.to.h > 200.0), "the wire stays clear of the centre");
    assert!(
        c.moves.iter().any(|m| m.kind == crate::wire::MoveKind::Lead && m.to.h > 200.0),
        "it backs out over the top"
    );
    // The bottom, a level face: swept in from outside, under the block, and back out.
    run(&mut s, "cut.swarf", json!({ "model": left, "faces": [bottom] }));
    let c = s.job.cuts.last().map(|c| c.path.clone()).unwrap_or_default();
    let level: Vec<_> = c.moves.iter().filter(|m| m.kind == crate::wire::MoveKind::Lead).collect();
    assert!(level.iter().all(|m| (m.to.h - (20.0 - s.job.machine.kerf / 2.0)).abs() < 1e-6), "{level:?}");
    // The centre block's bottom is on the table: nothing to cut under it, and it says so.
    let on_table = s.job.model(&centre).and_then(|m| {
        m.faces.faces.iter().position(|f| matches!(f.kind, FaceKind::Plane { normal, .. } if normal.z < -0.99))
    });
    run(&mut s, "cut.swarf", json!({ "model": centre, "faces": [on_table], "ignore": [left] }));
    let w = s.job.cuts.last().map(|c| c.warnings.clone()).unwrap_or_default();
    assert!(w.iter().any(|w| w.contains("no foam under it")), "{w:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_swarf_cut_takes_faces_from_several_models() {
    use CosmicCut_geom::faces::FaceKind;
    let dir = std::env::temp_dir().join(format!("cosmiccut-swarf-two-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("block.stl").to_string_lossy().into_owned();
    let _ = std::fs::write(&path, box_stl([60.0, 60.0, 100.0]));
    let mut s = Session::new();
    run(&mut s, "stock.block", json!({ "width": 500.0, "depth": 200.0, "height": 200.0 }));
    let id = |v: Value| v["id"].as_str().unwrap_or_default().to_owned();
    let a = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "A" })));
    run(&mut s, "model.move", json!({ "id": a, "x": -120.0 }));
    let b = id(run(&mut s, "model.import", json!({ "path": path, "units": "mm", "name": "B" })));
    run(&mut s, "model.move", json!({ "id": b, "x": 120.0 }));
    let face = |m: &str, dir: glam::Vec3| {
        s.job.model(m).and_then(|m| {
            m.faces
                .faces
                .iter()
                .position(|f| matches!(f.kind, FaceKind::Plane { normal, .. } if normal.dot(dir) > 0.99))
        })
    };
    let (a_out, a_in) = (face(&a, -glam::Vec3::X), face(&a, glam::Vec3::X));
    let (b_out, b_in) = (face(&b, glam::Vec3::X), face(&b, -glam::Vec3::X));
    // The outside face of each block, in one cut.
    let parts = json!([{ "model": a, "faces": [a_out] }, { "model": b, "faces": [b_out] }]);
    run(&mut s, "cut.swarf", json!({ "parts": parts }));
    let cut = s.job.cuts.last();
    assert!(cut.is_some_and(|c| c.error.is_none() && c.keeps.len() == 2), "{:?}", cut.map(|c| &c.error));
    // A face of A that would take the wire through B going out level backs out instead; the cut
    // is still made.
    let inner = json!([{ "model": a, "faces": [a_in] }, { "model": b, "faces": [b_in] }]);
    let e = s.execute("cut.swarf", &json!({ "parts": inner }));
    assert!(e.is_ok(), "{e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
