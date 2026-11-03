//! The 3D viewport: the turntable, the foam, the models, the cut paths and the wire, drawn with
//! wgpu. Geometry is built in the table frame; while simulating, it is drawn through a camera
//! turned against B, so the turntable (with the foam on it) turns and the wire, its towers and
//! the floor grid stay put.

mod dims;
mod gizmo;
mod gpu;
mod overlay;
mod scene;
mod sketch;

use std::panic::{AssertUnwindSafe, catch_unwind};

use CosmicCut_engine::cut::Built;
use CosmicCut_engine::wire::WirePos;
use CosmicCut_engine::{Display, Session, Theme};
use CosmicCut_geom::{Aabb, Vec3};
use CosmicCut_render::{OrbitCamera, SceneGeometry, TriadStyle};
use egui::{Color32, Pos2, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::Request;
use crate::navigation::{self, NavCaps};
use crate::theme::{self, Tokens};
use crate::ui_kit::{Container, Surface};

/// What the uploaded scene was built from: theme, job revision, display, units, selection,
/// simulating, preview, paths hidden.
/// What the uploaded scene was built from; the last, whether the view looks up from under the
/// table (the turntable is then see-through, so faces underneath can be picked).
type SceneKey = (Theme, u64, Display, bool, Option<String>, bool, usize, bool, bool);

pub struct Viewport {
    /// The cut being set up in the cut panel, drawn over the scene; set by the app each frame.
    pub preview: Option<Built>,
    /// The stock is being set up: no cut paths, and its dimensions drawn and editable.
    pub stock_edit: bool,
    /// New sketch was clicked: the planes are shown to pick one from.
    pub picking_plane: bool,
    /// What to do next in the viewport, for the Instructions tab.
    pub instructions: String,
    /// Picking a cut's start point on the plane the wire cuts in at this table angle (degrees),
    /// and what came of it: the point (`u`, `h`), or None if cancelled.
    pub pick_start: Option<f64>,
    pub picked_start: Option<Option<glam::DVec2>>,
    /// A stock dimension being typed in the viewport.
    dim_edit: dims::DimEdit,
    /// Drawing in a sketch.
    sketch_ui: sketch::SketchUi,
    gpu: Option<gpu::ViewportGpu>,
    gpu_error: Option<String>,
    /// The camera on screen; eases towards the session's camera.
    shown: OrbitCamera,
    scene_key: Option<SceneKey>,
    scene_bounds: Aabb,
    fit_pending: bool,
    last_render_time: Option<f64>,
    fps: f32,
    cube_hover: Option<overlay::CubeHover>,
    /// The foam last drawn: (voxel revision, theme, coarse), and when.
    foam_key: Option<(u64, Theme, bool)>,
    foam_built: f64,
    /// Where the wire was last drawn.
    wire_key: Option<[i64; 4]>,
    /// The model being laid flat: clicking one of its faces puts that face on the table.
    pub lay_flat: Option<String>,
    /// The model the Move tool's handles are on.
    pub moving: Option<String>,
    gizmo: gizmo::Gizmo,
    /// The triangle of that model under the pointer, and the face last highlighted.
    hover_triangle: Option<usize>,
    highlight_key: Option<String>,
    /// A swarf cut's model, its faces, and whether more are being picked; set by the app each
    /// frame. A face clicked, and Esc to stop picking, go back the same way.
    pub face_pick: Option<FacePick>,
    pub picked_face: Option<(String, Vec<usize>)>,
    pub stop_face_pick: bool,
    /// A profile's lines while picking which to leave out: the table angle, each line (id,
    /// points in the cut plane, left out), whether picking; set by the app each frame. The line
    /// clicked, and Esc, go back the same way.
    pub line_pick: Option<LinePick>,
    pub picked_line: Option<u32>,
    pub stop_line_pick: bool,
    /// The faces a click would pick now: (model, face under the pointer, Alt held, the faces).
    hover_surface: Option<(String, usize, bool, Vec<usize>)>,
    /// The turntable's angle the frame is drawn at, degrees.
    table_b: f64,
    /// Scene built before the GPU existed, uploaded once it does.
    pending_upload: Option<(SceneGeometry, SceneGeometry)>,
    animating: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Self::new()
    }
}

impl Viewport {
    pub fn new() -> Self {
        Self {
            preview: None,
            stock_edit: false,
            picking_plane: false,
            instructions: String::new(),
            pick_start: None,
            picked_start: None,
            dim_edit: None,
            sketch_ui: sketch::SketchUi::default(),
            gpu: None,
            gpu_error: None,
            shown: OrbitCamera::default(),
            scene_key: None,
            scene_bounds: Aabb::EMPTY,
            fit_pending: true,
            last_render_time: None,
            fps: 0.0,
            cube_hover: None,
            foam_key: None,
            foam_built: f64::NEG_INFINITY,
            wire_key: None,
            lay_flat: None,
            hover_triangle: None,
            highlight_key: None,
            face_pick: None,
            picked_face: None,
            stop_face_pick: false,
            hover_surface: None,
            line_pick: None,
            picked_line: None,
            stop_line_pick: false,
            moving: None,
            gizmo: gizmo::Gizmo::default(),
            table_b: 0.0,
            pending_upload: None,
            animating: false,
        }
    }

    /// Whether the viewport takes this key itself just now (R and C mid-line in a sketch).
    pub fn takes_key(&self, key: egui::Key) -> bool {
        self.sketch_ui.chaining() && matches!(key, egui::Key::R | egui::Key::C)
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut Session,
        tokens: &Tokens,
        render_state: Option<&RenderState>,
        requests: &mut Vec<Request>,
    ) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if !(rect.width() >= 2.0 && rect.height() >= 2.0) {
            return;
        }
        session.set_viewport_aspect(rect.aspect_ratio());
        self.rebuild_scene_if_needed(session, tokens);
        if self.fit_pending && self.scene_key.is_some() {
            self.fit_pending = false;
            requests.push(("view.fit".into(), Value::Null));
        }
        let mut cam = self.shown;
        cam.projection = session.view.camera.projection;
        // The Move tool's handles, on a model that is still there, out of the simulation and sketches.
        if self.moving.as_deref().is_some_and(|id| session.job.model(id).is_none()) {
            self.moving = None;
        }
        let moving = self.moving.clone().filter(|_| session.sim.is_none() && session.view.sketch.is_none());
        let mut gizmo = std::mem::take(&mut self.gizmo);
        let grab = match moving.as_deref().and_then(|id| session.job.model(id)) {
            Some(m) => {
                let project = |p: glam::DVec3| self.project(rect, p.as_vec3());
                let basis = table_camera(&self.shown, self.table_b).basis();
                let view = gizmo::View {
                    rect,
                    project: &project,
                    toward: -basis.forward.as_dvec3(),
                    right: basis.right.as_dvec3(),
                };
                gizmo.grabs(ui, &view, m)
            }
            None => false,
        };
        self.gizmo = gizmo;
        // In a sketch the left button draws and drags, and on a Move handle it drags the handle;
        // the view pans and zooms but does not turn.
        let caps = if session.view.sketch.is_some() || grab {
            NavCaps { orbit: false, ..NavCaps::FULL }
        } else {
            NavCaps::FULL
        };
        if navigation::handle(ui, &response, &mut cam, caps) {
            session.view.camera = cam;
            self.shown = cam;
        }
        self.animate(ui, session);
        self.table_b = session.sim.as_deref().map_or(0.0, |s| s.wire().b);
        self.pick_face(ui, &response, rect, session, requests);
        self.update_highlight(session, tokens);
        self.update_foam(ui, session, tokens);
        self.update_wire(session, tokens);

        let painter = ui.painter_at(rect);
        match self.render(ui, rect, tokens, render_state) {
            Some(texture) => {
                painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(rect, 0.0, tokens.viewport_bottom);
                let msg = self.gpu_error.as_deref().unwrap_or("3D viewport needs the wgpu renderer");
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    msg,
                    egui::FontId::proportional(14.0),
                    tokens.text_muted,
                );
            }
        }

        // A sketch's plane grid, under the overlays.
        if session.view.sketch.is_some() && session.sim.is_none() {
            let to_f32 = |p: glam::DVec3| Vec3::new(p.x as f32, p.y as f32, p.z as f32);
            let project = |p: glam::DVec3| self.project(rect, to_f32(p));
            let ray = |p: Pos2| {
                self.ray(rect, p).map(|(o, d)| {
                    (
                        glam::DVec3::new(o.x.into(), o.y.into(), o.z.into()),
                        glam::DVec3::new(d.x.into(), d.y.into(), d.z.into()),
                    )
                })
            };
            sketch::plane_grid(&painter, &sketch::View { rect, project: &project, ray: &ray }, session, tokens);
        }
        let display = session.prefs.display;
        if display.view_cube {
            let cube_rect = overlay::cube_rect(rect);
            let cube_response = ui.interact(cube_rect, ui.id().with("view_cube"), Sense::click());
            self.cube_hover = cube_response.hover_pos().and_then(|p| overlay::cube_hit(&self.shown, cube_rect, p));
            if cube_response.clicked()
                && let Some(hit) = self.cube_hover
            {
                let (yaw, pitch) = hit.view_degrees();
                requests.push(("view.camera.set".into(), json!({ "yaw": yaw, "pitch": pitch })));
            }
            if self.cube_hover.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            overlay::paint_view_cube(&painter, cube_rect, &self.shown, tokens, self.cube_hover);
        } else {
            self.cube_hover = None;
        }
        view_tools(ui, rect, session, tokens, requests);
        if self.stock_edit && display.stock && session.sim.is_none() {
            let cam = table_camera(&self.shown, self.table_b);
            let basis = cam.basis();
            let stock = session.job.stock;
            let mut edit = self.dim_edit.take();
            let view =
                dims::View { rect, toward: -basis.forward, right: basis.right, project: |p| self.project(rect, p) };
            dims::show(ui, view, &stock, session.prefs.units, tokens, &mut edit, requests);
            self.dim_edit = edit;
        } else {
            self.dim_edit = None;
        }
        if session.sim.is_none() {
            let mut sketch_ui = std::mem::take(&mut self.sketch_ui);
            let to_f32 = |p: glam::DVec3| Vec3::new(p.x as f32, p.y as f32, p.z as f32);
            let project = |p: glam::DVec3| self.project(rect, to_f32(p));
            let ray = |p: Pos2| {
                self.ray(rect, p).map(|(o, d)| {
                    (
                        glam::DVec3::new(o.x.into(), o.y.into(), o.z.into()),
                        glam::DVec3::new(d.x.into(), d.y.into(), d.z.into()),
                    )
                })
            };
            let view = sketch::View { rect, project: &project, ray: &ray };
            sketch_ui.show(ui, &view, &response, session, tokens, requests);
            let mut instructions = if session.view.sketch.is_some() {
                sketch_ui.instructions.clone()
            } else if self.lay_flat.is_some() {
                "Bottom face: click the face of the model to put down on the table. Esc cancels.".to_owned()
            } else if self.face_pick.as_ref().is_some_and(|(_, on)| *on) {
                "Swarf faces: click a face of the model for the wire to follow; the click takes the whole surface \
                 the wire can follow from it (Alt-click takes the face alone), and a second click takes it away. \
                 The wire lies along the faces at every height, so they must be made of level straight lines.\n\
                 Esc or right-click when done."
                    .to_owned()
            } else {
                IDLE.to_owned()
            };
            let mut picking = self.picking_plane;
            if picking {
                picking = session.view.sketch.is_none()
                    && sketch::pick_plane(ui, &view, &response, session, tokens, &mut instructions, requests);
            }
            let mut picked = None;
            if let Some(b) = self.pick_start {
                instructions = "Cut start: click where the cut should start, on the plane the wire cuts in. The wire \
                                goes into the outline at its point nearest there.\nEsc cancels."
                    .to_owned();
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(pos) = response.interact_pointer_pos()
                    && let Some((o, d)) = ray(pos)
                {
                    // The plane through the turntable's axis square to the wire at angle b.
                    let n = CosmicCut_engine::wire::wire_dir(b);
                    let dn = d.dot(n);
                    if dn.abs() > 1.0e-9 {
                        let p = o + d * (-o.dot(n) / dn);
                        let u = p.dot(CosmicCut_engine::wire::u_axis(b));
                        picked = Some(Some(glam::DVec2::new(u, p.z)));
                    }
                } else if response.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    picked = Some(None);
                }
            }
            self.picking_plane = picking;
            if picked.is_some() {
                self.picked_start = picked;
            }
            // With no tool using the pointer, a click picks the model under it (or nothing), and
            // the Move handles follow it.
            let free = session.view.sketch.is_none()
                && !self.picking_plane
                && self.pick_start.is_none()
                && self.lay_flat.is_none()
                && !self.face_pick.as_ref().is_some_and(|(_, on)| *on)
                && self.line_pick.is_none()
                && !grab;
            if free && response.clicked_by(egui::PointerButton::Primary) {
                let hit = response.interact_pointer_pos().and_then(|p| self.ray(rect, p)).and_then(|(o, d)| {
                    session
                        .job
                        .models
                        .iter()
                        .filter(|m| m.visible)
                        .filter_map(|m| m.mesh.raycast(o, d).map(|(t, _)| (t, m.id.clone())))
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, id)| id)
                });
                if hit.is_some() || session.view.selection.as_deref().is_some_and(|s| s.starts_with("model.")) {
                    requests.push(("selection.set".into(), json!({ "id": hit })));
                }
                if let Some(id) = &hit
                    && self.moving.is_some()
                {
                    self.moving = Some(id.clone());
                }
            }
            if let Some((b, lines, true)) = &self.line_pick {
                instructions = "Leave out lines: click a line of the shape to leave it out of the cut (dashed), or \
                                click a dashed one to put it back. Pick the start point (Path) to say which end the \
                                wire starts from.\nEsc or right-click when done."
                    .to_owned();
                let to_screen = |q: &glam::DVec2| {
                    let p = CosmicCut_engine::wire::on_wire(&CosmicCut_engine::wire::WirePos::new(q.x, q.y, *b), 0.0);
                    self.project(rect, p.as_vec3())
                };
                let screen: Vec<(u32, Vec<Pos2>, bool)> = lines
                    .iter()
                    .map(|(id, pts, out)| (*id, pts.iter().filter_map(to_screen).collect(), *out))
                    .collect();
                let pointer = response.hover_pos();
                let near = pointer.and_then(|p| {
                    screen
                        .iter()
                        .map(|(id, pts, _)| {
                            let d = pts
                                .windows(2)
                                .filter_map(|w| match w {
                                    [a, c] => Some(segment_distance(p, *a, *c)),
                                    _ => None,
                                })
                                .fold(f32::INFINITY, f32::min);
                            (*id, d)
                        })
                        .filter(|(_, d)| *d < 8.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(id, _)| id)
                });
                let painter = ui.painter_at(rect);
                for (id, pts, out) in &screen {
                    let lit = near == Some(*id);
                    let colour = match (out, lit) {
                        (_, true) => tokens.warning,
                        (true, false) => tokens.text_muted,
                        (false, false) => tokens.accent,
                    };
                    let width = if lit { 4.0 } else { 2.5 };
                    if *out {
                        for w in pts.windows(2) {
                            if let [a, c] = w {
                                painter.add(egui::Shape::dashed_line(
                                    &[*a, *c],
                                    egui::Stroke::new(width, colour),
                                    6.0,
                                    5.0,
                                ));
                            }
                        }
                    } else {
                        painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(width, colour)));
                    }
                }
                if near.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if response.clicked_by(egui::PointerButton::Primary) {
                    self.picked_line = near;
                }
                if response.secondary_clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.stop_line_pick = true;
                }
            }
            if let Some(m) = moving.as_deref().and_then(|id| session.job.model(id)) {
                let mut gizmo = std::mem::take(&mut self.gizmo);
                let project = |p: glam::DVec3| self.project(rect, p.as_vec3());
                let basis = table_camera(&self.shown, self.table_b).basis();
                let view = gizmo::View {
                    rect,
                    project: &project,
                    toward: -basis.forward.as_dvec3(),
                    right: basis.right.as_dvec3(),
                };
                let floor = session.job.stock.bottom();
                let done = gizmo.show(ui, &view, &response, m, floor, session.prefs.units, tokens, requests);
                instructions = gizmo.instructions().to_owned();
                self.gizmo = gizmo;
                if done {
                    self.moving = None;
                }
            }
            self.instructions = instructions;
            self.sketch_ui = sketch_ui;
        }
        if self.lay_flat.is_some() {
            lay_flat_hint(ui, rect, tokens);
        } else if session.sim.is_some() {
            playback(ui, rect, session, tokens, requests);
        } else if session.job.cuts.is_empty() && session.view.sketch.is_none() && !self.picking_plane {
            hint(ui, rect, session, tokens);
        }
        if display.orientation_triad {
            overlay::paint_triad(&painter, overlay::triad_anchor(rect), &self.shown, tokens);
        }
        if display.readout {
            overlay::paint_readout(
                &painter,
                Pos2::new(rect.right() - 10.0, rect.bottom() - 8.0),
                &self.shown,
                self.fps,
                session.prefs.fps_limit,
                tokens,
            );
        }
    }

    fn rebuild_scene_if_needed(&mut self, session: &mut Session, tokens: &Tokens) {
        let display = session.prefs.display;
        let inch = session.prefs.units == CosmicCut_engine::units::DisplayUnits::Inches;
        let simulating = session.sim.is_some();
        let preview_id = self.preview.as_ref().map_or(0, |b| b.path.moves.len() ^ (b.path.moves.as_ptr() as usize));
        let key: SceneKey = (
            session.prefs.theme,
            session.job.revision,
            display,
            inch,
            session.view.selection.clone(),
            simulating,
            preview_id,
            self.stock_edit,
            self.shown.eye().z < -0.6,
        );
        if self.scene_key.as_ref() == Some(&key) {
            return;
        }
        let (palette, _, machine) = scene::colours(tokens);
        let job = &session.job;
        let mut content = SceneGeometry::default();
        // Seen from below, the turntable is drawn last and faint, so the models show through it.
        let below = key.8;
        if !below {
            scene::turntable(&mut content, job.machine.table_radius as f32, machine, palette.grid_major);
        }
        // Models (before the see-through stock, which would hide them), with their outline
        // where the mesh is small enough to work it out quickly.
        // The selected one tinted, its edges in the accent.
        let accent = theme::linear(tokens.accent);
        for m in job.models.iter().filter(|m| m.visible && !simulating) {
            let picked = session.view.selection.as_deref() == Some(m.id.as_str());
            let edge = if picked { accent } else { palette.part_edge };
            let edges = (m.mesh.triangles().len() < 300_000).then_some(edge);
            let fill = if picked {
                let mix = |a: f32, b: f32| a + (b - a) * 0.3;
                [
                    mix(palette.part[0], accent[0]),
                    mix(palette.part[1], accent[1]),
                    mix(palette.part[2], accent[2]),
                    palette.part[3],
                ]
            } else {
                palette.part
            };
            content.push_mesh(&m.mesh, fill, edges);
        }
        if display.stock && !simulating {
            let fill = theme::linear(tokens.stock.gamma_multiply(0.16));
            scene::stock(&mut content, &job.stock, fill, palette.stock_edge);
        }
        if below {
            let faint = [machine[0], machine[1], machine[2], 0.15];
            scene::turntable(&mut content, job.machine.table_radius as f32, faint, palette.grid_major);
        }
        // The cut paths: those shown, the selected one bold; the preview over them.
        let selected = session.view.selection.as_deref();
        // While simulating, the foam stands in for the stock and the paths.
        let previewing = self.preview.is_some();
        // While the stock is set up, no paths at all: just the stock and the models.
        let hidden = self.stock_edit;
        // What the machine does, as the simulation plays it and the post writes it: the program,
        // from home to each cut, between them, the part-off and home again, in pieces by cut.
        if !previewing && !simulating && !hidden {
            let prog = CosmicCut_engine::program::Program::build(&job.machine, &job.stock, &job.cuts, &job.ops);
            let release = prog.release_at.unwrap_or(usize::MAX);
            let home = prog.home_at.unwrap_or(usize::MAX);
            let mut pieces: Vec<(Option<usize>, bool, CosmicCut_engine::wire::WirePath)> = Vec::new();
            let mut at = prog.start;
            for (i, (owner, m)) in prog.moves.iter().enumerate() {
                let parting = (release..home).contains(&i);
                let new_piece = pieces.last().is_none_or(|(o, r, _)| o != owner || *r != parting);
                if new_piece {
                    pieces.push((*owner, parting, CosmicCut_engine::wire::WirePath::new(at)));
                }
                if let Some((_, _, path)) = pieces.last_mut() {
                    path.moves.push(*m);
                }
                at = m.to;
            }
            let cut_selected = selected.is_some_and(|s| s.starts_with("cut."));
            let dim = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * 0.35];
            for (owner, parting, path) in &pieces {
                let cut = owner.and_then(|ci| job.cuts.get(ci));
                // A hidden cut is left out; the moves between cuts show with the rest.
                if cut.is_some_and(|c| !c.visible && selected != Some(c.id.as_str())) {
                    continue;
                }
                let bold = cut.is_some_and(|c| selected == Some(c.id.as_str()));
                let mut p = palette;
                if *parting && (bold || !cut_selected) {
                    // The part-off, in red.
                    p.path_cut = theme::linear(tokens.error);
                    p.path_lead = p.path_cut;
                } else if !bold && cut_selected {
                    p.path_cut = dim(p.path_cut);
                    p.path_lead = dim(p.path_lead);
                    p.path_rapid = dim(p.path_rapid);
                    p.path_link = dim(p.path_link);
                }
                let rulings = display.rulings && cut.is_some() && !*parting && (bold || selected.is_none());
                scene::path(&mut content, path, &job.stock, &p, rulings, bold);
            }
        }
        if let Some(b) = self.preview.as_ref().filter(|_| !hidden) {
            scene::path(&mut content, &b.path, &job.stock, &palette, display.rulings, true);
            if let Some(rel) = &b.release {
                let mut red = palette;
                red.path_cut = theme::linear(tokens.error);
                red.path_lead = red.path_cut;
                scene::path(&mut content, rel, &job.stock, &red, false, true);
            }
        }
        let jb = job.bounds();
        content.bounds.include(jb.min);
        content.bounds.include(jb.max);
        // While simulating, the grid is drawn with the wire, still while the table turns.
        let mut world = if display.grid && !simulating {
            scene::grid(&content.bounds, &palette, sketch::grid_step(session.prefs.units) as f32)
        } else {
            SceneGeometry::default()
        };
        world.triangles.extend_from_slice(&content.triangles);
        world.lines.extend_from_slice(&content.lines);
        let mut triad = SceneGeometry::default();
        let size = (content.bounds.radius() * 0.18).clamp(15.0, 200.0);
        triad.push_triad(Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], size, &palette, TriadStyle::Faint);

        // Re-frame when the job itself changes (opened, new), not on every edit.
        let reframe = self.scene_key.as_ref().map(|k| k.1.abs_diff(key.1) >= 500).unwrap_or(true);
        session.set_scene_bounds(jb);
        self.scene_bounds = Aabb::from_points(world.lines.iter().chain(&world.triangles).map(|v| Vec3::from(v.pos)));
        self.scene_bounds.include(content.bounds.min);
        self.scene_bounds.include(content.bounds.max);
        if reframe {
            self.fit_pending = true;
        }
        self.wire_key = None;
        if let Some(gpu) = &mut self.gpu {
            let r = catch_unwind(AssertUnwindSafe(|| {
                gpu.set_scene(&world);
                gpu.set_overlay(&triad);
            }));
            if r.is_err() {
                self.fail("uploading the scene to the GPU failed");
                return;
            }
        } else {
            self.pending_upload = Some((world, triad));
        }
        self.scene_key = Some(key);
    }

    /// The simulated foam, meshed again when the voxels change (less often while playing).
    fn update_foam(&mut self, ui: &Ui, session: &Session, tokens: &Tokens) {
        let Some(gpu) = &mut self.gpu else { return };
        let Some(sim) = session.sim.as_deref() else {
            if self.foam_key.take().is_some() {
                gpu.set_sim_stock(&SceneGeometry::default());
            }
            return;
        };
        let Some(v) = &sim.voxels else { return };
        let now = ui.input(|i| i.time);
        let coarse = sim.playing;
        let key = (v.revision, session.prefs.theme, coarse);
        if self.foam_key == Some(key) || (sim.playing && now - self.foam_built < 0.25 && self.foam_key.is_some()) {
            return;
        }
        let stock = theme::linear(tokens.stock.gamma_multiply(0.75));
        let cut = theme::linear(tokens.accent.gamma_multiply(0.85));
        let g = scene::foam(v, stock, cut, coarse);
        gpu.set_sim_stock(&g);
        self.foam_key = Some(key);
        self.foam_built = now;
    }

    /// The wire and its towers: where the simulation has it, else waiting at the program's start.
    fn update_wire(&mut self, session: &Session, tokens: &Tokens) {
        let Some(gpu) = &mut self.gpu else { return };
        let job = &session.job;
        let pos = match session.sim.as_deref() {
            Some(sim) => sim.wire(),
            // Out of the simulation the wire waits at home, machine X0 Y0.
            None => CosmicCut_engine::wire::WirePos::new(-job.machine.table_x, job.machine.wire_y0, 0.0),
        };
        let shown = session.prefs.display.machine;
        let key = [
            (pos.u * 100.0) as i64,
            (pos.h * 100.0) as i64,
            (pos.b * 100.0) as i64,
            i64::from(shown)
                | (i64::from(session.prefs.theme == Theme::Light) << 1)
                | (i64::from(session.sim.is_some() && session.prefs.display.grid) << 2)
                | ((job.revision as i64) << 3),
        ];
        if self.wire_key == Some(key) {
            return;
        }
        let mut g = SceneGeometry::default();
        if shown {
            let (_, wire, machine) = scene::colours(tokens);
            scene::machine(&mut g, session, &pos, wire, machine);
        }
        // The floor grid stays still while the table turns: drawn turned back by B.
        if session.sim.is_some() && session.prefs.display.grid {
            let mut grid =
                scene::grid(&job.bounds(), &tokens.scene_palette(), sketch::grid_step(session.prefs.units) as f32);
            scene::rotate_z(&mut grid, -pos.b);
            g.lines.extend_from_slice(&grid.lines);
        }
        gpu.set_sim_tool(&g);
        self.wire_key = Some(key);
    }

    fn animate(&mut self, ui: &Ui, session: &Session) {
        let goal = session.view.camera.sanitized();
        self.animating = !self.shown.approx_eq(&goal);
        if !self.animating {
            self.shown = goal;
            return;
        }
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.1);
        self.shown = self.shown.lerp(&goal, 1.0 - (-dt * 14.0).exp());
        let fps = session.prefs.fps_limit.max(1) as f32;
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(1.0 / fps));
    }

    fn render(
        &mut self,
        ui: &Ui,
        rect: Rect,
        tokens: &Tokens,
        render_state: Option<&RenderState>,
    ) -> Option<egui::TextureId> {
        if self.gpu_error.is_some() {
            return None;
        }
        let render_state = render_state?;
        if self.gpu.is_none() {
            match catch_unwind(AssertUnwindSafe(|| gpu::ViewportGpu::new(render_state))) {
                Ok(mut g) => {
                    if let Some((world, triad)) = self.pending_upload.take() {
                        g.set_scene(&world);
                        g.set_overlay(&triad);
                    }
                    self.gpu = Some(g);
                    self.foam_key = None;
                    self.wire_key = None;
                }
                Err(_) => {
                    self.fail("could not create the 3D viewport on this GPU");
                    return None;
                }
            }
        }
        let ppp = ui.ctx().pixels_per_point();
        let size_px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
        let cam = table_camera(&self.shown, self.table_b);
        let b = cam.basis();
        let radius = self.scene_bounds.radius() + cam.target.distance(self.scene_bounds.center());
        let params = gpu::FrameParams {
            size_px,
            view_proj: cam.view_proj(rect.aspect_ratio(), radius),
            light_dir: -b.forward + b.up * 0.35 + b.right * 0.25,
            bg_top: theme::linear(tokens.viewport_top),
            bg_bottom: theme::linear(tokens.viewport_bottom),
        };
        let gpu = self.gpu.as_mut()?;
        if let Some(e) = gpu.error() {
            self.fail(&format!("the GPU reported an error: {e}"));
            return None;
        }
        match catch_unwind(AssertUnwindSafe(|| gpu.render(&params))) {
            Ok(id) => {
                let now = ui.input(|i| i.time);
                if let Some(last) = self.last_render_time {
                    let dt = (now - last) as f32;
                    if dt > 0.0 && dt < 1.0 {
                        let inst = 1.0 / dt;
                        self.fps = if self.fps > 0.0 { self.fps * 0.9 + inst * 0.1 } else { inst };
                    }
                }
                self.last_render_time = Some(now);
                id
            }
            Err(_) => {
                self.fail("the GPU rejected a viewport frame");
                None
            }
        }
    }

    /// Where table-frame point `p` is drawn on screen.
    fn project(&self, rect: Rect, p: Vec3) -> Option<Pos2> {
        let cam = table_camera(&self.shown, self.table_b);
        let radius = self.scene_bounds.radius() + cam.target.distance(self.scene_bounds.center());
        let h = cam.view_proj(rect.aspect_ratio(), radius) * CosmicCut_geom::Vec4::new(p.x, p.y, p.z, 1.0);
        if h.w <= 1.0e-9 {
            return None;
        }
        let n = h.truncate() / h.w;
        let at = pos2(rect.left() + (n.x + 1.0) * 0.5 * rect.width(), rect.top() + (1.0 - n.y) * 0.5 * rect.height());
        at.is_finite().then_some(at)
    }

    /// The camera ray through screen point `p` in the table frame: (origin, unit direction).
    fn ray(&self, rect: Rect, p: Pos2) -> Option<(Vec3, Vec3)> {
        let cam = table_camera(&self.shown, self.table_b);
        let radius = self.scene_bounds.radius() + cam.target.distance(self.scene_bounds.center());
        let inv = cam.view_proj(rect.aspect_ratio(), radius).inverse();
        let x = (p.x - rect.left()) / rect.width() * 2.0 - 1.0;
        let y = 1.0 - (p.y - rect.top()) / rect.height() * 2.0;
        let unproject = |z: f32| {
            let h = inv * CosmicCut_geom::Vec4::new(x, y, z, 1.0);
            (h.w.abs() > 1.0e-9).then(|| h.truncate() / h.w)
        };
        let (near, far) = (unproject(0.0)?, unproject(1.0)?);
        let dir = (far - near).normalize_or_zero();
        (near.is_finite() && dir != Vec3::ZERO).then_some((near, dir))
    }

    /// While laying a model flat: the face under the pointer, and a click puts it down. While
    /// picking a swarf cut's faces: a click on any model adds the surface under the pointer, or
    /// takes it away.
    fn pick_face(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        rect: Rect,
        session: &Session,
        requests: &mut Vec<Request>,
    ) {
        let done = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if self.face_pick.as_ref().is_some_and(|(_, on)| *on) {
            // The nearest visible model under the pointer, and its triangle there.
            let hit = response.hover_pos().and_then(|p| self.ray(rect, p)).and_then(|(o, d)| {
                session
                    .job
                    .models
                    .iter()
                    .filter(|m| m.visible)
                    .filter_map(|m| m.mesh.raycast(o, d).map(|(t, tri)| (t, m, tri)))
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, m, tri)| (m, tri))
            });
            self.hover_triangle = hit.map(|(_, t)| t);
            if hit.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            // A click takes the whole surface the wire can follow from the face; Alt, the face alone.
            let alone = ui.input(|i| i.modifiers.alt);
            let face = hit.map(|(m, t)| (m, m.faces.face_of(t).unwrap_or(t)));
            let fresh = |h: &(String, usize, bool, Vec<usize>)| {
                face.is_some_and(|(m, f)| h.0 == m.id && h.1 == f && h.2 == alone)
            };
            if !self.hover_surface.as_ref().is_some_and(fresh) {
                self.hover_surface = face.map(|(m, f)| {
                    let faces = if alone {
                        vec![f]
                    } else {
                        CosmicCut_engine::swarf::surface(m, f, CosmicCut_engine::swarf::DEFAULT_TOLERANCE)
                    };
                    (m.id.clone(), f, alone, faces)
                });
            }
            if response.clicked()
                && hit.is_some()
                && let Some((id, _, _, faces)) = &self.hover_surface
            {
                self.picked_face = Some((id.clone(), faces.clone()));
            }
            if done || response.secondary_clicked() {
                self.stop_face_pick = true;
            }
            return;
        }
        let Some(model) = self.lay_flat.as_deref().and_then(|id| session.job.model(id)) else {
            self.lay_flat = None;
            self.hover_triangle = None;
            return;
        };
        self.hover_triangle = response
            .hover_pos()
            .and_then(|p| self.ray(rect, p))
            .and_then(|(o, d)| model.mesh.raycast(o, d))
            .map(|(_, t)| t);
        if self.hover_triangle.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked()
            && let Some(t) = self.hover_triangle
        {
            requests.push(("model.lay_flat".into(), json!({ "id": model.id, "triangle": t })));
            self.lay_flat = None;
            self.hover_triangle = None;
        }
        if done {
            self.lay_flat = None;
            self.hover_triangle = None;
        }
    }

    /// Faces drawn over everything: the one under the pointer while laying a model flat, or a
    /// swarf cut's faces on every model (and the surface under the pointer while picking them).
    fn update_highlight(&mut self, session: &Session, tokens: &Tokens) {
        let Some(gpu) = &mut self.gpu else { return };
        // (model, face, colour): picked swarf faces in their own colour, the surface under the
        // pointer in the accent.
        let mut faces: Vec<(&CosmicCut_engine::model::Model, usize, egui::Color32)> = Vec::new();
        if let Some((parts, picking)) = &self.face_pick {
            for (id, picked) in parts {
                if let Some(m) = session.job.model(id) {
                    faces.extend(picked.iter().map(|f| (m, *f, tokens.warning.gamma_multiply(0.9))));
                }
            }
            if let Some((id, _, _, hover)) =
                self.hover_surface.as_ref().filter(|_| *picking && self.hover_triangle.is_some())
                && let Some(m) = session.job.model(id)
            {
                faces.extend(hover.iter().map(|f| (m, *f, tokens.accent.gamma_multiply(0.9))));
            }
        } else if let Some(m) = self.lay_flat.as_deref().and_then(|id| session.job.model(id))
            && let Some(t) = self.hover_triangle
        {
            faces.push((m, m.faces.face_of(t).unwrap_or(t), tokens.accent.gamma_multiply(0.7)));
        }
        let key = Some(format!(
            "{:?} {}",
            faces.iter().map(|(m, f, c)| (m.id.as_str(), *f, *c)).collect::<Vec<_>>(),
            session.job.revision
        ));
        if key == self.highlight_key {
            return;
        }
        let mut g = SceneGeometry::default();
        for (m, face, colour) in &faces {
            let colour = theme::linear(*colour);
            let tris: Vec<usize> =
                if m.faces.faces.is_empty() { vec![*face] } else { m.faces.triangles_of(*face).collect() };
            for tri in tris {
                if let Some(idx) = m.mesh.triangles().get(tri) {
                    let [a, b, c] = m.mesh.corners(*idx);
                    g.push_triangle(a, b, c, colour);
                }
            }
        }
        gpu.set_highlight(&g);
        self.highlight_key = key;
    }

    fn fail(&mut self, message: &str) {
        log::error!("viewport: {message}");
        self.gpu = None;
        self.gpu_error = Some(format!("3D viewport unavailable: {message}"));
    }
}

use egui_wgpu::RenderState;

/// Size of one floating view tool button.
const TOOL_BUTTON: f32 = 38.0;
/// Gap between the tool row and the viewport's bottom edge.
const TOOL_MARGIN: f32 = 10.0;
/// Space between two tool buttons.
const TOOL_GAP: f32 = 4.0;

/// The row of view tools floating at the bottom centre of the viewport.
fn view_tools(ui: &Ui, viewport: Rect, session: &Session, tokens: &Tokens, requests: &mut Vec<Request>) {
    let tools = CosmicCut_engine::cmd::VIEWPORT_TOOLS;
    let width = tools.len() as f32 * (TOOL_BUTTON + TOOL_GAP) - TOOL_GAP;
    let left = viewport.center().x - width / 2.0;
    let centre_y = viewport.bottom() - TOOL_MARGIN - TOOL_BUTTON / 2.0;
    for (n, (id, menu)) in tools.iter().enumerate() {
        let Some(spec) = CosmicCut_engine::cmd::find(id) else {
            continue;
        };
        let button = Rect::from_center_size(
            pos2(left + TOOL_BUTTON / 2.0 + n as f32 * (TOOL_BUTTON + TOOL_GAP), centre_y),
            egui::vec2(TOOL_BUTTON, TOOL_BUTTON),
        );
        if button.left() < viewport.left() + 4.0 || button.right() > viewport.right() - 4.0 {
            continue;
        }
        let enabled = session.is_enabled(spec);
        let response = ui.interact(button, ui.id().with(("view-tool", *id)), Sense::click());
        let checked = crate::ribbon::is_checked(id, session);
        let fill = if response.is_pointer_button_down_on() {
            tokens.accent.gamma_multiply(0.3)
        } else if checked {
            tokens.accent.gamma_multiply(0.22)
        } else if response.hovered() {
            tokens.hover_bg
        } else {
            tokens.panel_bg.gamma_multiply(0.85)
        };
        let stroke = egui::Stroke::new(1.0, if response.hovered() || checked { tokens.accent } else { tokens.border });
        ui.painter().rect(button, 4.0, fill, stroke, egui::StrokeKind::Inside);
        crate::ribbon::paint_icon(ui, button.shrink(6.0), spec, tokens, enabled);
        let response = crate::ribbon::with_tooltip(response, spec, enabled);
        if response.clicked() && enabled {
            requests.push(((*id).to_owned(), Value::Null));
        }
        if !menu.is_empty() {
            crate::ui_kit::Menu::below(&response).context(tokens, |ui| {
                for item in menu.iter().filter_map(|m| CosmicCut_engine::cmd::find(m)) {
                    if ui.button(item.label).clicked() {
                        requests.push((item.id.to_owned(), Value::Null));
                    }
                }
            });
        }
    }
}

/// What to do first, over an empty job.
fn hint(ui: &mut Ui, viewport: Rect, session: &Session, tokens: &Tokens) {
    let text = if session.job.shapes.is_empty() {
        "Import a model (STL, STEP…) or a shape (DXF, SVG, airfoil .dat), then add a cut"
    } else {
        "Add a cut from the Cuts gallery: Profile, Two-view, Lathe or Slabs"
    };
    let at = Rect::from_center_size(pos2(viewport.center().x, viewport.top() + 34.0), vec2(560.0, 30.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
        Container::new(Surface::Overlay).show(ui, tokens, |ui| {
            ui.label(RichText::new(text).color(tokens.text_muted));
        });
    });
}

/// Instructions when nothing in particular is going on.
const IDLE: &str = "New sketch draws a shape on a plane; Import model or Import sketch brings one in. Then add a cut \
                    from the gallery, simulate it, and post the G-code.\nInstructions for each tool show here while \
                    you use it.";

/// Selects all of a text field's text, so what is typed replaces it.
pub(super) fn select_all(ui: &Ui, id: egui::Id, chars: usize) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
        let range = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars));
        state.cursor.set_char_range(Some(range));
        state.store(ui.ctx(), id);
    }
}

/// The camera that shows the table frame turned by `b` degrees: the user's camera turned back
/// by B about the table's axis, so the turntable appears to turn and the world stays put.
fn table_camera(cam: &OrbitCamera, b: f64) -> OrbitCamera {
    if b.abs() < 1.0e-9 {
        return *cam;
    }
    let r = (-b as f32).to_radians();
    let (s, c) = r.sin_cos();
    let t = cam.target;
    let mut out = *cam;
    out.target = Vec3::new(t.x * c - t.y * s, t.x * s + t.y * c, t.z);
    out.yaw = CosmicCut_render::camera::wrap_angle(cam.yaw + r);
    out
}

/// What to do while laying a model flat.
fn lay_flat_hint(ui: &mut Ui, viewport: Rect, tokens: &Tokens) {
    let at = Rect::from_center_size(pos2(viewport.center().x, viewport.top() + 34.0), vec2(520.0, 30.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
        Container::new(Surface::Overlay).show(ui, tokens, |ui| {
            ui.label(
                RichText::new("Click the face of the model to put down on the turntable  ·  Esc to cancel")
                    .color(tokens.accent),
            );
        });
    });
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The simulation's controls, floating above the view tools: transport buttons, the timeline,
/// the speed and where the wire is.
fn playback(ui: &mut Ui, viewport: Rect, session: &Session, tokens: &Tokens, requests: &mut Vec<Request>) {
    let Some(sim) = session.sim.as_deref() else { return };
    let width = (viewport.width() - 40.0).clamp(300.0, 760.0);
    let bottom = viewport.bottom() - TOOL_MARGIN - TOOL_BUTTON - 10.0;
    let at = Rect::from_min_max(
        pos2(viewport.center().x - width / 2.0, bottom - if sim.notice.is_some() { 90.0 } else { 64.0 }),
        pos2(viewport.center().x + width / 2.0, bottom),
    );
    let units = session.prefs.units;
    let m = &session.job.machine;
    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
        Container::new(Surface::Overlay).show(ui, tokens, |ui| {
            ui.set_width(width - 20.0);
            ui.horizontal(|ui| {
                let button = |ui: &mut Ui, id: &str, size: f32| {
                    let Some(spec) = CosmicCut_engine::cmd::find(id) else { return false };
                    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 3.0, tokens.hover_bg);
                    }
                    crate::ribbon::paint_icon(ui, rect.shrink(2.0), spec, tokens, true);
                    crate::ribbon::with_tooltip(response, spec, true).clicked()
                };
                for id in ["sim.jump.start", "sim.jump.previous"] {
                    if button(ui, id, 22.0) {
                        requests.push((id.into(), Value::Null));
                    }
                }
                let play = if sim.playing { "sim.pause" } else { "sim.play" };
                if button(ui, play, 26.0) {
                    requests.push((play.into(), Value::Null));
                }
                for id in ["sim.jump.next", "sim.jump.end"] {
                    if button(ui, id, 22.0) {
                        requests.push((id.into(), Value::Null));
                    }
                }
                ui.add_space(6.0);
                let mut t = sim.time;
                let duration = sim.duration().max(1.0e-6);
                ui.spacing_mut().slider_width = (ui.available_width() - 190.0).max(80.0);
                let slider = ui.add(egui::Slider::new(&mut t, 0.0..=duration).show_value(false));
                if slider.changed() {
                    requests.push(("sim.seek".into(), json!({ "time": t })));
                }
                ui.label(RichText::new(format!("{} / {}", clock(sim.time), clock(duration))).monospace());
                if ui
                    .button(format!("×{}", trim(sim.speed)))
                    .on_hover_text("Playback speed: click for faster")
                    .clicked()
                {
                    requests.push(("sim.speed".into(), Value::Null));
                }
                if ui.button("✕").on_hover_text("Close the simulation").clicked() {
                    requests.push(("sim.stop".into(), Value::Null));
                }
            });
            // The machine waits here for the operator.
            if let Some(notice) = &sim.notice {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⏸ Machine stop").strong().color(tokens.warning));
                    ui.label(RichText::new(notice).color(tokens.text));
                    ui.label(RichText::new("· press Play to carry on").color(tokens.text_muted));
                });
            }
            let w: WirePos = sim.wire();
            let (x, y) = m.machine_xy(w.u, w.h);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("X {}   Y {}   B {:.2}°", units.plain(x), units.plain(y), m.machine_b(w.b)))
                        .monospace()
                        .color(tokens.wire),
                );
                ui.label(
                    RichText::new(format!("  ·  {}", sim.current_cut().unwrap_or("moving between cuts")))
                        .color(tokens.text_muted),
                );
                if let Some(v) = &sim.voxels {
                    let left = v.volume() / session.job.stock.volume().max(1.0) * 100.0;
                    ui.label(RichText::new(format!("  ·  {left:.0}% of the foam left")).color(tokens.text_muted));
                }
            });
        });
    });
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A swarf cut's faces by model (id, face numbers), and whether more are being picked.
pub type FacePick = (Vec<(String, Vec<usize>)>, bool);

/// A profile's lines while picking: the table angle, each line (id, points in the cut plane,
/// left out), and whether picking.
pub type LinePick = (f64, Vec<(u32, Vec<glam::DVec2>, bool)>, bool);

/// Distance from `p` to the segment `a`–`b`, points.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 1.0e-6 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

#[cfg(test)]
mod select_all_tests {
    use super::*;

    /// A field opened for a new size has its text selected: typing replaces it.
    #[test]
    fn typing_into_an_opened_field_replaces_its_text() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("size");
        let mut text = "12.5".to_owned();
        let frame = |events: Vec<egui::Event>, text: &mut String| {
            let input = egui::RawInput { events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let field = ui.add(egui::TextEdit::singleline(text).id(id));
                if !field.has_focus() && !field.lost_focus() {
                    field.request_focus();
                    select_all(ui, field.id, text.chars().count());
                }
            });
            output.textures_delta.clear();
        };
        frame(Vec::new(), &mut text);
        frame(Vec::new(), &mut text);
        frame(vec![egui::Event::Text("7".into())], &mut text);
        assert_eq!(text, "7");
    }
}
