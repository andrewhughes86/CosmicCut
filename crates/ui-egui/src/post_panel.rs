//! The post processor editor: the Post processor tab, beside the cut plane and the G-code. How
//! one post processor in the library writes programs; Save runs `post.settings` with the
//! settings, which changes the post for every machine that uses it.

use CosmicCut_engine::Session;
use CosmicCut_engine::post::PostSettings;
use CosmicCut_engine::units::DisplayUnits;
use egui::{RichText, Ui, vec2};

use crate::Request;
use crate::machine_panel::card;
use crate::theme::Tokens;

pub struct PostPanel {
    /// The library post being changed.
    name: String,
    edit: PostSettings,
    /// Decimals typed (blank for the units' default).
    decimals: String,
    error: Option<String>,
}

impl PostPanel {
    /// The panel for library post `name` (default the job machine's).
    pub fn new(session: &Session, name: Option<&str>) -> Self {
        let name = name.map_or_else(|| session.job.machine.post_name.clone(), str::to_owned);
        let edit =
            session.library.post(&name).map(|p| p.settings.clone()).unwrap_or_else(|| session.job.machine.post.clone());
        let decimals = edit.decimals.map(|d| d.to_string()).unwrap_or_default();
        Self { name, edit, decimals, error: None }
    }

    /// The command failed: keep the panel open and say why.
    pub fn fail(&mut self, message: String) {
        self.error = Some(message);
    }

    /// Draws the tab. Returns the command to run when Save is pressed, and whether to stay open.
    pub fn show(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) -> (Option<Request>, bool) {
        self.decimals_into_edit();
        let mut keep = true;
        let mut save = false;
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            crate::ribbon::paint_named_icon(ui, rect, "post_settings", tokens, true);
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("Post processor: {}", self.name)).strong().size(18.0));
                let users: Vec<&str> = session
                    .library
                    .machines
                    .iter()
                    .filter(|m| m.post_name == self.name)
                    .map(|m| m.name.as_str())
                    .collect();
                let used = if users.is_empty() { "no machine yet".to_owned() } else { users.join(", ") };
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "How programs are written for the controller. Saving changes it for every machine that \
                             uses it: {used}."
                        ))
                        .color(tokens.text_muted),
                    )
                    .wrap(),
                );
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("post-panel")
            .max_height((ui.available_height() - 90.0).max(160.0))
            .auto_shrink([false, true])
            .show(ui, |ui| self.settings(ui, tokens));
        if let Some(e) = &self.error {
            ui.add(egui::Label::new(RichText::new(e).color(tokens.error)).wrap());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let primary = egui::Button::new(RichText::new("Save post").strong().color(tokens.on_accent))
                .fill(tokens.accent)
                .min_size(vec2(120.0, 30.0));
            save = ui.add(primary).clicked();
            if ui.add(egui::Button::new("Cancel").min_size(vec2(90.0, 30.0))).clicked() {
                keep = false;
            }
            ui.add_space(12.0);
            if ui.button("Defaults").on_hover_text("The settings CosmicCut starts with").clicked() {
                self.edit = PostSettings::default();
                self.decimals.clear();
            }
        });
        if save {
            let mut settings = self.edit.to_json();
            settings["name"] = serde_json::json!(self.name);
            return (Some(("post.settings".into(), settings)), true);
        }
        (None, keep)
    }

    fn decimals_into_edit(&mut self) {
        self.edit.decimals = self.decimals.trim().parse::<u32>().ok().map(|d| d.min(8));
    }

    fn settings(&mut self, ui: &mut Ui, tokens: &Tokens) {
        let e = &mut self.edit;
        let note = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).small().color(tokens.text_muted));
        };
        let code = |ui: &mut Ui, text: &mut String, rows: usize| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY),
            );
        };
        card(ui, tokens, "Program", "Units, number format and what is written besides the moves.", |ui| {
            egui::Grid::new("post-program").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Units");
                egui::ComboBox::from_id_salt("post-units").selected_text(e.units.label()).show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.units, DisplayUnits::Inches, "Inches (G20)");
                    ui.selectable_value(&mut e.units, DisplayUnits::Millimetres, "Millimetres (G21)");
                });
                ui.end_row();
                ui.label("Decimals");
                ui.add(egui::TextEdit::singleline(&mut self.decimals).hint_text("default").desired_width(60.0))
                    .on_hover_text("Decimal places of X and Y; blank for 4 in inches, 3 in millimetres");
                ui.end_row();
                ui.label("Extension");
                ui.add(egui::TextEdit::singleline(&mut e.extension).desired_width(60.0));
                ui.end_row();
                ui.label("Comments");
                ui.checkbox(&mut e.comments, "Name the job, the stock and each cut");
                ui.end_row();
                ui.label("% wrappers");
                ui.checkbox(&mut e.percent, "% at the start and end");
                ui.end_row();
                ui.label("Line numbers");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut e.line_numbers, "N");
                    ui.add_enabled(e.line_numbers, egui::DragValue::new(&mut e.line_start).prefix("from "));
                    ui.add_enabled(
                        e.line_numbers,
                        egui::DragValue::new(&mut e.line_step).range(1..=1000).prefix("step "),
                    );
                });
                ui.end_row();
            });
        });
        card(ui, tokens, "Moves", "How each move line is written.", |ui| {
            ui.checkbox(&mut e.motion_every_line, "G0 or G1 on every move line");
            ui.checkbox(&mut e.feed_every_line, "Feed (F) on every feed move");
            ui.checkbox(&mut e.arcs, "G2/G3 arcs where the path follows circles");
            ui.horizontal(|ui| {
                ui.label("Turntable axis letter");
                ui.add(egui::TextEdit::singleline(&mut e.b_letter).desired_width(30.0));
            });
        });
        card(ui, tokens, "Wire", "Switching the wire on and off, and what happens between cuts.", |ui| {
            ui.label("Wire on");
            code(ui, &mut e.wire_on, 2);
            note(ui, "Written before the first cut, e.g. M3, M3 S60 or M64 P0");
            ui.horizontal(|ui| {
                ui.label("Preheat");
                ui.add(egui::DragValue::new(&mut e.preheat).range(0.0..=600.0).speed(0.1).suffix(" s"));
                note(ui, "a G4 dwell after the wire comes on");
            });
            ui.label("Wire off");
            code(ui, &mut e.wire_off, 2);
            ui.checkbox(&mut e.wire_off_between_cuts, "Wire off while travelling between cuts");
            ui.label("Between cuts");
            code(ui, &mut e.between_cuts, 2);
            note(ui, "Written between cuts with the wire above the foam, e.g. M0 to pause");
        });
        card(ui, tokens, "Start and end", "The code before the first move, the trip home, and the code after.", |ui| {
            ui.label("Start (safety) block");
            code(ui, &mut e.header, 4);
            note(ui, "{units} becomes G20 or G21, {job} the job's name, {preheat} the preheat seconds");
            ui.checkbox(&mut e.square_b, "Turn the table to B0 or B180, whichever is nearer, after the last cut");
            note(
                ui,
                "With the wire off and clear of the foam (out past where it swings, or above it), before going home",
            );
            ui.checkbox(&mut e.return_home, "Return home to X0 Y0 at the end");
            note(
                ui,
                "With the wire off: up to the top of the travel, across to X0 and down when the part is between the \
                 wire and home, else straight home",
            );
            ui.label("End code");
            code(ui, &mut e.footer, 3);
            note(ui, "Written after the wire is switched off");
        });
    }
}
