//! The machine library and the post processor library: the operator's machines and post
//! processors, kept between jobs. A list on the left, the chosen entry on the right, and what
//! can be done with it. Everything goes through commands.

use CosmicCut_engine::Session;
use egui::{RichText, Ui, vec2};
use serde_json::{Value, json};

use crate::Request;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Surface, Window};

/// The window's width, and the chosen entry's card in it (beside the 200 px list).
const WIDTH: f32 = 640.0;
const DETAILS_WIDTH: f32 = WIDTH - 200.0 - 24.0 - 8.0 - 2.0 * 2.0 * 14.0 - 4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Machines,
    Posts,
}

pub struct LibraryDialog {
    pub kind: Kind,
    /// The entry chosen in the list, by name.
    chosen: Option<String>,
}

impl LibraryDialog {
    pub fn new(session: &Session, kind: Kind) -> Self {
        // Start on what the job uses.
        let chosen = Some(match kind {
            Kind::Machines => session.job.machine.name.clone(),
            Kind::Posts => session.job.machine.post_name.clone(),
        });
        Self { kind, chosen }
    }

    /// Draws the window; returns whether it stays open. Commands go into `requests`.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        session: &Session,
        tokens: &Tokens,
        requests: &mut Vec<Request>,
    ) -> bool {
        let mut open = true;
        let title = match self.kind {
            Kind::Machines => "Machine library",
            Kind::Posts => "Post processor library",
        };
        let names: Vec<String> = match self.kind {
            Kind::Machines => session.library.machines.iter().map(|m| m.name.clone()).collect(),
            Kind::Posts => session.library.posts.iter().map(|p| p.name.clone()).collect(),
        };
        if self.chosen.as_ref().is_none_or(|c| !names.contains(c)) {
            self.chosen = names.first().cloned();
        }
        let in_use = match self.kind {
            Kind::Machines => session.job.machine.name.clone(),
            Kind::Posts => session.job.machine.post_name.clone(),
        };
        Window::browser(title, vec2(WIDTH, 520.0)).show(ctx, tokens, &mut open, |ui| {
            ui.set_max_width(WIDTH - 24.0);
            ui.add(
                egui::Label::new(
                    RichText::new(match self.kind {
                        Kind::Machines => {
                            "Your machines, kept between jobs. Choose the one this job runs on; Machine on the ribbon \
                         changes its settings, and the library keeps them."
                        }
                        Kind::Posts => {
                            "Post processors write the program for the controller. Each machine uses one; changing a \
                         post changes it for every machine that uses it."
                        }
                    })
                    .color(tokens.text_muted),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                // The list.
                Container::new(Surface::Card).show(ui, tokens, |ui| {
                    ui.set_width(200.0);
                    ui.vertical(|ui| {
                        ui.set_min_height(380.0);
                        egui::ScrollArea::vertical().id_salt(("library-list", title)).show(ui, |ui| {
                            for name in &names {
                                let label = if *name == in_use {
                                    RichText::new(format!("{name}   ✔ in use")).strong()
                                } else {
                                    RichText::new(name)
                                };
                                if ui.selectable_label(self.chosen.as_deref() == Some(name), label).clicked() {
                                    self.chosen = Some(name.clone());
                                }
                            }
                        });
                        ui.add_space(8.0);
                        let new = match self.kind {
                            Kind::Machines => ("machine.library.new", "New machine"),
                            Kind::Posts => ("post.library.new", "New post (a copy)"),
                        };
                        if ui.button(new.1).clicked() {
                            let params = match (self.kind, &self.chosen) {
                                (Kind::Posts, Some(from)) => json!({ "from": from }),
                                _ => Value::Null,
                            };
                            requests.push((new.0.into(), params));
                        }
                    });
                });
                ui.add_space(8.0);
                // The chosen entry.
                Container::new(Surface::Card).show(ui, tokens, |ui| {
                    ui.vertical(|ui| {
                        // Sized from the window's width: on the first frame there is no window yet,
                        // and the room available would be the whole screen.
                        ui.set_width(DETAILS_WIDTH);
                        ui.set_min_height(380.0);
                        match self.chosen.clone() {
                            Some(name) => self.details(ui, session, tokens, &name, name == in_use, requests),
                            None => {
                                ui.label("The library is empty.");
                            }
                        }
                    });
                });
            });
        });
        open
    }

    fn details(
        &self,
        ui: &mut Ui,
        session: &Session,
        tokens: &Tokens,
        name: &str,
        in_use: bool,
        requests: &mut Vec<Request>,
    ) {
        let units = session.prefs.units;
        let rows: Vec<(String, String)> = match self.kind {
            Kind::Machines => session.library.machine(name).map(|m| m.properties(units)).unwrap_or_default(),
            Kind::Posts => session
                .library
                .post(name)
                .map(|p| {
                    let s = &p.settings;
                    let users: Vec<&str> = session
                        .library
                        .machines
                        .iter()
                        .filter(|m| m.post_name == name)
                        .map(|m| m.name.as_str())
                        .collect();
                    vec![
                        ("Units".into(), s.units.label().into()),
                        ("Start block".into(), s.header.replace('\n', "  ·  ")),
                        ("Wire on".into(), format!("{}   (preheat {} s)", s.wire_on.replace('\n', "  ·  "), s.preheat)),
                        ("Wire off".into(), s.wire_off.replace('\n', "  ·  ")),
                        ("End code".into(), s.footer.replace('\n', "  ·  ")),
                        ("G word on every line".into(), if s.motion_every_line { "yes" } else { "no" }.into()),
                        ("Arcs (G2/G3)".into(), if s.arcs { "yes" } else { "no" }.into()),
                        ("Line numbers".into(), if s.line_numbers { "yes" } else { "no" }.into()),
                        ("File extension".into(), format!(".{}", s.extension)),
                        ("Used by".into(), if users.is_empty() { "no machine".into() } else { users.join(", ") }),
                    ]
                })
                .unwrap_or_default(),
        };
        // What it is above, what can be done with it along the bottom.
        let value_width = DETAILS_WIDTH - 140.0;
        ui.vertical(|ui| {
            ui.set_min_height(318.0);
            ui.label(RichText::new(name).strong().size(17.0));
            ui.add_space(4.0);
            egui::Grid::new(("library-details", name))
                .num_columns(2)
                .striped(true)
                .max_col_width(value_width)
                .spacing([14.0, 5.0])
                .show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).color(tokens.text_muted));
                        ui.add(egui::Label::new(v).wrap());
                        ui.end_row();
                    }
                });
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let primary = |text: &str| {
                egui::Button::new(RichText::new(text).strong().color(tokens.on_accent))
                    .fill(tokens.accent)
                    .min_size(vec2(120.0, 28.0))
            };
            match self.kind {
                Kind::Machines => {
                    if in_use {
                        if ui.add(primary("Edit settings…")).clicked() {
                            requests.push(("machine.edit".into(), Value::Null));
                        }
                    } else if ui.add(primary("Use for this job")).clicked() {
                        requests.push(("machine.library.use".into(), json!({ "id": name })));
                    }
                    if ui.button("Copy").clicked() {
                        requests.push(("machine.library.duplicate".into(), json!({ "id": name })));
                    }
                    if ui.button("Rename…").clicked() {
                        requests.push((
                            "ui.rename".into(),
                            json!({ "command": "machine.library.rename", "id": name, "name": name }),
                        ));
                    }
                    let can_delete = session.library.machines.len() > 1;
                    if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked() {
                        requests.push((
                            "ui.delete".into(),
                            json!({ "command": "machine.library.remove", "id": name, "label": format!("the machine {name}") }),
                        ));
                    }
                }
                Kind::Posts => {
                    if ui.add(primary("Edit…")).clicked() {
                        requests.push(("ui.post.edit".into(), json!({ "name": name })));
                    }
                    if !in_use && ui.button("Use for this machine").clicked() {
                        requests.push(("post.library.use".into(), json!({ "id": name })));
                    }
                    if ui.button("Rename…").clicked() {
                        requests.push((
                            "ui.rename".into(),
                            json!({ "command": "post.library.rename", "id": name, "name": name }),
                        ));
                    }
                    let can_delete = session.library.posts.len() > 1;
                    if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked() {
                        requests.push((
                            "ui.delete".into(),
                            json!({ "command": "post.library.remove", "id": name, "label": format!("the post processor {name}") }),
                        ));
                    }
                }
            }
        });
    }
}
