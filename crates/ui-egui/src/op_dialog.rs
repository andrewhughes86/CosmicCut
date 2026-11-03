//! The operation settings dialog: what the operation is for, the machine stop it starts with,
//! the message the controller shows the operator, and notes written into the program.

use CosmicCut_engine::Session;
use CosmicCut_engine::op::Stop;
use egui::{RichText, vec2};
use serde_json::json;

use crate::Request;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Surface, Window};

pub struct OpDialog {
    id: String,
    /// Its place in the job, for the title.
    n: usize,
    name: String,
    stop: Stop,
    message: String,
    notes: String,
    /// Models off the table before this operation, and every model to choose from (id, name).
    ignore: Vec<String>,
    models: Vec<(String, String)>,
}

impl OpDialog {
    /// The dialog for operation `id`, if there is one.
    pub fn new(session: &Session, id: &str) -> Option<Self> {
        let n = session.job.op_index(id)?;
        let op = session.job.ops.get(n)?;
        Some(Self {
            id: op.id.clone(),
            n,
            name: op.name.clone(),
            stop: op.stop,
            message: op.message.clone(),
            notes: op.notes.clone(),
            ignore: op.ignore.clone(),
            models: session.job.models.iter().map(|m| (m.id.clone(), m.name.clone())).collect(),
        })
    }

    /// Draws the dialog. Returns the command to run on OK, and whether to stay open.
    pub fn show(&mut self, ctx: &egui::Context, tokens: &Tokens) -> (Option<Request>, bool) {
        let mut open = true;
        let mut done: Option<bool> = None;
        let title = format!("Op {} settings", self.n + 1);
        Window::dialog(title).show(ctx, tokens, &mut open, |ui| {
            ui.set_width(520.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                RichText::new(
                    "An operation is a group of cuts that starts with the machine waiting for the operator, for \
                     example to turn the stock by hand. The wire is switched off while the machine waits.",
                )
                .color(tokens.text_muted),
            );
            ui.add_space(4.0);
            Container::new(Surface::Card).show(ui, tokens, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Name").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .hint_text("e.g. Turn the stock a quarter")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(6.0);
                ui.label(RichText::new("Machine stop").strong());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for stop in [Stop::None, Stop::Pause, Stop::Optional] {
                        ui.selectable_value(&mut self.stop, stop, stop.label());
                    }
                });
                ui.label(
                    RichText::new(match self.stop {
                        Stop::None => "The machine carries straight on into this operation's cuts.",
                        Stop::Pause => "M0: the machine stops until the operator presses Resume.",
                        Stop::Optional => {
                            "M1: the machine stops only when the controller's optional stop is switched on."
                        }
                    })
                    .size(11.5)
                    .color(tokens.text_muted),
                );
            });
            ui.add_space(6.0);
            Container::new(Surface::Card).show(ui, tokens, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Message to the operator").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.message)
                        .hint_text("e.g. Turn the stock 90° clockwise, then Resume")
                        .desired_width(f32::INFINITY),
                );
                ui.label(
                    RichText::new(
                        "Shown by the controller when the operation starts, as (MSG, …); it pauses there too.",
                    )
                    .size(11.5)
                    .color(tokens.text_muted),
                );
                ui.add_space(6.0);
                ui.label(RichText::new("Notes").strong());
                ui.add(egui::TextEdit::multiline(&mut self.notes).desired_rows(4).desired_width(f32::INFINITY));
                ui.label(
                    RichText::new("Written into the program as comments, and shown in the inspector.")
                        .size(11.5)
                        .color(tokens.text_muted),
                );
            });
            if !self.models.is_empty() {
                ui.add_space(6.0);
                Container::new(Surface::Card).show(ui, tokens, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Models already off the table").strong());
                    for (id, name) in &self.models {
                        let mut on = self.ignore.contains(id);
                        if ui.checkbox(&mut on, name.as_str()).changed() {
                            if on {
                                self.ignore.push(id.clone());
                            } else {
                                self.ignore.retain(|i| i != id);
                            }
                        }
                    }
                    ui.label(
                        RichText::new(
                            "Parts cut and taken off before this operation starts: the wire of its cuts may pass \
                             through where they were. Otherwise a cut fails if the wire would cut a model it is not \
                             making.",
                        )
                        .size(11.5)
                        .color(tokens.text_muted),
                    );
                });
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = egui::Button::new(RichText::new("Save").strong().color(tokens.on_accent))
                    .fill(tokens.accent)
                    .min_size(vec2(100.0, 28.0));
                if ui.add(ok).clicked() {
                    done = Some(true);
                }
                if ui.add(egui::Button::new("Cancel").min_size(vec2(80.0, 28.0))).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => (
                Some((
                    "op.edit".into(),
                    json!({
                        "id": self.id, "name": self.name, "stop": self.stop.id(),
                        "message": self.message, "notes": self.notes, "ignore": self.ignore,
                    }),
                )),
                false,
            ),
            Some(false) => (None, false),
            None => (None, open),
        }
    }
}
