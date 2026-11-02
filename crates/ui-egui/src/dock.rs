//! Dockable workspace: panes can be dragged, stacked, split and resized.

use CosmicCut_engine::{Pane, Session};
use egui::{Id, Ui, WidgetText};
use egui_dock::{DockState, NodeIndex, SurfaceIndex, TabViewer};
use egui_wgpu::RenderState;

use crate::Request;
use crate::panes::{self, ConsoleState, GcodeState};
use crate::theme::Tokens;
use crate::viewport::Viewport;

/// Dock tabs are the engine's panes, so commands can name them.
pub type Tab = Pane;

/// Viewport in the middle, explorer over inspector on the left, the cut plane and G-code on
/// the right, the console along the bottom.
pub fn default_layout() -> DockState<Tab> {
    let mut state = DockState::new(vec![Pane::Viewport]);
    let tree = state.main_surface_mut();
    // Fractions size the left / upper side of each split.
    let [center, left] = tree.split_left(NodeIndex::root(), 0.19, vec![Pane::Explorer]);
    tree.split_below(left, 0.55, vec![Pane::Inspector]);
    let [center, _right] = tree.split_right(center, 0.66, vec![Pane::Plane, Pane::Gcode]);
    tree.split_below(center, 0.8, vec![Pane::Console, Pane::Instructions]);
    state
}

/// Panes currently in the dock, in `Pane::ALL` order.
pub fn open_panes(state: &DockState<Tab>) -> Vec<Pane> {
    Pane::ALL.into_iter().filter(|p| state.find_tab(p).is_some()).collect()
}

pub fn hide(state: &mut DockState<Tab>, pane: Pane) {
    if pane == Pane::Viewport {
        return;
    }
    if let Some(path) = state.find_tab(&pane) {
        state.remove_tab(path);
    }
}

/// Opens `pane` where it would be in the default layout, next to its neighbours.
pub fn show(state: &mut DockState<Tab>, pane: Pane) {
    if state.find_tab(&pane).is_some() {
        return;
    }
    let main_leaf = |state: &DockState<Tab>, p: Pane| {
        state.find_tab(&p).filter(|path| path.surface == SurfaceIndex::main()).map(|path| path.node)
    };
    let partner = match pane {
        Pane::Explorer => Some(Pane::Inspector),
        Pane::Inspector => Some(Pane::Explorer),
        Pane::Plane => Some(Pane::Gcode),
        Pane::Gcode | Pane::Cut | Pane::Stock | Pane::Machine | Pane::Post => Some(Pane::Plane),
        Pane::Instructions => Some(Pane::Console),
        Pane::Console => Some(Pane::Instructions),
        _ => None,
    };
    if let Some(partner) = partner
        && let Some(node) = main_leaf(state, partner)
    {
        match pane {
            Pane::Explorer => {
                state.main_surface_mut().split_above(node, 0.45, vec![pane]);
            }
            Pane::Inspector => {
                state.main_surface_mut().split_below(node, 0.55, vec![pane]);
            }
            // The cut plane and the G-code share one stack; so do the console and the instructions.
            _ => {
                if let Some(path) = state.find_tab(&partner) {
                    state.set_focused_node_and_surface(path.node_path());
                }
                state.push_to_focused_leaf(pane);
            }
        }
        return;
    }
    match main_leaf(state, Pane::Viewport) {
        Some(node) => {
            let surface = state.main_surface_mut();
            match pane {
                Pane::Console | Pane::Instructions => {
                    surface.split_below(node, 0.8, vec![pane]);
                }
                Pane::Plane | Pane::Gcode | Pane::Cut | Pane::Stock | Pane::Machine | Pane::Post => {
                    surface.split_right(node, 0.72, vec![pane]);
                }
                _ => {
                    surface.split_left(node, 0.19, vec![pane]);
                }
            }
        }
        None => state.push_to_first_leaf(pane),
    }
}

/// Opens `pane` if needed and makes it the active tab of its stack.
pub fn focus(state: &mut DockState<Tab>, pane: Pane) {
    show(state, pane);
    if let Some(path) = state.find_tab(&pane) {
        let _ = state.set_active_tab(path);
        state.set_focused_node_and_surface(path.node_path());
    }
}

pub struct Viewer<'a> {
    pub session: &'a mut Session,
    pub tokens: &'a Tokens,
    pub viewport: &'a mut Viewport,
    pub render_state: Option<&'a RenderState>,
    pub console: &'a mut ConsoleState,
    pub gcode: &'a mut GcodeState,
    pub requests: &'a mut Vec<Request>,
    /// The cut being edited, shown in the Cut tab, and what its buttons asked for this frame:
    /// a command to run, and whether to stay open.
    pub cut_panel: &'a mut Option<crate::cut_panel::CutPanel>,
    pub cut_outcome: &'a mut Option<(Option<Request>, bool)>,
    /// The stock being set up, shown in the Stock tab, and whether Done was pressed.
    pub stock_panel: &'a mut Option<crate::stock_panel::StockPanel>,
    pub stock_done: &'a mut bool,
    /// The machine and the post processor being set up, in their tabs, and what a tab's
    /// buttons asked for this frame: the tab, a command to run, and whether to stay open.
    pub machine_panel: &'a mut Option<crate::machine_panel::MachinePanel>,
    pub post_panel: &'a mut Option<crate::post_panel::PostPanel>,
    pub side_outcome: &'a mut Option<(Pane, Option<Request>, bool)>,
    /// Which of the console and the instructions was showing this frame.
    pub bottom: &'a mut Option<Pane>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> Id {
        Id::new(("dock-tab", tab.id()))
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Tab) {
        if matches!(tab, Pane::Console | Pane::Instructions) {
            *self.bottom = Some(*tab);
        }
        match tab {
            Pane::Instructions => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(4.0);
                    for (k, line) in self.viewport.instructions.lines().enumerate() {
                        let text = egui::RichText::new(line).size(14.0);
                        let text = if k == 0 { text.color(self.tokens.text) } else { text.color(self.tokens.accent) };
                        ui.add(egui::Label::new(text).wrap());
                    }
                });
            }
            Pane::Viewport => self.viewport.show(ui, self.session, self.tokens, self.render_state, self.requests),
            Pane::Explorer => panes::explorer(ui, self.session, self.tokens, self.requests),
            Pane::Inspector => panes::inspector(ui, self.session, self.tokens),
            Pane::Plane => panes::cut_plane(ui, self.session, self.tokens, self.viewport.preview.as_ref()),
            Pane::Gcode => panes::gcode(ui, self.session, self.tokens, self.gcode, self.requests),
            Pane::Console => panes::console(ui, self.session, self.tokens, self.console, self.requests),
            Pane::Machine => match self.machine_panel {
                Some(panel) => {
                    let (request, keep) = panel.show(ui, self.session, self.tokens);
                    *self.side_outcome = Some((Pane::Machine, request, keep));
                }
                None => {
                    ui.label(
                        egui::RichText::new("Open the machine with Machine on the ribbon.")
                            .color(self.tokens.text_muted),
                    );
                }
            },
            Pane::Post => match self.post_panel {
                Some(panel) => {
                    let (request, keep) = panel.show(ui, self.session, self.tokens);
                    *self.side_outcome = Some((Pane::Post, request, keep));
                }
                None => {
                    ui.label(
                        egui::RichText::new("Open a post processor with Post processor on the ribbon.")
                            .color(self.tokens.text_muted),
                    );
                }
            },
            Pane::Stock => match self.stock_panel {
                Some(panel) => *self.stock_done = !panel.show(ui, self.session, self.tokens, self.requests),
                None => {
                    ui.label(
                        egui::RichText::new(
                            "Open the stock with Stock on the ribbon, or double-click it in the explorer.",
                        )
                        .color(self.tokens.text_muted),
                    );
                }
            },
            Pane::Cut => match self.cut_panel {
                Some(panel) => *self.cut_outcome = Some(panel.show(ui, self.session, self.tokens)),
                None => {
                    ui.label(
                        egui::RichText::new("Open a cut from the Cuts gallery, or double-click one in the explorer.")
                            .color(self.tokens.text_muted),
                    );
                }
            },
        }
    }

    fn is_closeable(&self, tab: &Tab) -> bool {
        *tab != Pane::Viewport
    }

    fn allowed_in_windows(&self, tab: &mut Tab) -> bool {
        // The viewport stays docked; its GPU target is sized to the dock.
        *tab != Pane::Viewport
    }

    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Pane::Inspector => [true, true],
            _ => [false, false],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_every_pane_once() {
        let state = default_layout();
        let tabs: Vec<Tab> = state.iter_all_tabs().map(|(_, t)| *t).collect();
        for tab in Pane::ALL {
            // The editor tabs open with something being edited.
            let want = usize::from(![Pane::Cut, Pane::Stock, Pane::Machine, Pane::Post].contains(&tab));
            assert_eq!(tabs.iter().filter(|t| **t == tab).count(), want, "{tab:?}");
        }
    }

    #[test]
    fn panes_hide_show_and_focus() {
        let mut state = default_layout();
        for pane in Pane::ALL {
            hide(&mut state, pane);
        }
        assert_eq!(open_panes(&state), vec![Pane::Viewport]);
        for pane in Pane::ALL {
            focus(&mut state, pane);
        }
        assert_eq!(open_panes(&state), Pane::ALL.to_vec());
        assert_eq!(state.iter_all_tabs().count(), Pane::ALL.len());
    }
}
