//! The stock editor: the Stock tab, beside the cut plane and the G-code. A foam block (width,
//! depth, height) or a round billet (diameter, height), and where it stands on the table. Each
//! value applies as soon as it is entered (Enter, or leaving the field), through `stock.block`
//! or `stock.cylinder`, so the viewport's dimensions, the tab and Undo always agree.

use CosmicCut_engine::Session;
use CosmicCut_engine::stock::{Stock, StockShape};
use CosmicCut_engine::units::DisplayUnits;
use egui::{RichText, Ui, vec2};
use serde_json::{Map, Value, json};

use crate::Request;
use crate::theme::Tokens;
use crate::ui_kit::{Container, Surface};

/// The values the tab edits, by their command parameter names.
const KEYS: [&str; 8] = ["width", "depth", "height", "diameter", "x", "y", "base", "rotation"];

pub struct StockPanel {
    round: bool,
    /// Each value as typed, in display units (degrees for the turn), in `KEYS` order.
    text: [String; 8],
    /// What the values were filled with, to tell an edit from a value left alone.
    filled: [String; 8],
    /// The stock and units the values were filled from.
    seen: Option<(Stock, DisplayUnits)>,
}

impl StockPanel {
    pub fn new(session: &Session) -> Self {
        let mut panel = Self { round: false, text: Default::default(), filled: Default::default(), seen: None };
        panel.fill(session);
        panel
    }

    /// Fills the values from the job's stock, when it changed (the viewport, Undo, another tab).
    fn fill(&mut self, session: &Session) {
        let stock = session.job.stock;
        let units = session.prefs.units;
        if self.seen == Some((stock, units)) {
            return;
        }
        self.seen = Some((stock, units));
        let (w, d, h, dia) = match stock.shape {
            StockShape::Block { size } => (size.x, size.y, size.z, size.x.max(size.y)),
            StockShape::Cylinder { radius, height } => (radius * 2.0, radius * 2.0, height, radius * 2.0),
        };
        self.round = matches!(stock.shape, StockShape::Cylinder { .. });
        let len = |mm: f64| units.plain(mm);
        self.text = [
            len(w),
            len(d),
            len(h),
            len(dia),
            len(stock.centre.x),
            len(stock.centre.y),
            len(stock.base),
            trim(stock.rotation),
        ];
        self.filled = self.text.clone();
    }

    /// The command that makes the stock the tab shows.
    fn request(&self, units: DisplayUnits) -> Result<Request, String> {
        let mut p = Map::new();
        let keys: &[&str] = if self.round {
            &["diameter", "height", "x", "y", "base"]
        } else {
            &["width", "depth", "height", "x", "y", "base", "rotation"]
        };
        for key in keys {
            let Some(i) = KEYS.iter().position(|k| k == key) else { continue };
            let text = self.text.get(i).map_or("", |t| t.trim());
            let v: f64 = text.parse().map_err(|_| format!("{} must be a number", label(key)))?;
            if !v.is_finite() {
                return Err(format!("{} must be a number", label(key)));
            }
            p.insert((*key).into(), json!(if *key == "rotation" { v } else { units.to_mm(v) }));
        }
        let command = if self.round { "stock.cylinder" } else { "stock.block" };
        Ok((command.into(), Value::Object(p)))
    }

    /// Draws the tab; commands go into `requests`. Returns whether it stays open.
    pub fn show(&mut self, ui: &mut Ui, session: &Session, tokens: &Tokens, requests: &mut Vec<Request>) -> bool {
        self.fill(session);
        let units = session.prefs.units;
        let mut keep = true;
        let mut apply = false;
        let mut error = None;
        ui.spacing_mut().item_spacing.y = 6.0;
        header(ui, tokens, self.round);
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("stock-panel")
            .max_height((ui.available_height() - 60.0).max(160.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                card(ui, tokens, "Shape", "A rectangular foam block, or a round billet standing on end.", |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (round, name) in [(false, "Block"), (true, "Round")] {
                            if ui.selectable_label(self.round == round, name).clicked() && self.round != round {
                                self.round = round;
                                apply = true;
                            }
                        }
                    });
                });
                let sizes: &[&str] = if self.round { &["diameter", "height"] } else { &["width", "depth", "height"] };
                card(ui, tokens, "Size", "Also set by clicking a dimension in the viewport.", |ui| {
                    for key in sizes {
                        apply |= self.row(ui, tokens, key, units.length());
                    }
                });
                let places: &[&str] = if self.round { &["x", "y", "base"] } else { &["x", "y", "base", "rotation"] };
                card(ui, tokens, "Placement", "Where it stands on the table; the turntable's axis is at 0, 0.", |ui| {
                    for key in places {
                        apply |= self.row(ui, tokens, key, if *key == "rotation" { "°" } else { units.length() });
                    }
                });
                if !session.job.models.is_empty() {
                    card(ui, tokens, "Fit", "Make it as large as the model needs, with an inch to spare.", |ui| {
                        if ui.button("Fit to model").clicked() {
                            requests.push(("stock.fit".into(), json!({})));
                        }
                    });
                }
            });
        if apply {
            match self.request(units) {
                Ok(request) => requests.push(request),
                Err(e) => error = Some(e),
            }
        }
        if let Some(e) = error {
            ui.label(RichText::new(e).color(tokens.error));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let done = egui::Button::new(RichText::new("Done").strong().color(tokens.on_accent))
                .fill(tokens.accent)
                .min_size(vec2(120.0, 30.0));
            if ui.add(done).clicked() {
                keep = false;
            }
            ui.label(RichText::new("Changes apply as you enter them; Undo takes them back.").color(tokens.text_muted));
        });
        keep
    }

    /// One value: its name, field and unit, what it is under them. Returns whether it was
    /// changed and entered.
    fn row(&mut self, ui: &mut Ui, tokens: &Tokens, key: &str, unit: &str) -> bool {
        let Some(i) = KEYS.iter().position(|k| *k == key) else { return false };
        let mut entered = false;
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(vec2(110.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(110.0);
                ui.label(RichText::new(label(key)).strong());
            });
            if let Some(text) = self.text.get_mut(i) {
                let response = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                entered = response.lost_focus() && self.filled.get(i) != Some(text);
            }
            ui.label(RichText::new(unit).color(tokens.text_muted));
        });
        ui.add(egui::Label::new(RichText::new(doc(key)).size(11.5).color(tokens.text_muted)).wrap());
        ui.add_space(4.0);
        entered
    }
}

fn header(ui: &mut Ui, tokens: &Tokens, round: bool) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), egui::Sense::hover());
        let icon = if round { "stock_cylinder" } else { "stock_block" };
        crate::ribbon::paint_named_icon(ui, rect, icon, tokens, true);
        ui.vertical(|ui| {
            ui.label(RichText::new("Stock").strong().size(18.0).color(tokens.text));
            ui.add(
                egui::Label::new(
                    RichText::new("The foam the part is cut from. Its dimensions show in the viewport.")
                        .color(tokens.text_muted),
                )
                .wrap(),
            );
        });
    });
}

fn card(ui: &mut Ui, tokens: &Tokens, title: &str, intro: &str, add: impl FnOnce(&mut Ui)) {
    Container::new(Surface::Card).show(ui, tokens, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong().size(15.0).color(tokens.text));
        ui.label(RichText::new(intro).color(tokens.text_muted));
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(8.0);
}

pub fn label(key: &str) -> &'static str {
    match key {
        "width" => "Width",
        "depth" => "Depth",
        "height" => "Height",
        "diameter" => "Diameter",
        "x" => "X",
        "y" => "Y",
        "base" => "Riser",
        _ => "Turn",
    }
}

fn doc(key: &str) -> &'static str {
    match key {
        "width" => "Across the wire at B = 0 (X).",
        "depth" => "Along the wire at B = 0 (Y).",
        "height" => "From its bottom to its top.",
        "diameter" => "Across the billet.",
        "x" => "Centre across the table, from the turntable's axis.",
        "y" => "Centre along the wire, from the turntable's axis.",
        "base" => "How high its bottom stands above the table top (a riser); 0 is on the table.",
        _ => "Turn of the block on the table, counter-clockwise from above.",
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}
