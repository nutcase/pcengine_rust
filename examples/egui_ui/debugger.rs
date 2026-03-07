use egui::{self, Color32, FontId, RichText};
use pce::debugger::{DebugBreak, Debugger};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DebuggerAction {
    None,
    TogglePause,
    Step,
    ClearBreak,
    AddBreakpoint(u16),
    RemoveBreakpoint(u16),
}

pub struct DebuggerUi {
    breakpoint_input: String,
    pub last_action: DebuggerAction,
    pub vram_viewer: VramViewer,
}

impl DebuggerUi {
    pub fn new() -> Self {
        Self {
            breakpoint_input: String::new(),
            last_action: DebuggerAction::None,
            vram_viewer: VramViewer::new(),
        }
    }

    pub fn take_action(&mut self) -> DebuggerAction {
        std::mem::replace(&mut self.last_action, DebuggerAction::None)
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.last_action = DebuggerAction::None;
        ui.heading("Debugger");
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Pause/Run").clicked() {
                self.last_action = DebuggerAction::TogglePause;
            }
            if ui.button("Step").clicked() {
                self.last_action = DebuggerAction::Step;
            }
            if ui.button("Clear Break").clicked() {
                self.last_action = DebuggerAction::ClearBreak;
            }
        });
        ui.separator();
        ui.label("Breakpoints:");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.breakpoint_input).desired_width(80.0));
            if ui.button("Add").clicked() {
                if let Some(pc) = parse_hex_u16(&self.breakpoint_input) {
                    self.last_action = DebuggerAction::AddBreakpoint(pc);
                }
            }
        });
    }

    pub fn show_breakpoint_list(&mut self, ui: &mut egui::Ui, debugger: &Debugger) {
        let mono = FontId::monospace(12.0);
        let mut any = false;
        for &pc in &debugger.breakpoints {
            any = true;
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("${:04X}", pc)).font(mono.clone()));
                if ui.button("Remove").clicked() {
                    self.last_action = DebuggerAction::RemoveBreakpoint(pc);
                }
            });
        }
        if !any {
            ui.label(RichText::new("(none)").color(Color32::GRAY));
        }
    }
}

#[derive(Clone, Copy)]
pub struct CpuSnapshot {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub status: u8,
    pub last_opcode: u8,
}

#[derive(Clone, Copy)]
pub struct VdcSnapshot {
    pub status: u8,
    pub scanline: u16,
    pub in_vblank: bool,
    pub vram_dma_busy: bool,
}

pub fn show_registers(ui: &mut egui::Ui, cpu: CpuSnapshot, vdc: VdcSnapshot) {
    let mono = FontId::monospace(12.0);
    ui.heading("CPU");
    ui.label(RichText::new(format!("PC ${:04X}", cpu.pc)).font(mono.clone()));
    ui.label(
        RichText::new(format!(
            "A  ${:02X}  X ${:02X}  Y ${:02X}",
            cpu.a, cpu.x, cpu.y
        ))
        .font(mono.clone()),
    );
    ui.label(
        RichText::new(format!(
            "SP ${:02X}  P ${:02X}  OP ${:02X}",
            cpu.sp, cpu.status, cpu.last_opcode
        ))
        .font(mono.clone()),
    );
    ui.separator();
    ui.heading("VDC");
    ui.label(RichText::new(format!("Scanline {}", vdc.scanline)).font(mono.clone()));
    ui.label(RichText::new(format!("Status ${:02X}", vdc.status)).font(mono.clone()));
    ui.label(
        RichText::new(format!(
            "VBlank {}",
            if vdc.in_vblank { "yes" } else { "no" }
        ))
        .font(mono.clone()),
    );
    ui.label(
        RichText::new(format!(
            "DMA busy {}",
            if vdc.vram_dma_busy { "yes" } else { "no" }
        ))
        .font(mono.clone()),
    );
}

pub fn describe_break(break_event: DebugBreak) -> &'static str {
    match break_event {
        DebugBreak::Breakpoint(_) => "BREAKPOINT",
        DebugBreak::Step(_) => "STEP",
    }
}

fn parse_hex_u16(s: &str) -> Option<u16> {
    let trimmed = s.trim().trim_start_matches('$');
    let trimmed = trimmed.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(trimmed, 16).ok()
}

pub struct VramViewer {
    pub auto_refresh: bool,
    pub refresh_requested: bool,
    texture: Option<egui::TextureHandle>,
    tile_cols: usize,
    tile_rows: usize,
    rgba: Vec<Color32>,
}

impl VramViewer {
    pub fn new() -> Self {
        Self {
            auto_refresh: false,
            refresh_requested: false,
            texture: None,
            tile_cols: 32,
            tile_rows: 16,
            rgba: Vec::new(),
        }
    }

    pub fn show_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Refresh VRAM").clicked() {
                self.refresh_requested = true;
            }
            ui.checkbox(&mut self.auto_refresh, "Auto");
        });
    }

    pub fn ensure_texture(&mut self, ui: &egui::Context, w: usize, h: usize) {
        let needs = self
            .texture
            .as_ref()
            .map(|t| t.size()[0] as usize != w || t.size()[1] as usize != h)
            .unwrap_or(true);
        if needs {
            let image = egui::ColorImage {
                size: [w, h],
                pixels: vec![Color32::BLACK; w * h],
            };
            self.texture = Some(ui.load_texture("vram_view", image, egui::TextureOptions::NEAREST));
        }
    }

    pub fn update_texture(&mut self, ui: &egui::Context, pixels: &[Color32], w: usize, h: usize) {
        self.ensure_texture(ui, w, h);
        if let Some(tex) = &mut self.texture {
            let image = egui::ColorImage {
                size: [w, h],
                pixels: pixels.to_vec(),
            };
            tex.set(image, egui::TextureOptions::NEAREST);
        }
    }

    pub fn refresh_from_vram(
        &mut self,
        ui: &egui::Context,
        vram: &[u16],
        palette_rgb: &dyn Fn(usize) -> u32,
        tile_count: usize,
    ) {
        let (w, h) = self.render_tiles(vram, palette_rgb, tile_count);
        self.update_texture(ui, &self.rgba, w, h);
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let Some(tex) = &self.texture else {
            ui.label("VRAM viewer not initialized.");
            return;
        };
        let size = tex.size();
        let desired = egui::vec2(size[0] as f32, size[1] as f32);
        ui.image((tex.id(), desired));
    }

    pub fn render_tiles(
        &mut self,
        vram: &[u16],
        palette_rgb: &dyn Fn(usize) -> u32,
        tile_count: usize,
    ) -> (usize, usize) {
        let cols = self.tile_cols;
        let rows = self.tile_rows;
        let tiles = tile_count.min(cols * rows);
        let width_px = cols * 8;
        let height_px = rows * 8;
        self.rgba.resize(width_px * height_px, Color32::BLACK);
        for tile_idx in 0..tiles {
            let tile_x = tile_idx % cols;
            let tile_y = tile_idx / cols;
            let base = (tile_idx * 16) & 0x7FFF;
            for row in 0..8 {
                let word0 = vram.get(base + row).copied().unwrap_or(0);
                let word1 = vram.get(base + row + 8).copied().unwrap_or(0);
                for bit in 0..8 {
                    let shift = 7 - bit;
                    let plane0 = ((word0 >> shift) & 1) as u8;
                    let plane1 = ((word0 >> (shift + 8)) & 1) as u8;
                    let plane2 = ((word1 >> shift) & 1) as u8;
                    let plane3 = ((word1 >> (shift + 8)) & 1) as u8;
                    let pixel = plane0 | (plane1 << 1) | (plane2 << 2) | (plane3 << 3);
                    let color = palette_rgb(pixel as usize);
                    let r = ((color >> 16) & 0xFF) as u8;
                    let g = ((color >> 8) & 0xFF) as u8;
                    let b = (color & 0xFF) as u8;
                    let px = tile_x * 8 + bit;
                    let py = tile_y * 8 + row;
                    self.rgba[py * width_px + px] = Color32::from_rgb(r, g, b);
                }
            }
        }
        (width_px, height_px)
    }
}
