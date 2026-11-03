//! The start page: what CosmicCut opens on. New project, Open, the imports, the projects opened
//! or saved lately, and the sample. Every button is a request for a command, as elsewhere.

use egui::{Align, Layout, Rect, RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::Request;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Panel, Surface};

/// Most recent projects remembered.
pub const MAX_RECENT: usize = 8;

/// Puts `path` first among the recent projects. Returns whether the list changed.
pub fn remember(recent: &mut Vec<String>, path: &str) -> bool {
    if recent.first().is_some_and(|p| p == path) || path.trim().is_empty() {
        return false;
    }
    recent.retain(|p| p != path);
    recent.insert(0, path.to_owned());
    recent.truncate(MAX_RECENT);
    true
}

#[derive(Default)]
pub struct Home {
    logo: Option<egui::TextureHandle>,
}

impl Home {
    /// Draws the page. `back` names the job to go back to, when one is open behind it.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        tokens: &Tokens,
        recent: &[String],
        back: Option<&str>,
        requests: &mut Vec<Request>,
    ) {
        let logo = self
            .logo
            .get_or_insert_with(|| {
                let image = crate::app_icon(128)
                    .map(|i| egui::ColorImage::from_rgba_unmultiplied([i.width as usize, i.height as usize], &i.rgba))
                    .unwrap_or_else(|| egui::ColorImage::filled([1, 1], tokens.accent));
                ui.ctx().load_texture("cosmiccut-logo", image, egui::TextureOptions::LINEAR)
            })
            .clone();
        Panel::central(Surface::Workspace).show(ui, tokens, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let width = ui.available_width().min(1040.0);
                let margin = ((ui.available_width() - width) / 2.0).max(16.0);
                ui.add_space(36.0);
                ui.horizontal(|ui| {
                    ui.add_space(margin);
                    ui.vertical(|ui| {
                        ui.set_width(width - 32.0);
                        header(ui, tokens, &logo, back, requests);
                        ui.add_space(28.0);
                        if ui.available_width() > 720.0 {
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(260.0);
                                    start(ui, tokens, requests);
                                });
                                ui.add_space(28.0);
                                ui.vertical(|ui| {
                                    projects(ui, tokens, recent, requests);
                                });
                            });
                        } else {
                            start(ui, tokens, requests);
                            ui.add_space(20.0);
                            projects(ui, tokens, recent, requests);
                        }
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new(
                                "Drop a CosmicCut job, a model or a drawing anywhere on the window to open it.",
                            )
                            .color(tokens.text_muted),
                        );
                    });
                });
                ui.add_space(24.0);
            });
        });
    }
}

fn header(ui: &mut Ui, tokens: &Tokens, logo: &egui::TextureHandle, back: Option<&str>, requests: &mut Vec<Request>) {
    ui.horizontal(|ui| {
        ui.add(egui::Image::new((logo.id(), vec2(64.0, 64.0))));
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("CosmicCut").size(30.0).strong().color(tokens.text));
            ui.label(
                RichText::new(format!("CAM for XYB hot-wire foam cutters  ·  version {}", env!("CARGO_PKG_VERSION")))
                    .color(tokens.text_muted),
            );
        });
        if let Some(job) = back {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = egui::Button::new(RichText::new(format!("Back to {job}")).color(tokens.text))
                    .min_size(vec2(0.0, 32.0));
                if ui.add(button).clicked() {
                    requests.push(("ui.home.close".into(), Value::Null));
                }
            });
        }
    });
}

/// New project, Open and the imports, as large buttons down the left.
fn start(ui: &mut Ui, tokens: &Tokens, requests: &mut Vec<Request>) {
    heading(ui, tokens, "Start");
    let items: [(&str, &str, &str, &str); 4] = [
        ("job_new", "New project", "An empty job on your machine", "ui.home.new"),
        ("job_open", "Open…", "A CosmicCut job (.ccut)", "job.open"),
        ("model_import", "Import model…", "STL, STEP, OBJ, 3MF and more", "model.import"),
        ("shape_import", "Import sketch…", "DXF, SVG, airfoil .dat, XY points", "shape.import"),
    ];
    for (icon, title, sub, command) in items {
        if tile(ui, tokens, icon, title, sub, 260.0).clicked() {
            requests.push((command.into(), Value::Null));
        }
        ui.add_space(6.0);
    }
}

/// The recent projects, then the sample.
fn projects(ui: &mut Ui, tokens: &Tokens, recent: &[String], requests: &mut Vec<Request>) {
    heading(ui, tokens, "Recent projects");
    // In a browser there are no files to go back to.
    let shown: Vec<&String> = if cfg!(target_arch = "wasm32") {
        Vec::new()
    } else {
        recent.iter().filter(|p| std::path::Path::new(p.as_str()).is_file()).collect()
    };
    if shown.is_empty() {
        Container::new(Surface::Card).show(ui, tokens, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(if cfg!(target_arch = "wasm32") {
                    "In the browser, open a saved job with Open…"
                } else {
                    "Jobs you open or save show here."
                })
                .color(tokens.text_muted),
            );
        });
    }
    let w = ui.available_width();
    for path in shown {
        let p = std::path::Path::new(path.as_str());
        let name = p.file_stem().map_or_else(|| path.clone(), |s| s.to_string_lossy().into_owned());
        let folder = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
        if tile(ui, tokens, "job_open", &name, &folder, w).on_hover_text(path.as_str()).clicked() {
            requests.push(("job.open".into(), json!({ "path": path })));
        }
        ui.add_space(4.0);
    }
    ui.add_space(18.0);
    heading(ui, tokens, "Sample");
    if tile(
        ui,
        tokens,
        "job_sample",
        "Sample penguin",
        "A low-poly penguin cut from a foam block, a profile every 22.5°, then simulated",
        w,
    )
    .clicked()
    {
        requests.push(("job.sample".into(), Value::Null));
    }
}

fn heading(ui: &mut Ui, tokens: &Tokens, text: &str) {
    ui.label(RichText::new(text).size(17.0).strong().color(tokens.text));
    ui.add_space(8.0);
}

/// A clickable row: an icon, a title and a line under it.
fn tile(ui: &mut Ui, tokens: &Tokens, icon: &str, title: &str, sub: &str, width: f32) -> egui::Response {
    let height = 56.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter_at(rect);
    let hovered = response.hovered();
    painter.rect(
        rect,
        6.0,
        if hovered { tokens.hover_bg } else { tokens.raised_bg },
        egui::Stroke::new(1.0, if hovered { tokens.accent } else { tokens.border }),
        egui::StrokeKind::Inside,
    );
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(30.0, 0.0), vec2(32.0, 32.0));
    crate::ribbon::paint_named_icon(ui, icon_rect, icon, tokens, true);
    let x = icon_rect.right() + 14.0;
    let room = (rect.right() - x - 10.0).max(20.0);
    let fit = |text: &str, size: f32, colour| {
        let galley = painter.layout(text.to_owned(), egui::FontId::proportional(size), colour, room);
        let row = galley.rows.first().map(|r| r.text().to_owned()).unwrap_or_default();
        let clipped = if galley.rows.len() > 1 { format!("{}…", row.trim_end()) } else { row };
        painter.layout_no_wrap(clipped, egui::FontId::proportional(size), colour)
    };
    let top = fit(title, 15.0, tokens.text);
    let under = fit(sub, 12.5, tokens.text_muted);
    let y = rect.center().y - (top.size().y + under.size().y + 2.0) / 2.0;
    painter.galley(egui::pos2(x, y), top.clone(), tokens.text);
    painter.galley(egui::pos2(x, y + top.size().y + 2.0), under, tokens.text_muted);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn recent_projects_go_first_once_and_are_capped() {
        let mut r = Vec::new();
        assert!(super::remember(&mut r, "/a.ccut"));
        assert!(super::remember(&mut r, "/b.ccut"));
        assert!(!super::remember(&mut r, "/b.ccut"));
        assert!(super::remember(&mut r, "/a.ccut"));
        assert_eq!(r, vec!["/a.ccut".to_owned(), "/b.ccut".to_owned()]);
        for k in 0..20 {
            super::remember(&mut r, &format!("/{k}.ccut"));
        }
        assert_eq!(r.len(), super::MAX_RECENT);
    }
}
