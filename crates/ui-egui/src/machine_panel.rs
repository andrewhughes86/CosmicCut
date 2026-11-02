//! The machine editor: the Machine tab, beside the cut plane and the G-code. The job's machine
//! in cards (the machine and its post, travel, the turntable, the wire); Save runs
//! `machine.edit`, which changes the library's copy too.

use CosmicCut_engine::Session;
use egui::{RichText, Ui, vec2};

use crate::Request;
use crate::param_form::ParamForm;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Surface};

/// The cards, what each is for, and the parameters in it; anything else goes under Wire.
const CARDS: &[(&str, &str, &[&str])] = &[
    ("Machine", "Its name in the library, and the post processor that writes its programs.", &["name", "post"]),
    ("Travel", "How far the wire moves, and where it is at Y0.", &["travel_x", "travel_y", "wire_y0", "wire_span"]),
    (
        "Turntable",
        "Where the turntable's axis is along X, its size, and how it turns.",
        &["table_x", "table_radius", "b_axis", "b_reversed", "b_rate"],
    ),
];

pub struct MachinePanel {
    form: ParamForm,
}

impl MachinePanel {
    /// The panel on the job's machine; None if there is no `machine.edit` command.
    pub fn new(session: &Session) -> Option<Self> {
        let spec = CosmicCut_engine::cmd::find("machine.edit")?;
        Some(Self { form: ParamForm::new(spec, session, &serde_json::Value::Null) })
    }

    /// The command failed: keep the panel open and say why.
    pub fn fail(&mut self, message: String) {
        self.form.fail(message);
    }

    /// Draws the tab. Returns the command to run when Save is pressed, and whether to stay open.
    pub fn show(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens) -> (Option<Request>, bool) {
        let mut keep = true;
        let mut save = false;
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
            crate::ribbon::paint_named_icon(ui, rect, "machine_edit", tokens, true);
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("Machine: {}", session.job.machine.name)).strong().size(18.0));
                ui.add(
                    egui::Label::new(
                        RichText::new("This job's machine. Saving changes the machine library's copy too.")
                            .color(tokens.text_muted),
                    )
                    .wrap(),
                );
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("machine-panel")
            .max_height((ui.available_height() - 90.0).max(160.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (title, intro, names) in CARDS {
                    card(ui, tokens, title, intro, |ui| self.form.fields_ui(ui, tokens, title, |n| names.contains(&n)));
                }
                card(
                    ui,
                    tokens,
                    "Wire",
                    "Rates, the kerf the wire burns, and how far it keeps clear of the foam.",
                    |ui| {
                        self.form
                            .fields_ui(ui, tokens, "wire", |n| !CARDS.iter().any(|(_, _, names)| names.contains(&n)));
                    },
                );
            });
        if let Some(e) = self.form.error() {
            ui.add(egui::Label::new(RichText::new(e).color(tokens.error)).wrap());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let primary = egui::Button::new(RichText::new("Save machine").strong().color(tokens.on_accent))
                .fill(tokens.accent)
                .min_size(vec2(120.0, 30.0));
            save = ui.add(primary).clicked();
            if ui.add(egui::Button::new("Cancel").min_size(vec2(90.0, 30.0))).clicked() {
                keep = false;
            }
        });
        if save {
            return (self.form.request(), true);
        }
        (None, keep)
    }
}

pub fn card(ui: &mut Ui, tokens: &Tokens, title: &str, intro: &str, add: impl FnOnce(&mut Ui)) {
    Container::new(Surface::Card).show(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0).color(tokens.text));
        ui.add(egui::Label::new(RichText::new(intro).color(tokens.text_muted)).wrap());
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(8.0);
}
