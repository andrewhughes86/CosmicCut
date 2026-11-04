//! The one outer UI container. Feature modules declare a [`Surface`] and get their
//! panel or frame from here; they never build raw egui frames, panels or windows.

use egui::{CornerRadius, Frame, InnerResponse, Margin, Popup, Response, Stroke, Ui};

use crate::theme::Tokens;

/// Semantic role of a container; it decides fill, border and spacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The contextual ribbon across the top.
    Ribbon,
    /// One labelled group of ribbon buttons.
    RibbonGroup,
    /// The status bar along the bottom.
    StatusBar,
    /// The dock area that hosts workspace panes.
    Workspace,
    /// Floating chrome over the viewport (hints, readouts).
    Overlay,
    /// Dropdown menus opened from buttons.
    Menu,
    /// Floating dialogs that gather a command's input.
    Dialog,
    /// A question before something that cannot be taken back lightly (deleting): it glows in
    /// the alert colour.
    Alert,
    /// A settings panel docked at the side of the window (the toolpath editor).
    SidePanel,
    /// A group of settings inside a panel.
    Card,
}

pub struct Container {
    surface: Surface,
}

impl Container {
    pub fn new(surface: Surface) -> Self {
        Self { surface }
    }

    pub fn frame(&self, t: &Tokens) -> Frame {
        match self.surface {
            // The ribbon body is one flat surface; groups are separated by rules, not boxes.
            Surface::Ribbon => {
                Frame::new().fill(t.panel_bg).inner_margin(Margin::ZERO).stroke(Stroke::new(1.0, t.border))
            }
            Surface::RibbonGroup => Frame::new().inner_margin(Margin::symmetric(4, 2)),
            Surface::StatusBar => {
                Frame::new().fill(t.raised_bg).inner_margin(Margin::symmetric(10, 3)).stroke(Stroke::new(1.0, t.border))
            }
            Surface::Workspace => Frame::new().fill(t.app_bg).inner_margin(Margin::same(4)),
            Surface::Overlay => Frame::new()
                .fill(t.panel_bg.gamma_multiply(0.92))
                .inner_margin(Margin::symmetric(8, 4))
                .corner_radius(CornerRadius::same(4))
                .stroke(Stroke::new(1.0, t.border)),
            Surface::SidePanel => {
                Frame::new().fill(t.panel_bg).inner_margin(Margin::same(10)).stroke(Stroke::new(1.0, t.border))
            }
            // Windows glow in the accent colour all round, so they stand off the work behind.
            Surface::Dialog => Frame::new()
                .fill(t.panel_bg)
                .inner_margin(Margin::same(12))
                .corner_radius(CornerRadius::same(6))
                .stroke(Stroke::new(1.0, t.accent.gamma_multiply(0.55)))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 0],
                    blur: 26,
                    spread: 2,
                    color: t.accent.gamma_multiply(0.38),
                }),
            // The alert colour all round and a strong glow of it, over the dimmed window.
            Surface::Alert => Frame::new()
                .fill(t.panel_bg)
                .inner_margin(Margin::ZERO)
                .corner_radius(CornerRadius::same(8))
                .stroke(Stroke::new(2.0, alert_colour(t)))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 0],
                    blur: 40,
                    spread: 4,
                    color: alert_colour(t).gamma_multiply(0.6),
                }),
            Surface::Card => Frame::new()
                .fill(t.raised_bg)
                .inner_margin(Margin::symmetric(14, 12))
                .corner_radius(CornerRadius::same(6))
                .stroke(Stroke::new(1.0, t.border)),
            Surface::Menu => Frame::new()
                .fill(t.panel_bg)
                .inner_margin(Margin::same(4))
                .corner_radius(CornerRadius::same(4))
                .stroke(Stroke::new(1.0, t.border)),
        }
    }

    pub fn show<R>(self, ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        self.frame(t).show(ui, add)
    }
}

/// A dropdown menu that opens below `button` when it is clicked.
pub struct Menu<'a> {
    button: &'a Response,
}

impl<'a> Menu<'a> {
    pub fn below(button: &'a Response) -> Self {
        Self { button }
    }

    pub fn show<R>(self, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        Popup::menu(self.button).frame(Container::new(Surface::Menu).frame(t)).show(add)
    }

    /// The same menu, opened by right-clicking `button` instead.
    pub fn context<R>(self, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        Popup::context_menu(self.button).frame(Container::new(Surface::Menu).frame(t)).show(add)
    }
}

/// Edge-docked or central application panels, framed by their [`Surface`].
pub struct Panel {
    kind: PanelKind,
    surface: Surface,
}

enum PanelKind {
    Top(&'static str),
    Bottom(&'static str),
    /// Resizable, at the right of the window, starting this wide.
    Right(&'static str, f32),
    Central,
}

impl Panel {
    pub fn top(id: &'static str, surface: Surface) -> Self {
        Self { kind: PanelKind::Top(id), surface }
    }

    pub fn bottom(id: &'static str, surface: Surface) -> Self {
        Self { kind: PanelKind::Bottom(id), surface }
    }

    pub fn right(id: &'static str, surface: Surface, width: f32) -> Self {
        Self { kind: PanelKind::Right(id, width), surface }
    }

    pub fn central(surface: Surface) -> Self {
        Self { kind: PanelKind::Central, surface }
    }

    pub fn show<R>(self, ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let frame = Container::new(self.surface).frame(t);
        match self.kind {
            PanelKind::Top(id) => egui::Panel::top(id).resizable(false).frame(frame).show(ui, add),
            PanelKind::Bottom(id) => egui::Panel::bottom(id).resizable(false).frame(frame).show(ui, add),
            PanelKind::Right(id, width) => {
                egui::Panel::right(id).resizable(true).default_size(width).min_size(280.0).frame(frame).show(ui, add)
            }
            PanelKind::Central => egui::CentralPanel::default().frame(frame).show(ui, add),
        }
    }
}

/// A floating, titled dialog window framed as [`Surface::Dialog`].
pub struct Window {
    title: String,
    /// Tool panels open at the upper left, clear of the part; dialogs open centred.
    tool: bool,
    /// Resizable windows (libraries, browsers) and their starting size.
    size: Option<egui::Vec2>,
}

impl Window {
    pub fn dialog(title: impl Into<String>) -> Self {
        Self { title: title.into(), tool: false, size: None }
    }

    /// A centred, resizable window that stays open beside the work (the tool library).
    pub fn browser(title: impl Into<String>, size: egui::Vec2) -> Self {
        Self { title: title.into(), tool: false, size: Some(size) }
    }

    /// A panel that stays open while the user works in the viewport.
    pub fn tool(title: impl Into<String>) -> Self {
        Self { title: title.into(), tool: true, size: None }
    }

    /// Shows the window while `open` is true; the title bar's close button clears it.
    pub fn show<R>(
        self,
        ctx: &egui::Context,
        t: &Tokens,
        open: &mut bool,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> Option<R> {
        let title = self.title.clone();
        let mut close = false;
        let shown = egui::Window::new(self.title)
            .open(open)
            .title_bar(false)
            .collapsible(false)
            .resizable(self.size.is_some())
            .max_size(ctx.content_rect().size() * 0.95)
            .default_size(self.size.unwrap_or(egui::vec2(320.0, 200.0)))
            .pivot(if self.tool { egui::Align2::LEFT_TOP } else { egui::Align2::CENTER_CENTER })
            .default_pos(if self.tool {
                ctx.content_rect().left_top() + egui::vec2(280.0, 160.0)
            } else {
                ctx.content_rect().center()
            })
            .frame(Container::new(Surface::Dialog).frame(t).inner_margin(Margin::ZERO))
            .show(ctx, |ui| {
                // A compact title bar, the height of the main window's tab strip. It is painted
                // once the content has set the window's width.
                let spacing = ui.spacing().item_spacing;
                ui.spacing_mut().item_spacing.y = 0.0;
                let top = ui.cursor().min;
                ui.add_space(TITLE_HEIGHT);
                let inner = Frame::new()
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = spacing;
                        add(ui)
                    })
                    .inner;
                let bar = egui::Rect::from_min_max(top, egui::pos2(ui.min_rect().right(), top.y + TITLE_HEIGHT));
                close = title_bar(ui, t, bar, &title);
                inner
            })
            .and_then(|r| r.inner);
        if close {
            *open = false;
        }
        shown
    }
}

/// The colour of an alert: the stock's purple on dark, red on light.
pub fn alert_colour(t: &Tokens) -> egui::Color32 {
    if t.dark { t.stock } else { t.error }
}

/// Text that reads on a `fill`: near black on a light one, white on a dark one.
pub fn ink_on(fill: egui::Color32) -> egui::Color32 {
    let l = 0.2126 * f32::from(fill.r()) + 0.7152 * f32::from(fill.g()) + 0.0722 * f32::from(fill.b());
    if l > 100.0 { egui::Color32::from_rgb(24, 6, 6) } else { egui::Color32::WHITE }
}

/// A question that has to be answered before anything else: the window behind dimmed, the
/// question in the middle on [`Surface::Alert`], its title bar in the alert colour.
pub struct Alert {
    title: String,
}

impl Alert {
    pub fn danger(title: impl Into<String>) -> Self {
        Self { title: title.into() }
    }

    /// Shows the alert; `open` is cleared by Esc or a click outside it.
    pub fn show<R>(self, ctx: &egui::Context, t: &Tokens, open: &mut bool, add: impl FnOnce(&mut Ui) -> R) -> R {
        let shown = egui::Modal::new(egui::Id::new(("alert", &self.title)))
            .frame(Container::new(Surface::Alert).frame(t))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                let (bar, _) = ui.allocate_exact_size(egui::vec2(440.0, 34.0), egui::Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(bar, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, alert_colour(t));
                painter.text(
                    egui::pos2(bar.left() + 16.0, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("⚠  {}", self.title),
                    egui::FontId::proportional(16.0),
                    ink_on(alert_colour(t)),
                );
                Frame::new().inner_margin(Margin::symmetric(18, 16)).show(ui, add).inner
            });
        if shown.should_close() {
            *open = false;
        }
        shown.inner
    }
}

/// Height of a window's title bar.
const TITLE_HEIGHT: f32 = 24.0;

/// Paints a window's title bar into `bar`; true when its close button was clicked.
fn title_bar(ui: &Ui, t: &Tokens, bar: egui::Rect, title: &str) -> bool {
    let painter = ui.painter();
    painter.rect_filled(bar, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.raised_bg);
    painter.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, t.border));
    painter.text(
        egui::pos2(bar.left() + 10.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(13.0),
        t.text,
    );
    let button = egui::Rect::from_min_max(egui::pos2(bar.right() - 32.0, bar.top()), bar.right_bottom());
    let response = ui.interact(button, ui.id().with("window-close"), egui::Sense::click());
    let ink = if response.hovered() {
        painter.rect_filled(button.shrink2(egui::vec2(4.0, 3.0)), 3.0, t.error);
        Stroke::new(1.2, egui::Color32::WHITE)
    } else {
        Stroke::new(1.2, t.text_muted)
    };
    let (c, h) = (button.center(), 4.0);
    painter.line_segment([c + egui::vec2(-h, -h), c + egui::vec2(h, h)], ink);
    painter.line_segment([c + egui::vec2(-h, h), c + egui::vec2(h, -h)], ink);
    response.on_hover_text("Close").clicked()
}
