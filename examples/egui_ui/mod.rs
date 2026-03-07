pub mod cheat_search;
pub mod debugger;
pub mod gl_game;
pub mod hex_viewer;

use cheat_search::CheatSearchUi;
use debugger::{CpuSnapshot, DebuggerAction, DebuggerUi, VdcSnapshot};
use hex_viewer::HexViewerState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    HexViewer,
    CheatSearch,
    Debugger,
}

pub struct CheatToolUi {
    pub active_tab: ActiveTab,
    pub hex_viewer: HexViewerState,
    pub cheat_search_ui: CheatSearchUi,
    pub debugger_ui: DebuggerUi,
    pub panel_visible: bool,
    /// Frozen snapshot shown in the panel. Updated only on Refresh.
    pub ram_snapshot: Vec<u8>,
    /// Set to true when Refresh is clicked; consumed by the main loop.
    pub refresh_requested: bool,
    /// When true, emulation is paused (no ticks). Useful for cheat search.
    pub paused: bool,
    /// When true, hex viewer auto-refreshes every frame.
    pub auto_refresh: bool,
    /// Reusable buffer for combined work RAM + cart RAM (avoids per-frame alloc).
    combined_ram: Vec<u8>,
}

pub struct DebuggerPanelData<'a> {
    pub debugger: &'a pce::debugger::Debugger,
    pub cpu: CpuSnapshot,
    pub vdc: VdcSnapshot,
    pub vram: &'a [u16],
    pub palette_rgb: &'a dyn Fn(usize) -> u32,
    pub egui_ctx: &'a egui::Context,
}

impl CheatToolUi {
    pub fn new() -> Self {
        Self {
            active_tab: ActiveTab::HexViewer,
            hex_viewer: HexViewerState::new(),
            cheat_search_ui: CheatSearchUi::new(),
            debugger_ui: DebuggerUi::new(),
            panel_visible: false,
            ram_snapshot: vec![0u8; 0x2000],
            refresh_requested: false,
            paused: false,
            auto_refresh: true,
            combined_ram: Vec::new(),
        }
    }

    /// Copy live RAM into the display snapshot.
    pub fn refresh(&mut self, ram: &[u8]) {
        let prev = self.ram_snapshot.clone();
        let len = ram.len().min(self.ram_snapshot.len());
        self.ram_snapshot[..len].copy_from_slice(&ram[..len]);
        self.hex_viewer.update_prev(&prev);
    }

    pub fn show_panel(
        &mut self,
        ui: &mut egui::Ui,
        ram_writes: &mut Vec<(usize, u8)>,
        wram: &[u8],
        cram: Option<&[u8]>,
        cheat_path: Option<&std::path::Path>,
        debug: Option<DebuggerPanelData<'_>>,
    ) -> DebuggerAction {
        let mut debug_action = DebuggerAction::None;
        // Rebuild combined RAM view, reusing existing allocation.
        self.combined_ram.clear();
        self.combined_ram.extend_from_slice(wram);
        if let Some(c) = cram {
            self.combined_ram.extend_from_slice(c);
        }
        let live_ram = &self.combined_ram;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.active_tab, ActiveTab::HexViewer, "Hex Viewer");
            ui.selectable_value(&mut self.active_tab, ActiveTab::CheatSearch, "Cheat Search");
            ui.selectable_value(&mut self.active_tab, ActiveTab::Debugger, "Debugger");
            ui.separator();
            ui.checkbox(&mut self.paused, "Pause");
        });
        ui.separator();

        match self.active_tab {
            ActiveTab::HexViewer => {
                ui.horizontal(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.refresh_requested = true;
                    }
                    ui.checkbox(&mut self.auto_refresh, "Auto");
                });
                ui.separator();
                if self.auto_refresh {
                    self.refresh_requested = true;
                }
                let snap = &self.ram_snapshot;
                self.hex_viewer.show(ui, snap, ram_writes);
            }
            ActiveTab::CheatSearch => {
                self.cheat_search_ui.show(ui, live_ram, cheat_path);
            }
            ActiveTab::Debugger => {
                if let Some(debug) = debug {
                    self.debugger_ui.show(ui);
                    ui.separator();
                    debugger::show_registers(ui, debug.cpu, debug.vdc);
                    ui.separator();
                    ui.label("Breakpoints:");
                    self.debugger_ui.show_breakpoint_list(ui, debug.debugger);
                    ui.separator();
                    self.debugger_ui.vram_viewer.show_header(ui);
                    if self.debugger_ui.vram_viewer.auto_refresh {
                        self.debugger_ui.vram_viewer.refresh_requested = true;
                    }
                    if self.debugger_ui.vram_viewer.refresh_requested {
                        self.debugger_ui.vram_viewer.refresh_from_vram(
                            debug.egui_ctx,
                            debug.vram,
                            debug.palette_rgb,
                            512,
                        );
                        self.debugger_ui.vram_viewer.refresh_requested = false;
                    }
                    self.debugger_ui.vram_viewer.show(ui);
                    debug_action = self.debugger_ui.take_action();
                } else {
                    ui.label("Debugger data unavailable.");
                }
            }
        }
        debug_action
    }
}
