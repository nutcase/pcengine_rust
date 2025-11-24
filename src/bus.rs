pub const PAGE_SIZE: usize = 0x2000; // 8 KiB per bank
const NUM_BANKS: usize = 8;
const RAM_SIZE: usize = PAGE_SIZE * NUM_BANKS;
const IO_REG_SIZE: usize = PAGE_SIZE; // full hardware page
pub const IRQ_DISABLE_IRQ2: u8 = 0x01;
pub const IRQ_DISABLE_IRQ1: u8 = 0x02;
pub const IRQ_DISABLE_TIMER: u8 = 0x04;
pub const IRQ_REQUEST_IRQ2: u8 = 0x01;
pub const IRQ_REQUEST_IRQ1: u8 = 0x02;
pub const IRQ_REQUEST_TIMER: u8 = 0x04;
const TIMER_CONTROL_START: u8 = 0x01;
const VDC_REGISTER_COUNT: usize = 32;
const LINES_PER_FRAME: u16 = 262;
const HW_JOYPAD_BASE: usize = 0x1000;
const HW_IRQ_BASE: usize = 0x1400;
const HW_CPU_CTRL_BASE: usize = 0x1C00;
const VDC_VBLANK_INTERVAL: u32 = 119_318; // ~7.16 MHz / 60 Hz
const MASTER_CLOCK_HZ: u32 = 7_159_090;
const AUDIO_SAMPLE_RATE: u32 = 44_100;
const PHI_CYCLES_PER_SAMPLE: u32 = MASTER_CLOCK_HZ / AUDIO_SAMPLE_RATE;
const VDC_BUSY_ACCESS_CYCLES: u32 = 64;
const VDC_DMA_WORD_CYCLES: u32 = 8;
const FRAME_WIDTH: usize = 256;
const FRAME_HEIGHT: usize = 240;
const TILE_WIDTH: usize = 8;
const TILE_HEIGHT: usize = 8;
const SPRITE_COUNT: usize = 64;
const VDC_CTRL_ENABLE_SPRITES_LEGACY: u16 = 0x0040;
const VDC_CTRL_ENABLE_BACKGROUND_LEGACY: u16 = 0x0080;
const VDC_CTRL_ENABLE_BACKGROUND: u16 = 0x0100;
const VDC_CTRL_ENABLE_SPRITES: u16 = 0x0200;
const DCR_ENABLE_VRAM_DMA: u8 = 0x01;
const DCR_ENABLE_CRAM_DMA: u8 = 0x02;
const DCR_ENABLE_SATB_DMA: u8 = 0x04;
const DCR_ENABLE_CRAM_DMA_ALT: u8 = 0x20; // Some docs describe bit5 as CRAM DMA enable.

#[derive(Clone, Copy)]
enum VdcPort {
    Control,
    Data,
}

/// Memory bus exposing an 8x8 KiB banked window into linear RAM/ROM data.
/// This mirrors the HuC6280 page architecture and provides simple helpers
/// for experimenting with bank switching.
#[derive(Clone)]
pub struct Bus {
    ram: Vec<u8>,
    rom: Vec<u8>,
    banks: [BankMapping; NUM_BANKS],
    mpr: [u8; NUM_BANKS],
    st_ports: [u8; 3],
    io: [u8; IO_REG_SIZE],
    io_port: IoPort,
    interrupt_disable: u8,
    interrupt_request: u8,
    timer: Timer,
    vdc: Vdc,
    psg: Psg,
    vce: Vce,
    audio_phi_accumulator: u32,
    audio_buffer: Vec<i16>,
    framebuffer: Vec<u32>,
    frame_ready: bool,
    cart_ram: Vec<u8>,
    bg_opaque: Vec<bool>,
    bg_priority: Vec<bool>,
    sprite_line_counts: Vec<u8>,
    io_write_hist: std::collections::HashMap<u16, u64>,
    vce_write_count: u64,
    vce_data_writes: u64,
    vce_control_writes: u64,
    vce_port_hits: u64,
    cram_dma_count: u64,
    vce_last_port_addr: u16,
    vce_last_control_high: u8,
    vce_last_control_high_max: u8,
    vdc_alias_write_counts: [u64; 0x20],
    #[cfg(feature = "trace_hw_writes")]
    last_pc_for_trace: Option<u16>,
    #[cfg(debug_assertions)]
    debug_force_ds_after: u64,
    #[cfg(feature = "trace_hw_writes")]
    st0_lock_window: u8,
}

impl Bus {
    #[inline]
    fn env_force_cram_dma() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| std::env::var("PCE_FORCE_CRAM_DMA").is_ok())
    }

    #[inline]
    fn env_force_mpr1_hardware() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| std::env::var("PCE_FORCE_MPR1_HW").is_ok())
    }

    #[inline]
    fn env_force_display_on() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| std::env::var("PCE_FORCE_DISPLAY_ON").is_ok())
    }

    #[inline]
    fn env_relax_io_mirror() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| {
            // Default to relaxed mapping so BIOS routines that stream through
            // 0x2000–0x23FF still hit the hardware even after MPR1 is remapped
            // to RAM. Set PCE_RELAX_IO_MIRROR=0 to restore strict decoding.
            match std::env::var("PCE_RELAX_IO_MIRROR") {
                Ok(val) if val == "0" => false,
                _ => true,
            }
        })
    }

    #[inline]
    fn env_fold_io_02xx() -> bool {
        std::env::var("PCE_FOLD_IO_02XX").is_ok()
    }

    #[inline]

    #[inline]
    fn env_vdc_busy_divisor() -> u32 {
        use std::sync::OnceLock;
        static DIV: OnceLock<u32> = OnceLock::new();
        *DIV.get_or_init(|| {
            std::env::var("PCE_VDC_BUSY_DIV")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|&n| n > 0)
                .unwrap_or(1)
        })
    }

    #[inline]
    fn env_force_test_palette() -> bool {
        std::env::var("PCE_FORCE_TEST_PALETTE").is_ok()
    }

    #[inline]
    fn env_vce_force_data() -> bool {
        std::env::var("PCE_VCE_FORCE_DATA").is_ok()
    }

    #[inline]
    fn env_vce_free_pass() -> bool {
        std::env::var("PCE_VCE_FREE_PASS").is_ok()
    }

    #[inline]
    fn env_vce_route_mirror_as_data() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        // Many HuCARDs (including the bundled System Card BIOS) stream CRAM
        // data through the VCE mirrors at 0x0404/0405 where A2=1 but A1..A0
        // still select the data port. Treating bit2 mirrors as data by default
        // lets those routines succeed without extra configuration. Set
        // PCE_VCE_ROUTE_MIRROR_AS_DATA=0 to revert to strict decoding.
        *FLAG.get_or_init(|| match std::env::var("PCE_VCE_ROUTE_MIRROR_AS_DATA") {
            Ok(val) if val == "0" => false,
            _ => true,
        })
    }

    #[inline]
    fn env_vce_catchall() -> bool {
        std::env::var("PCE_VCE_CATCHALL").is_ok()
    }

    #[inline]
    fn env_trace_mpr() -> bool {
        std::env::var("PCE_TRACE_MPR").is_ok()
    }

    #[inline]
    fn env_extreme_mirror() -> bool {
        std::env::var("PCE_VDC_EXTREME_MIRROR").is_ok()
    }

    #[inline]
    fn env_vdc_ultra_mirror() -> bool {
        std::env::var("PCE_VDC_ULTRA_MIRROR").is_ok()
    }

    #[inline]
    fn env_vdc_catchall() -> bool {
        std::env::var("PCE_VDC_CATCHALL").is_ok()
    }

    #[inline]
    fn env_pad_default() -> u8 {
        use std::sync::OnceLock;
        static PAD: OnceLock<u8> = OnceLock::new();
        *PAD.get_or_init(|| {
            std::env::var("PCE_PAD_DEFAULT")
                .ok()
                .and_then(|s| u8::from_str_radix(&s, 16).ok())
                .unwrap_or(0xFF)
        })
    }

    #[inline]
    fn env_irq_status_default() -> Option<u8> {
        std::env::var("PCE_IRQ_STATUS_DEFAULT")
            .ok()
            .and_then(|s| u8::from_str_radix(&s, 16).ok())
    }

    #[inline]
    fn env_timer_default_start() -> bool {
        std::env::var("PCE_TIMER_DEFAULT_START").is_ok()
    }

    #[inline]
    fn env_force_palette_every_frame() -> bool {
        std::env::var("PCE_FORCE_PALETTE").is_ok()
    }
    #[inline]
    fn vce_ports_swapped() -> bool {
        use std::sync::OnceLock;
        static SWAP: OnceLock<bool> = OnceLock::new();
        *SWAP.get_or_init(|| std::env::var("PCE_VCE_SWAP_PORTS").is_ok())
    }
    pub fn new() -> Self {
        let mut bus = Self {
            ram: vec![0; RAM_SIZE],
            rom: Vec::new(),
            banks: [BankMapping::Ram { base: 0 }; NUM_BANKS],
            mpr: [0; NUM_BANKS],
            st_ports: [0; 3],
            io: [0; IO_REG_SIZE],
            io_port: IoPort::new(),
            interrupt_disable: 0,
            interrupt_request: 0,
            timer: Timer::new(),
            vdc: Vdc::new(),
            psg: Psg::new(),
            vce: Vce::new(),
            audio_phi_accumulator: 0,
            audio_buffer: Vec::new(),
            framebuffer: vec![0; FRAME_WIDTH * FRAME_HEIGHT],
            frame_ready: false,
            cart_ram: Vec::new(),
            bg_opaque: vec![false; FRAME_WIDTH * FRAME_HEIGHT],
            bg_priority: vec![false; FRAME_WIDTH * FRAME_HEIGHT],
            sprite_line_counts: vec![0; FRAME_HEIGHT],
            vce_write_count: 0,
            vce_data_writes: 0,
            vce_control_writes: 0,
            vce_port_hits: 0,
            cram_dma_count: 0,
            vce_last_port_addr: 0,
            vce_last_control_high: 0,
            vce_last_control_high_max: 0,
            vdc_alias_write_counts: [0; 0x20],
            #[cfg(feature = "trace_hw_writes")]
            last_pc_for_trace: None,
            #[cfg(debug_assertions)]
            debug_force_ds_after: 0,
            #[cfg(feature = "trace_hw_writes")]
            st0_lock_window: 0,
            io_write_hist: std::collections::HashMap::new(),
        };

        // Power-on mapping: expose internal RAM in bank 0 for ZP/stack and
        // the hardware page in bank 1, mirroring HuC6280 reset behaviour.
        // Remaining banks default to RAM; the HuCARD loader remaps banks 4–7
        // to ROM after parsing the image header.
        let ram_pages = RAM_SIZE / PAGE_SIZE;
        for index in 0..NUM_BANKS {
            let page = index % ram_pages;
            bus.mpr[index] = 0xF8u8.saturating_add(page as u8);
            bus.update_mpr(index);
        }
        // On real hardware MPR0 starts mapped to the hardware page so that
        // $0000-$1FFF hits VDC/VCE/PSG on reset.
        bus.mpr[0] = 0xFF;
        bus.update_mpr(0);
        // Keep the top bank pointing at RAM so the reset vector can be patched
        // when loading raw programs; HuCARD mapping will override this later.
        bus.mpr[NUM_BANKS - 1] = 0xF8;
        bus.update_mpr(NUM_BANKS - 1);
        // Hardware page on reset so BIOS can hit VDC/VCE before MPR1 writes.
        bus.mpr[1] = 0xFF;
        bus.update_mpr(1);

        if Self::env_force_mpr1_hardware() {
            bus.set_mpr(1, 0xFF);
        }
        // Allow overriding default pad input for BIOS waits.
        bus.io_port.input = Self::env_pad_default();
        // Optionally start timer running by default (debug aid).
        if Self::env_timer_default_start() {
            bus.timer.enabled = true;
            bus.timer.counter = bus.timer.reload;
            bus.timer.prescaler = 0;
        }

        bus
    }

    #[cfg(feature = "trace_hw_writes")]
    fn log_hw_access(kind: &str, addr: u16, value: u8) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let idx = COUNT.fetch_add(1, Ordering::Relaxed);
        if idx < 1_000_000 {
            eprintln!("{kind} {:04X} -> {:02X}", addr, value);
        }
    }

    #[inline]
    pub fn read(&mut self, addr: u16) -> u8 {
        if addr < 0x0200 && matches!(self.banks[0], BankMapping::Hardware) {
            return self.ram[addr as usize];
        }
        if matches!(self.banks[0], BankMapping::Hardware) && (addr as usize) < PAGE_SIZE {
            let offset = (addr as usize) & (PAGE_SIZE - 1);
            let value = self.read_io_internal(offset);
            #[cfg(feature = "trace_hw_writes")]
            {
                Self::log_hw_access("R", addr, value);
                if offset <= 0x0403 {
                    eprintln!("  HW read offset {:04X} -> {:02X}", offset, value);
                }
            }
            self.refresh_vdc_irq();
            return value;
        }
        if (0x2000..=0x3FFF).contains(&addr) {
            if matches!(self.banks.get(1), Some(BankMapping::Hardware))
                || Self::env_relax_io_mirror()
                || Self::env_extreme_mirror()
                || Self::env_vdc_ultra_mirror()
            {
                let offset = (addr - 0x2000) as usize;
                let value = self.read_io_internal(offset);
                #[cfg(feature = "trace_hw_writes")]
                {
                    Self::log_hw_access("R", addr, value);
                    if offset <= 0x0403 || Self::env_extreme_mirror() {
                        eprintln!("  IO read offset {:04X} -> {:02X}", offset, value);
                    }
                    if offset >= 0x1C00 && offset <= 0x1C13 {
                        eprintln!("  TIMER/IRQ read {:04X} -> {:02X}", offset, value);
                    }
                    if offset >= 0x1C60 && offset <= 0x1C63 {
                        eprintln!("  PSG ctrl read {:04X} -> {:02X}", offset, value);
                    }
                }
                self.refresh_vdc_irq();
                return value;
            }
        }
        if (0xFF00..=0xFF7F).contains(&addr) {
            let offset = HW_CPU_CTRL_BASE + (addr - 0xFF00) as usize;
            let value = self.read_io_internal(offset);
            #[cfg(feature = "trace_hw_writes")]
            Self::log_hw_access("R", addr, value);
            self.refresh_vdc_irq();
            return value;
        }
        if let Some(index) = Self::mpr_index_for_addr(addr) {
            return self.mpr[index];
        }
        let (mapping, offset) = self.resolve(addr);
        match mapping {
            BankMapping::Ram { base } => self.ram[base + offset],
            BankMapping::Rom { base } => self.rom.get(base + offset).copied().unwrap_or(0xFF),
            BankMapping::CartRam { base } => {
                self.cart_ram.get(base + offset).copied().unwrap_or(0x00)
            }
            BankMapping::Hardware => {
                let io_offset = (addr as usize) & (PAGE_SIZE - 1);
                let value = self.read_io_internal(io_offset);
                self.refresh_vdc_irq();
                #[cfg(feature = "trace_hw_writes")]
                {
                    Self::log_hw_access("R", addr, value);
                    if io_offset <= 0x0403 {
                        eprintln!("  HW read offset {:04X} -> {:02X}", io_offset, value);
                    }
                    if io_offset >= 0x1C00 && io_offset <= 0x1C13 {
                        eprintln!("  TIMER/IRQ read {:04X} -> {:02X}", io_offset, value);
                    }
                    if io_offset >= 0x1C60 && io_offset <= 0x1C63 {
                        eprintln!("  PSG ctrl read {:04X} -> {:02X}", io_offset, value);
                    }
                }
                value
            }
        }
    }

    #[inline]
    pub fn write(&mut self, addr: u16, value: u8) {
        if addr < 0x0200 && matches!(self.banks[0], BankMapping::Hardware) {
            let index = addr as usize;
            if index < self.ram.len() {
                self.ram[index] = value;
            }
            return;
        }
        // Fast path: any offset 0x0400–0x07FF within the hardware page maps to the VCE.
        // The VCE only decodes A0–A1, so all higher bits inside this 0x400 byte window
        // are mirrors.
        let mapping = self.banks[(addr as usize) >> 13];
        let mirrored = addr & 0x1FFF;
        if (matches!(mapping, BankMapping::Hardware) || Self::env_extreme_mirror())
            && (0x0400..=0x07FF).contains(&mirrored)
        {
            self.vce_port_hits = self.vce_port_hits.saturating_add(1);
            self.vce_last_port_addr = addr;
            if cfg!(debug_assertions) && self.vce_port_hits <= 8 {
                eprintln!(
                    "  VCE fastpath addr {:04X} sub {:02X} <= {:02X}",
                    addr,
                    mirrored & 0x0003,
                    value
                );
            }
            self.write_vce_port(mirrored as u16, value);
            self.refresh_vdc_irq();
            return;
        }
        // Catch-all debug: force any <0x4000 write to go to VCE ports (decode only A1..A0).
        if Self::env_vce_catchall() && (addr as usize) < 0x4000 {
            self.vce_port_hits = self.vce_port_hits.saturating_add(1);
            self.vce_last_port_addr = addr;
            self.write_vce_port(addr as u16, value);
            self.refresh_vdc_irq();
            return;
        }
        if matches!(self.banks[0], BankMapping::Hardware) && (addr as usize) < PAGE_SIZE {
            let offset = (addr as usize) & (PAGE_SIZE - 1);
            self.write_io_internal(offset, value);
        #[cfg(feature = "trace_hw_writes")]
        {
            Self::log_hw_access("W", addr, value);
            // For debug builds, only print the hottest area (0x2000-mirrored VDC/VCE)
            if offset <= 0x03FF {
                eprintln!("  HW write offset {:04X} -> {:02X}", offset, value);
            }
        }
        self.record_io_write(offset as u16);
        self.refresh_vdc_irq();
        return;
        }
        #[cfg(feature = "trace_hw_writes")]
        if (addr & 0x1FFF) >= 0x0400 && (addr & 0x1FFF) <= 0x0403 {
            eprintln!(
                "  WARN write {:04X} -> {:02X} (mapping {:?})",
                addr,
                value,
                self.banks[(addr as usize) >> 13]
            );
        }

        if (0x2000..=0x3FFF).contains(&addr) {
            if matches!(self.banks.get(1), Some(BankMapping::Hardware))
                || Self::env_relax_io_mirror()
                || Self::env_extreme_mirror()
            {
                let offset = (addr - 0x2000) as usize;
                self.write_io_internal(offset, value);
                #[cfg(feature = "trace_hw_writes")]
                {
                    // Reduce spam: only show IO writes when offset <= 0x0100 or value non-zero.
                    if offset <= 0x0100 || value != 0 || Self::env_extreme_mirror() {
                        Self::log_hw_access("W", addr, value);
                        if offset <= 0x03FF || Self::env_extreme_mirror() {
                            eprintln!("  IO write offset {:04X} -> {:02X}", offset, value);
                        }
                    }
                }
                self.record_io_write(offset as u16);
                self.refresh_vdc_irq();
                return;
            }
        }
        if (0xFF00..=0xFF7F).contains(&addr) {
            let offset = HW_CPU_CTRL_BASE + (addr - 0xFF00) as usize;
            self.write_io_internal(offset, value);
            #[cfg(feature = "trace_hw_writes")]
            Self::log_hw_access("W", addr, value);
            self.refresh_vdc_irq();
            return;
        }
        if let Some(index) = Self::mpr_index_for_addr(addr) {
            self.set_mpr(index, value);
            return;
        }
        let (mapping, offset) = self.resolve(addr);
        match mapping {
            BankMapping::Ram { base } => {
                let index = base + offset;
                if index < self.ram.len() {
                    #[cfg(feature = "trace_hw_writes")]
                    if index == 0x20 {
                        eprintln!("  ZP[20] <= {:02X}", value);
                    }
                    self.ram[index] = value;
                }
            }
            BankMapping::CartRam { base } => {
                let index = base + offset;
                if index < self.cart_ram.len() {
                    self.cart_ram[index] = value;
                }
            }
            BankMapping::Hardware => {
                let io_offset = (addr as usize) & (PAGE_SIZE - 1);
                self.write_io_internal(io_offset, value);
                #[cfg(feature = "trace_hw_writes")]
                {
                    Self::log_hw_access("W", addr, value);
                    if io_offset <= 0x0403 {
                        eprintln!("  HW write offset {:04X} -> {:02X}", io_offset, value);
                    }
                }
                self.record_io_write(io_offset as u16);
                self.refresh_vdc_irq();
            }
            BankMapping::Rom { .. } => {}
        }
    }

    /// Copy a slice into memory starting at the given address.
    pub fn load(&mut self, start: u16, data: &[u8]) {
        let mut addr = start;
        for byte in data {
            self.write(addr, *byte);
            addr = addr.wrapping_add(1);
        }
    }

    #[inline]
    pub fn read_u16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        let hi = self.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    #[inline]
    pub fn write_u16(&mut self, addr: u16, value: u16) {
        self.write(addr, (value & 0x00FF) as u8);
        self.write(addr.wrapping_add(1), (value >> 8) as u8);
    }

    pub fn clear(&mut self) {
        self.ram.fill(0);
        self.io.fill(0);
        self.io_port.reset();
        self.interrupt_disable = 0;
        self.interrupt_request = 0;
        self.timer.reset();
        self.vdc.reset();
        self.psg.reset();
        self.vce.reset();
        self.audio_phi_accumulator = 0;
        self.audio_buffer.clear();
        self.framebuffer.fill(0);
        self.frame_ready = false;
        self.cart_ram.fill(0);
        self.bg_opaque.fill(false);
        self.bg_priority.fill(false);
        self.sprite_line_counts.fill(0);
        self.vdc.clear_sprite_overflow();
        self.vce_write_count = 0;
        self.vce_data_writes = 0;
        self.vce_port_hits = 0;
        self.cram_dma_count = 0;
        self.vce_last_port_addr = 0;
        self.vce_last_control_high = 0;
        self.vdc_alias_write_counts = [0; 0x20];
        self.io_write_hist.clear();
        #[cfg(debug_assertions)]
        {
            self.debug_force_ds_after = 0;
        }
        #[cfg(feature = "trace_hw_writes")]
        {
            self.st0_lock_window = 0;
        }
    }

    /// Replace backing ROM data. Bank mappings are left untouched so the
    /// caller can decide which windows should point at the new image.
    pub fn load_rom_image(&mut self, data: Vec<u8>) {
        self.rom = data;
        for idx in 0..NUM_BANKS {
            self.update_mpr(idx);
        }
        self.io_write_hist.clear();
    }

    pub fn map_bank_to_ram(&mut self, bank: usize, page: usize) {
        if bank < NUM_BANKS {
            let pages = self.total_ram_pages();
            let page_index = if pages == 0 { 0 } else { page % pages };
            self.mpr[bank] = 0xF8u8.saturating_add(page_index as u8);
            self.update_mpr(bank);
        }
    }

    pub fn map_bank_to_rom(&mut self, bank: usize, rom_bank: usize) {
        if bank < NUM_BANKS {
            let pages = self.rom_pages();
            let page_index = if pages == 0 { 0 } else { rom_bank % pages };
            self.mpr[bank] = page_index as u8;
            self.update_mpr(bank);
        }
    }

    pub fn set_mpr(&mut self, index: usize, value: u8) {
        if index < NUM_BANKS {
            if index == 1 && Self::env_force_mpr1_hardware() {
                #[cfg(feature = "trace_hw_writes")]
                eprintln!(
                    "  MPR1 force-hardware active: ignoring write {:02X}, keeping FF",
                    value
                );
                self.mpr[1] = 0xFF;
                self.update_mpr(1);
                return;
            }
            self.mpr[index] = value;
            self.update_mpr(index);
            #[cfg(feature = "trace_hw_writes")]
            eprintln!("  MPR{index} <= {:02X} -> {:?}", value, self.banks[index]);
        }
    }

    pub fn mpr(&self, index: usize) -> u8 {
        self.mpr[index]
    }

    pub fn mpr_array(&self) -> [u8; NUM_BANKS] {
        let mut out = [0u8; NUM_BANKS];
        out.copy_from_slice(&self.mpr);
        out
    }

    pub fn rom_page_count(&self) -> usize {
        self.rom.len() / PAGE_SIZE
    }

    pub fn write_st_port(&mut self, port: usize, value: u8) {
        let slot_index = port.min(self.st_ports.len().saturating_sub(1));
        if let Some(slot) = self.st_ports.get_mut(slot_index) {
            *slot = value;
        }
        #[cfg(feature = "trace_hw_writes")]
        if Self::env_trace_mpr() && self.vce_port_hits < 1000 {
            use std::fmt::Write as _;
            let mut m = String::new();
            for (i, val) in self.mpr.iter().enumerate() {
                let _ = write!(m, "{}:{:02X} ", i, val);
            }
            eprintln!(
                "  TRACE MPR pc={:04X} st{}={:02X} mpr={}",
                self.last_pc_for_trace.unwrap_or(0),
                port,
                value,
                m.trim_end()
            );
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  ST{port} <= {:02X} (addr={:04X})",
            value, self.vdc.last_io_addr
        );
        match port {
            0 => {
                #[cfg(feature = "trace_hw_writes")]
                if !Self::st0_hold_enabled() {
                    self.vdc.st0_hold_counter = 0;
                }
                #[cfg(feature = "trace_hw_writes")]
                if self.vdc.st0_hold_counter > 0 {
                    // Mirror spam often re-writes 0 to ST0 immediately after a data byte.
                    // Ignore those redundant zeros, but allow a non-zero selector to punch
                    // through even while the hold is active.
                    if value == self.vdc.selected_register() {
                        self.vdc.st0_hold_counter = self.vdc.st0_hold_counter.saturating_sub(1);
                        let idx = (self.vdc.last_io_addr as usize) & 0xFF;
                        if let Some(slot) = self.vdc.st0_hold_addr_hist.get_mut(idx) {
                            *slot = slot.saturating_add(1);
                        }
                        eprintln!(
                            "  ST0 ignored (hold) pending={:?} phase={:?} value={:02X}",
                            self.vdc.pending_write_register, self.vdc.write_phase, value
                        );
                        return;
                    }
                    // Let the new selection proceed; clear the hold so the register change
                    // isn't dropped.
                    self.vdc.st0_hold_counter = 0;
                }
                self.vdc.write_port(0, value)
            }
            1 => {
                #[cfg(feature = "trace_hw_writes")]
                {
                    if Self::st0_hold_enabled() {
                        const HOLD_SPAN: u8 = 8;
                        self.vdc.st0_hold_counter = HOLD_SPAN;
                    } else {
                        self.vdc.st0_hold_counter = 0;
                    }
                }
                self.vdc.write_port(1, value)
            }
            2 => {
                #[cfg(feature = "trace_hw_writes")]
                {
                    if Self::st0_hold_enabled() {
                        const HOLD_SPAN: u8 = 8;
                        self.vdc.st0_hold_counter = HOLD_SPAN;
                    } else {
                        self.vdc.st0_hold_counter = 0;
                    }
                }
                self.vdc.write_port(2, value)
            }
            _ => {}
        }
        #[cfg(feature = "trace_hw_writes")]
        if port == 0 && value == 0x05 {
            self.vdc.pending_traced_register = Some(0x05);
            #[cfg(feature = "trace_hw_writes")]
            eprintln!("  TRACE select R05");
        }
        #[cfg(feature = "trace_hw_writes")]
        if matches!(port, 1 | 2) {
            if let Some(sel) = self.vdc.pending_traced_register.take() {
                #[cfg(feature = "trace_hw_writes")]
                {
                    use std::fmt::Write as _;
                    let mut mpr_buf = String::new();
                    for (i, m) in self.mpr.iter().enumerate() {
                        if i > 0 {
                            mpr_buf.push(' ');
                        }
                        let _ = write!(mpr_buf, "{:02X}", m);
                    }
                    eprintln!(
                        "  TRACE R{:02X} data via ST{} = {:02X} (selected={:02X} pc={:04X} mpr={})",
                        sel,
                        port,
                        value,
                        self.vdc.selected_register(),
                        self.last_pc_for_trace.unwrap_or(0),
                        mpr_buf
                    );
                }
            }
        }
        if let Some(mask) = self.vdc.take_dcr_request() {
            self.handle_vdc_dcr(mask);
        }
        self.refresh_vdc_irq();
    }

    pub fn read_st_port(&mut self, port: usize) -> u8 {
        let value = match port {
            0 => self.vdc.selected_register(),
            1 => self.vdc.read_port(1),
            2 => self.vdc.read_port(2),
            _ => 0,
        };
        let slot_index = port.min(self.st_ports.len().saturating_sub(1));
        if let Some(slot) = self.st_ports.get_mut(slot_index) {
            *slot = value;
        }
        self.refresh_vdc_irq();
        value
    }

    pub fn st_port(&self, port: usize) -> u8 {
        self.st_ports.get(port).copied().unwrap_or(0)
    }

    pub fn vdc_register(&self, index: usize) -> Option<u16> {
        self.vdc.register(index)
    }

    pub fn vdc_status_bits(&self) -> u8 {
        self.vdc.status_bits()
    }

    pub fn vdc_map_dimensions(&self) -> (usize, usize) {
        self.vdc.map_dimensions()
    }

    #[cfg(test)]
    pub fn vdc_vram_word(&self, addr: u16) -> u16 {
        let idx = (addr as usize) & 0x7FFF;
        *self.vdc.vram.get(idx).unwrap_or(&0)
    }

    #[cfg(test)]
    pub fn vdc_satb_word(&self, index: usize) -> u16 {
        self.vdc.satb.get(index).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn sprite_line_counts_for_test(&self) -> &[u8] {
        &self.sprite_line_counts
    }

    #[cfg(test)]
    pub fn vce_palette_word(&self, index: usize) -> u16 {
        self.vce.palette_word(index)
    }

    pub fn vce_palette_rgb(&self, index: usize) -> u32 {
        self.vce.palette_rgb(index)
    }

    #[cfg(test)]
    pub fn vdc_set_status_for_test(&mut self, mask: u8) {
        self.vdc.raise_status(mask);
        self.refresh_vdc_irq();
    }

    pub fn read_io(&mut self, offset: usize) -> u8 {
        let value = self.read_io_internal(offset);
        self.refresh_vdc_irq();
        value
    }

    pub fn write_io(&mut self, offset: usize, value: u8) {
        self.write_io_internal(offset, value);
        self.refresh_vdc_irq();
    }

    pub fn tick(&mut self, cycles: u32, high_speed: bool) -> bool {
        let phi_cycles = if high_speed {
            cycles
        } else {
            cycles.saturating_mul(4)
        };

        // Debug: force timer expiry to drive IRQ2 if requested.
        if std::env::var("PCE_FORCE_TIMER").is_ok() {
            self.timer.counter = 0;
            self.interrupt_request |= IRQ_REQUEST_TIMER;
        }

        if self.vdc.tick(phi_cycles) {
            self.refresh_vdc_irq();
        }

        if self.vdc.in_vblank && self.vdc.cram_pending {
            self.perform_cram_dma();
            self.refresh_vdc_irq();
        }

        if self.vdc.frame_ready() {
            self.render_frame_from_vram();
        }

        if self.timer.tick(cycles, high_speed) {
            self.interrupt_request |= IRQ_REQUEST_TIMER;
        }

        if self.psg.tick(cycles) {
            self.raise_irq(IRQ_REQUEST_IRQ2);
        }

        self.enqueue_audio_samples(phi_cycles);

        self.refresh_vdc_irq();

        self.irq_pending()
    }

    #[cfg(feature = "trace_hw_writes")]
    pub fn set_last_pc_for_trace(&mut self, pc: u16) {
        self.last_pc_for_trace = Some(pc);
    }

    pub fn psg_sample(&mut self) -> i16 {
        self.psg.generate_sample()
    }

    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.audio_buffer)
    }

    pub fn take_frame(&mut self) -> Option<Vec<u32>> {
        if !self.frame_ready {
            return None;
        }
        self.frame_ready = false;
        Some(self.framebuffer.clone())
    }

    fn record_io_write(&mut self, offset: u16) {
        use std::collections::hash_map::Entry;
        match self.io_write_hist.entry(offset) {
            Entry::Vacant(v) => {
                v.insert(1);
            }
            Entry::Occupied(mut o) => {
                *o.get_mut() = o.get().saturating_add(1);
            }
        }
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }

    pub fn io_write_hist_top(&self, limit: usize) -> Vec<(u16, u64)> {
        let mut entries: Vec<(u16, u64)> =
            self.io_write_hist.iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(limit);
        entries
    }

    pub fn vce_write_count(&self) -> u64 {
        self.vce_write_count
    }

    pub fn vce_data_write_count(&self) -> u64 {
        self.vce_data_writes
    }

    pub fn vce_control_write_count(&self) -> u64 {
        self.vce_control_writes
    }

    pub fn vce_port_hit_count(&self) -> u64 {
        self.vce_port_hits
    }

    pub fn cram_dma_count(&self) -> u64 {
        self.vdc.cram_dma_count
    }

    pub fn vce_last_port_addr(&self) -> u16 {
        self.vce_last_port_addr
    }

    pub fn vce_last_control_high(&self) -> u8 {
        self.vce_last_control_high
    }

    pub fn vce_last_control_high_max(&self) -> u8 {
        self.vce_last_control_high_max
    }

    pub fn vdc_alias_write_counts(&self) -> &[u64; 0x20] {
        &self.vdc_alias_write_counts
    }

    pub fn vdc_r05_low_writes(&self) -> u64 {
        self.vdc.r05_low_writes()
    }

    pub fn vdc_r05_high_writes(&self) -> u64 {
        self.vdc.r05_high_writes()
    }

    pub fn vdc_last_r05_low(&self) -> u8 {
        self.vdc.last_r05_low()
    }

    pub fn vdc_control_write_count(&self) -> u64 {
        self.vdc.control_write_count()
    }

    pub fn vdc_last_control(&self) -> u16 {
        self.vdc.last_control_value()
    }

    pub fn vdc_satb_pending(&self) -> bool {
        self.vdc.satb_pending()
    }

    pub fn vdc_satb_source(&self) -> u16 {
        self.vdc.satb_source()
    }

    pub fn vdc_cram_last_source(&self) -> u16 {
        self.vdc.last_cram_source
    }

    pub fn vdc_cram_last_length(&self) -> u16 {
        self.vdc.last_cram_length
    }

    pub fn vdc_vram_dma_count(&self) -> u64 {
        self.vdc.vram_dma_count
    }

    pub fn vdc_vram_last_source(&self) -> u16 {
        self.vdc.last_vram_dma_source
    }

    pub fn vdc_vram_last_destination(&self) -> u16 {
        self.vdc.last_vram_dma_destination
    }

    pub fn vdc_vram_last_length(&self) -> u16 {
        self.vdc.last_vram_dma_length
    }

    pub fn vdc_register_write_count(&self, index: usize) -> u64 {
        self.vdc.register_write_count(index)
    }

    pub fn vdc_register_write_counts(&self) -> &[u64; VDC_REGISTER_COUNT] {
        &self.vdc.register_write_counts
    }

    pub fn vdc_register_select_count(&self, index: usize) -> u64 {
        self.vdc.register_select_count(index)
    }

    pub fn vdc_register_select_counts(&self) -> &[u64; VDC_REGISTER_COUNT] {
        &self.vdc.register_select_counts
    }

    pub fn vdc_dcr_write_count(&self) -> u64 {
        self.vdc.dcr_write_count
    }

    pub fn vdc_last_dcr_value(&self) -> u8 {
        self.vdc.last_dcr_value
    }

    pub fn configure_cart_ram(&mut self, size: usize) {
        if size == 0 {
            self.cart_ram.clear();
        } else if self.cart_ram.len() != size {
            self.cart_ram = vec![0; size];
        } else {
            self.cart_ram.fill(0);
        }
        for idx in 0..NUM_BANKS {
            self.update_mpr(idx);
        }
    }

    pub fn cart_ram_size(&self) -> usize {
        self.cart_ram.len()
    }

    pub fn set_joypad_input(&mut self, state: u8) {
        self.io_port.input = state;
    }

    pub fn cart_ram(&self) -> Option<&[u8]> {
        if self.cart_ram.is_empty() {
            None
        } else {
            Some(&self.cart_ram)
        }
    }

    pub fn cart_ram_mut(&mut self) -> Option<&mut [u8]> {
        if self.cart_ram.is_empty() {
            None
        } else {
            Some(&mut self.cart_ram)
        }
    }

    pub fn load_cart_ram(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if self.cart_ram.is_empty() {
            return Err("cart RAM not present");
        }
        if self.cart_ram.len() != data.len() {
            return Err("cart RAM size mismatch");
        }
        self.cart_ram.copy_from_slice(data);
        Ok(())
    }

    fn read_control_register(&mut self, offset: usize) -> Option<u8> {
        match Self::decode_control_register(offset)? {
            ControlRegister::TimerCounter => Some(self.timer.read_counter()),
            ControlRegister::TimerControl => Some(self.timer.control()),
            ControlRegister::IrqMask => Some(self.interrupt_disable),
            ControlRegister::IrqStatus => {
                if let Some(force) = Self::env_irq_status_default() {
                    Some(self.interrupt_request | force)
                } else {
                    Some(self.interrupt_request)
                }
            }
        }
    }

    fn write_control_register(&mut self, offset: usize, value: u8) -> bool {
        match Self::decode_control_register(offset) {
            Some(ControlRegister::TimerCounter) => {
                self.timer.write_reload(value);
                true
            }
            Some(ControlRegister::TimerControl) => {
                self.timer.write_control(value);
                true
            }
            Some(ControlRegister::IrqMask) => {
                let mask = IRQ_DISABLE_IRQ2 | IRQ_DISABLE_IRQ1 | IRQ_DISABLE_TIMER;
                self.interrupt_disable = value & mask;
                true
            }
            Some(ControlRegister::IrqStatus) => {
                self.interrupt_request &= !value;
                true
            }
            None => false,
        }
    }

    fn decode_control_register(offset: usize) -> Option<ControlRegister> {
        if (HW_IRQ_BASE..=HW_IRQ_BASE + 0x03FF).contains(&offset) {
            match offset & 0x03 {
                0x00 => Some(ControlRegister::TimerCounter),
                0x01 => Some(ControlRegister::TimerControl),
                0x02 => Some(ControlRegister::IrqMask),
                0x03 => Some(ControlRegister::IrqStatus),
                _ => None,
            }
        } else if (HW_CPU_CTRL_BASE..=HW_CPU_CTRL_BASE + 0x03FF).contains(&offset) {
            match offset & 0xFF {
                0x10 => Some(ControlRegister::TimerCounter),
                0x11 => Some(ControlRegister::TimerControl),
                0x12 => Some(ControlRegister::IrqMask),
                0x13 => Some(ControlRegister::IrqStatus),
                _ => None,
            }
        } else {
            None
        }
    }

    fn mpr_index_for_addr(addr: u16) -> Option<usize> {
        if !(0xFF80..=0xFFBF).contains(&addr) {
            return None;
        }
        let offset = (addr - 0xFF80) as usize;
        Some(offset & 0x07)
    }

    fn enqueue_audio_samples(&mut self, phi_cycles: u32) {
        self.audio_phi_accumulator = self.audio_phi_accumulator.saturating_add(phi_cycles);
        while self.audio_phi_accumulator >= PHI_CYCLES_PER_SAMPLE {
            self.audio_phi_accumulator -= PHI_CYCLES_PER_SAMPLE;
            let sample = self.psg.generate_sample();
            self.audio_buffer.push(sample);
        }
    }

    fn render_frame_from_vram(&mut self) {
        self.vdc.clear_frame_trigger();
        let ctrl = self.vdc.control();
        let display_on = (ctrl & 0x8000) != 0 || Self::env_force_display_on();
        let sprites_enabled = (ctrl & (VDC_CTRL_ENABLE_SPRITES | VDC_CTRL_ENABLE_SPRITES_LEGACY))
            != 0
            || (display_on && (ctrl & (0x0200 | 0x0400)) != 0);
        let background_enabled =
            (ctrl & (VDC_CTRL_ENABLE_BACKGROUND | VDC_CTRL_ENABLE_BACKGROUND_LEGACY)) != 0
                || (display_on && (ctrl & (0x0800 | 0x1000)) != 0);
        if !sprites_enabled && !background_enabled {
            let colour = self.vce.palette_rgb(0);
            self.framebuffer.fill(colour);
            self.frame_ready = true;
            return;
        }

        if self.vdc.vram.is_empty() {
            self.framebuffer.fill(self.vce.palette_rgb(0));
            self.frame_ready = true;
            return;
        }

        #[derive(Clone, Copy, Default)]
        struct TileSample {
            chr0: u16,
            chr1: u16,
            palette_base: usize,
            h_flip: bool,
            priority: bool,
        }

        self.bg_opaque.fill(false);
        self.bg_priority.fill(false);
        for count in self.sprite_line_counts.iter_mut() {
            *count = 0;
        }
        self.vdc.clear_sprite_overflow();

        let background_colour = self.vce.palette_rgb(0);
        if Self::env_force_test_palette() {
            // デバッグ: パレットを簡易グラデーションに初期化
            for i in 0..self.vce.palette.len() {
                let v = i as u16;
                if let Some(slot) = self.vce.palette.get_mut(i) {
                    *slot = ((v & 0x0F) << 8) | ((v >> 4) & 0x0F) << 4 | (v & 0x0F);
                }
            }
        }
        if Self::env_force_palette_every_frame() {
            for i in 0..self.vce.palette.len() {
                let v = (i as u16) & 0x3FF;
                if let Some(slot) = self.vce.palette.get_mut(i) {
                    *slot = ((v & 0x0F) << 8) | (((v >> 4) & 0x0F) << 4) | (v & 0x0F);
                }
            }
        }
        if background_enabled {
            let mut tile_cache: Vec<TileSample> =
                Vec::with_capacity((FRAME_WIDTH / TILE_WIDTH) + 2);
            let (map_width_tiles, map_height_tiles) = self.vdc.map_dimensions();
            let map_width = map_width_tiles.max(1);
            let map_height = map_height_tiles.max(1);
            let mwr = self.vdc.registers[0x09] as usize;
            let cg_mode_bit = (mwr >> 7) & 0x01;
            let pixel_width_mode = mwr & 0x03;
            let restrict_planes = pixel_width_mode == 0x03;
            let vram_mask = self.vdc.vram.len().saturating_sub(1);

            for y in 0..FRAME_HEIGHT {
                if Self::env_force_test_palette() {
                    // パレットを毎行クリアして強制表示色を維持
                    for i in 0..self.vce.palette.len() {
                        let v = i as u16;
                        if let Some(slot) = self.vce.palette.get_mut(i) {
                            *slot = ((v & 0x0F) << 8) | (((v >> 4) & 0x0F) << 4) | (v & 0x0F);
                        }
                    }
                }
                let (x_scroll, y_scroll) = self.vdc.scroll_values_for_line(y);
                let (zoom_x_raw, zoom_y_raw) = self.vdc.zoom_values_for_line(y);
                let step_x = Vdc::zoom_step_value(zoom_x_raw);
                let step_y = Vdc::zoom_step_value(zoom_y_raw);
                let vram = &self.vdc.vram;
                let start_x_fp = (x_scroll as usize) << 4;
                let sample_y_fp = ((y_scroll as usize) << 4) + step_y * y;
                let sample_y = sample_y_fp >> 4;
                let tile_row = (sample_y / TILE_HEIGHT) % map_height;
                let line_in_tile = (sample_y % TILE_HEIGHT) as usize;
                let start_sample_x = start_x_fp >> 4;
                let start_tile_int = start_sample_x / TILE_WIDTH;
                let end_sample_x_fp = start_x_fp + step_x * (FRAME_WIDTH - 1);
                let end_sample_x = (end_sample_x_fp >> 4) + 1;
                let end_tile_int = (end_sample_x + TILE_WIDTH - 1) / TILE_WIDTH;
                let mut tiles_needed = end_tile_int.saturating_sub(start_tile_int) + 2;
                tiles_needed = tiles_needed.max(1);

                tile_cache.clear();
                tile_cache.reserve(tiles_needed);

                for tile_offset in 0..tiles_needed {
                    let tile_col = (start_tile_int + tile_offset) % map_width;
                    let map_addr = self.vdc.map_entry_address(tile_row, tile_col);
                    let tile_entry = vram.get(map_addr & vram_mask).copied().unwrap_or(0);
                    let tile_id = (tile_entry & 0x03FF) as usize;
                    let palette_bank = ((tile_entry >> 12) & 0x0F) as usize;
                    let h_flip = (tile_entry & 0x0400) != 0;
                    let v_flip = (tile_entry & 0x0800) != 0;
                    let tile_base = (tile_id * 16) & vram_mask;
                    let row_index = if v_flip {
                        TILE_HEIGHT - 1 - line_in_tile
                    } else {
                        line_in_tile
                    };
                    let row_addr = (tile_base + row_index) & vram_mask;
                    let mut chr0 = vram.get(row_addr).copied().unwrap_or(0);
                    let mut chr1 = vram.get((row_addr + 8) & vram_mask).copied().unwrap_or(0);
                    if restrict_planes {
                        if cg_mode_bit == 0 {
                            chr1 = 0;
                        } else {
                            chr0 = 0;
                        }
                    }
                    tile_cache.push(TileSample {
                        chr0,
                        chr1,
                        palette_base: (palette_bank << 4) & 0x1F0,
                        h_flip,
                        priority: (tile_entry & 0x8000) != 0,
                    });
                }

                let mut sample_x_fp = start_x_fp;
                let start_tile_int = start_tile_int;
                for x in 0..FRAME_WIDTH {
                    let screen_index = y * FRAME_WIDTH + x;
                    let sample_x = (sample_x_fp >> 4) as usize;
                    let tile_idx_int = sample_x / TILE_WIDTH;
                    let tile_offset = tile_idx_int.saturating_sub(start_tile_int);
                    let sample = tile_cache.get(tile_offset).copied().unwrap_or_default();
                    let intra_tile_x = sample_x % TILE_WIDTH;
                    let bit_index = if sample.h_flip {
                        TILE_WIDTH - 1 - intra_tile_x
                    } else {
                        intra_tile_x
                    };
                    let shift = 7 - bit_index;
                    let plane0 = ((sample.chr0 >> shift) & 0x01) as u8;
                    let plane1 = ((sample.chr0 >> (shift + 8)) & 0x01) as u8;
                    let plane2 = ((sample.chr1 >> shift) & 0x01) as u8;
                    let plane3 = ((sample.chr1 >> (shift + 8)) & 0x01) as u8;
                    let pixel = plane0 | (plane1 << 1) | (plane2 << 2) | (plane3 << 3);
                    if pixel == 0 {
                        self.framebuffer[screen_index] = background_colour;
                    } else {
                        self.bg_opaque[screen_index] = true;
                        self.bg_priority[screen_index] = sample.priority;
                        let colour_idx = (sample.palette_base | pixel as usize) & 0x1FF;
                        self.framebuffer[screen_index] = self.vce.palette_rgb(colour_idx);
                    }
                    sample_x_fp += step_x;
                }
            }
        } else {
            self.framebuffer.fill(background_colour);
        }
        if sprites_enabled {
            self.render_sprites();
        }
        self.frame_ready = true;
    }

    fn render_sprites(&mut self) {
        if self.vdc.vram.is_empty() {
            return;
        }
        let vram = &self.vdc.vram;
        let vram_mask = vram.len().saturating_sub(1);
        let mut overflow_detected = false;

        for sprite in (0..SPRITE_COUNT).rev() {
            let base = sprite * 4;
            let y_word = self.vdc.satb.get(base).copied().unwrap_or(0);
            let tile_word = self.vdc.satb.get(base + 1).copied().unwrap_or(0);
            let attr_word = self.vdc.satb.get(base + 2).copied().unwrap_or(0);
            let x_word = self.vdc.satb.get(base + 3).copied().unwrap_or(0);

            let y = (y_word & 0x03FF) as i32 - 64;
            let x = (x_word & 0x03FF) as i32 - 32;
            let width_cells = if (attr_word & 0x0100) != 0 { 2 } else { 1 };
            let height_code = ((y_word >> 12) & 0x03) as usize;
            let height_cells = match height_code {
                0 => 1,
                1 => 2,
                2 => 3,
                _ => 4,
            };
            let tiles_wide = width_cells * 2;
            let tiles_high = height_cells * 2;
            let sprite_width = (tiles_wide * TILE_WIDTH) as i32;
            let sprite_height = (tiles_high * TILE_HEIGHT) as i32;

            if x >= FRAME_WIDTH as i32 || y >= FRAME_HEIGHT as i32 {
                continue;
            }
            if x + sprite_width <= 0 || y + sprite_height <= 0 {
                continue;
            }

            let tile_base_index = (tile_word & 0x07FF) as usize;
            let palette_base = ((attr_word & 0x000F) as usize) << 4;
            let sprite_behind_bg = (attr_word & 0x0080) != 0;
            let h_flip = (attr_word & 0x0400) != 0;
            let v_flip = (attr_word & 0x0200) != 0;

            for tile_y in 0..tiles_high {
                let row_base = y + (tile_y * TILE_HEIGHT) as i32;
                if row_base >= FRAME_HEIGHT as i32 || row_base + TILE_HEIGHT as i32 <= 0 {
                    continue;
                }
                let src_tile_y = if v_flip {
                    tiles_high - 1 - tile_y
                } else {
                    tile_y
                };

                for row in 0..TILE_HEIGHT {
                    let dest_y = row_base + row as i32;
                    if dest_y < 0 || dest_y >= FRAME_HEIGHT as i32 {
                        continue;
                    }

                    let dest_row = dest_y as usize;
                    let count = &mut self.sprite_line_counts[dest_row];
                    if *count >= 16 {
                        overflow_detected = true;
                        continue;
                    }
                    if *count < u8::MAX {
                        *count += 1;
                    }

                    let sample_row = if v_flip { TILE_HEIGHT - 1 - row } else { row };

                    for tile_x in 0..tiles_wide {
                        let col_base = x + (tile_x * TILE_WIDTH) as i32;
                        if col_base >= FRAME_WIDTH as i32 || col_base + TILE_WIDTH as i32 <= 0 {
                            continue;
                        }
                        let src_tile_x = if h_flip {
                            tiles_wide - 1 - tile_x
                        } else {
                            tile_x
                        };
                        let tile_index = tile_base_index + src_tile_y * tiles_wide + src_tile_x;
                        let tile_base = (tile_index * 16) & vram_mask;
                        let chr0 = vram[(tile_base + sample_row) & vram_mask];
                        let chr1 = vram[(tile_base + sample_row + 8) & vram_mask];

                        for col in 0..TILE_WIDTH {
                            let dest_x = col_base + col as i32;
                            if dest_x < 0 || dest_x >= FRAME_WIDTH as i32 {
                                continue;
                            }
                            let sample_col = if h_flip { TILE_WIDTH - 1 - col } else { col };
                            let shift = 7 - sample_col;
                            let plane0 = ((chr0 >> shift) & 0x01) as u8;
                            let plane1 = ((chr0 >> (shift + 8)) & 0x01) as u8;
                            let plane2 = ((chr1 >> shift) & 0x01) as u8;
                            let plane3 = ((chr1 >> (shift + 8)) & 0x01) as u8;
                            let pixel = plane0 | (plane1 << 1) | (plane2 << 2) | (plane3 << 3);
                            if pixel == 0 {
                                continue;
                            }
                            let offset = dest_row * FRAME_WIDTH + dest_x as usize;
                            if sprite_behind_bg && self.bg_opaque[offset] {
                                continue;
                            }
                            if self.bg_priority[offset] {
                                continue;
                            }
                            let colour_index = (palette_base | pixel as usize) & 0x1FF;
                            self.framebuffer[offset] = self.vce.palette_rgb(colour_index);
                        }
                    }
                }
            }
        }

        if overflow_detected {
            self.vdc.raise_status(VDC_STATUS_OR);
        }
    }
    pub fn irq_pending(&self) -> bool {
        (self.interrupt_request & self.enabled_irq_mask()) != 0
    }

    pub fn pending_interrupts(&self) -> u8 {
        self.interrupt_request & self.enabled_irq_mask()
    }

    pub fn raise_irq(&mut self, mask: u8) {
        self.interrupt_request |= mask;
    }

    pub fn clear_irq(&mut self, mask: u8) {
        self.interrupt_request &= !mask;
    }

    pub fn acknowledge_irq(&mut self, mask: u8) {
        self.clear_irq(mask);
        if mask & IRQ_REQUEST_IRQ2 != 0 {
            self.psg.acknowledge();
        }
    }

    pub fn next_irq(&self) -> Option<u8> {
        let masked = self.pending_interrupts();
        if masked & IRQ_REQUEST_TIMER != 0 {
            return Some(IRQ_REQUEST_TIMER);
        }
        if masked & IRQ_REQUEST_IRQ1 != 0 {
            return Some(IRQ_REQUEST_IRQ1);
        }
        if masked & IRQ_REQUEST_IRQ2 != 0 {
            return Some(IRQ_REQUEST_IRQ2);
        }
        None
    }

    fn resolve(&self, addr: u16) -> (BankMapping, usize) {
        let index = (addr as usize) >> 13;
        let offset = (addr as usize) & (PAGE_SIZE - 1);
        (self.banks[index], offset)
    }

    fn update_mpr(&mut self, bank: usize) {
        let value = self.mpr[bank];
        let rom_pages = self.rom_pages();
        let cart_pages = self.cart_ram_pages();
        let mapping = match value {
            0xFF => BankMapping::Hardware,
            0xF8..=0xFD => {
                let ram_pages = self.total_ram_pages().max(1);
                let logical = (value - 0xF8) as usize % ram_pages;
                BankMapping::Ram {
                    base: logical * PAGE_SIZE,
                }
            }
            _ => {
                let logical = value as usize;
                if cart_pages > 0 && value >= 0x80 {
                    let cart_page = (logical - 0x80) % cart_pages.max(1);
                    BankMapping::CartRam {
                        base: cart_page * PAGE_SIZE,
                    }
                } else if rom_pages > 0 {
                    let rom_page = logical % rom_pages;
                    BankMapping::Rom {
                        base: rom_page * PAGE_SIZE,
                    }
                } else {
                    BankMapping::Ram { base: 0 }
                }
            }
        };
        let mapping = if bank == 1 && Self::env_force_mpr1_hardware() {
            BankMapping::Hardware
        } else {
            mapping
        };
        self.banks[bank] = mapping;
    }

    fn total_ram_pages(&self) -> usize {
        (self.ram.len() / PAGE_SIZE).max(1)
    }

    fn rom_pages(&self) -> usize {
        self.rom.len() / PAGE_SIZE
    }

    fn cart_ram_pages(&self) -> usize {
        self.cart_ram.len() / PAGE_SIZE
    }

    fn vdc_port_kind(offset: usize) -> Option<VdcPort> {
        // VDC is mirrored over the 0x0000–0x03FF IO window. Only A1..A0 select
        // control/data; A2+ are ignored by the chip. Many HuCARDs stream writes
        // via 0x2002/0x2003/0x200A/0x200B, so ensure any offset whose low two
        // bits are 0/1 goes to Control, 2/3 goes to Data.
        // For debug `PCE_VDC_ULTRA_MIRROR`, widen to the entire hardware page.
        let mirrored = offset & 0x1FFF;
        let ultra = Self::env_vdc_ultra_mirror();
        let catchall = Self::env_vdc_catchall();
        if !catchall {
            if !Self::env_extreme_mirror() && !ultra && mirrored >= 0x0400 {
                return None;
            }
            if Self::env_extreme_mirror() && !ultra && mirrored >= 0x1000 {
                return None;
            }
            if ultra && mirrored >= 0x2000 {
                return None;
            }
        }
        match mirrored & 0x03 {
            0x00 | 0x01 => Some(VdcPort::Control),
            0x02 | 0x03 => Some(VdcPort::Data),
            _ => None,
        }
    }

    #[cfg(feature = "trace_hw_writes")]
    fn st0_hold_enabled() -> bool {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var("PCE_TRACE_DISABLE_ST0_HOLD").is_err())
    }

    fn env_route_02xx_hw() -> bool {
        use std::sync::OnceLock;
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| match std::env::var("PCE_ROUTE_02XX_HW") {
            Ok(v) if v == "0" => false,
            _ => true, // default: route 0x0200–0x021F to hardware
        })
    }

    fn normalized_io_offset(offset: usize) -> usize {
        // Optional: fold 0x0200–0x03FF down to 0x0000–0x01FF when debugging
        // HuCARDs that stream hardware writes through the wider mirror region.
        if Self::env_fold_io_02xx() && offset >= 0x0200 && offset < 0x0400 {
            offset & 0x01FF
        } else {
            offset
        }
    }

    fn read_io_internal(&mut self, raw_offset: usize) -> u8 {
        // The HuC6280 only decodes A0–A10 for the hardware page; fold everything
        // into 0x0000–0x1FFF first, then optional 0x0200 folding for debug.
        let mut offset = raw_offset & 0x1FFF;
        offset = Self::normalized_io_offset(offset);
        if Self::env_route_02xx_hw() && offset >= 0x0200 && offset < 0x0220 {
            offset &= 0x01FF; // map 0x0200–0x021F to 0x0000–0x001F
        }
        if let Some(port) = Self::vdc_port_kind(offset) {
            #[cfg(feature = "trace_hw_writes")]
            {
                self.vdc.last_io_addr = offset as u16;
            }
            return match port {
                VdcPort::Control => self.vdc.read_status(),
                VdcPort::Data => {
                    let port_index = if offset & 0x01 != 0 { 2 } else { 1 };
                    self.vdc.read_port(port_index)
                }
            };
        }
        match offset {
            0x0400..=0x07FF | 0x1C40..=0x1C47 => {
                let sub = offset & 0x0003;
                let mirror_bit2 = (offset & 0x0004) != 0;
                let swap = Self::vce_ports_swapped();
                let mirror_route = Self::env_vce_route_mirror_as_data();
                let mut is_data = if swap {
                    (sub & 0x02) == 0
                } else {
                    (sub & 0x02) != 0
                };
                if !is_data && mirror_bit2 && mirror_route {
                    is_data = true;
                }
                let is_high = (sub & 0x01) != 0;
                if is_data {
                    if is_high {
                        self.vce.read_data_high()
                    } else {
                        self.vce.read_data_low()
                    }
                } else if is_high {
                    self.vce.read_control_high()
                } else {
                    self.vce.read_control_low()
                }
            }
            0x0800..=0x0BFF | 0x1C60..=0x1C63 => match offset & 0x03 {
                0x00 => self.psg.read_address(),
                0x01 => self.io[offset],
                0x02 => self.psg.read_data(),
                _ => self.psg.read_status(),
            },
            0x1000..=0x13FF => {
                if let Some(value) = self.io_port.read(offset - HW_JOYPAD_BASE) {
                    value
                } else {
                    self.io[offset]
                }
            }
            0x1400..=0x17FF | 0x1C10..=0x1C13 => {
                if let Some(value) = self.read_control_register(offset) {
                    value
                } else {
                    self.io[offset]
                }
            }
            0x1C00..=0x1FFF => {
                if let Some(value) = self.read_control_register(offset) {
                    value
                } else {
                    self.io[offset]
                }
            }
            _ => self.io[offset],
        }
    }

    #[inline]
    pub fn stack_read(&self, addr: u16) -> u8 {
        let index = addr as usize;
        self.ram.get(index).copied().unwrap_or(0)
    }

    #[inline]
    pub fn stack_write(&mut self, addr: u16, value: u8) {
        let index = addr as usize;
        if let Some(slot) = self.ram.get_mut(index) {
            *slot = value;
        }
    }

    #[inline]
    pub fn read_zero_page(&self, addr: u8) -> u8 {
        self.ram.get(addr as usize).copied().unwrap_or(0)
    }

    #[inline]
    pub fn write_zero_page(&mut self, addr: u8, value: u8) {
        if let Some(slot) = self.ram.get_mut(addr as usize) {
            #[cfg(feature = "trace_hw_writes")]
            if (0x20..=0x23).contains(&addr) {
                eprintln!("  ZP[{addr:02X}] (zp) <= {value:02X}");
            }
            *slot = value;
        }
    }

    fn write_io_internal(&mut self, raw_offset: usize, value: u8) {
        // Fold to 0x0000–0x1FFF to mirror HuC6280 hardware page decode.
        let mut offset = raw_offset & 0x1FFF;
        offset = Self::normalized_io_offset(offset);
        if Self::env_route_02xx_hw() && offset >= 0x0200 && offset < 0x0220 {
            offset &= 0x01FF; // map 0x0200–0x021F to 0x0000–0x001F
        }
        if let Some(port) = Self::vdc_port_kind(offset) {
            #[cfg(feature = "trace_hw_writes")]
            {
                self.vdc.last_io_addr = offset as u16;
            }
            let slot = offset & 0x1F;
            if let Some(entry) = self.vdc_alias_write_counts.get_mut(slot) {
                *entry = entry.saturating_add(1);
            }
            match port {
                VdcPort::Control => self.write_st_port(0, value),
                VdcPort::Data => {
                    let port_index = if offset & 0x01 != 0 { 2 } else { 1 };
                    self.write_st_port(port_index, value)
                }
            }
            return;
        }
        #[cfg(feature = "trace_hw_writes")]
        if (offset & 0x1FFF) >= 0x2400 && (offset & 0x1FFF) < 0x2800 {
            eprintln!(
                "  IO write HIGH mirror offset {:04X} -> {:02X}",
                offset, value
            );
        }
        #[cfg(feature = "trace_hw_writes")]
        if (offset & 0xE000) == 0 && value != 0 {
            eprintln!("  HW page data write {:04X} -> {:02X}", offset, value);
        }
        match offset {
            // VCE mirrors also appear at 0x1C40–0x1C43 in some docs; treat them the same.
            0x0400..=0x07FF | 0x1C40..=0x1C47 => {
                let sub = (offset & 0x0003) as u16;
                self.write_vce_port(sub, value);
            }
            // PSG mirrors at 0x1C60–0x1C63.
            0x0800..=0x0BFF | 0x1C60..=0x1C63 => match offset & 0x03 {
                0x00 => self.psg.write_address(value),
                0x01 => self.psg.write_data(value),
                _ => self.io[offset] = value,
            },
            0x0C00..=0x0C03 | 0x1400..=0x1403 | 0x1C10..=0x1C13 => {
                // Timer/IRQ registers (mirrored)
                if !self.write_control_register(offset, value) {
                    self.io[offset] = value;
                }
            }
            0x1000..=0x13FF => {
                if !self.io_port.write(offset - HW_JOYPAD_BASE, value) {
                    self.io[offset] = value;
                }
            }
            0x1C00..=0x1FFF => {
                // Treat as additional mirror for control/TIMER/IRQ/PSG status
                if (offset & 0x3F) >= 0x40 && (offset & 0x3F) <= 0x43 {
                    // Mirror of VCE control area? leave as IO
                    self.io[offset] = value;
                } else if !self.write_control_register(offset, value) {
                    self.io[offset] = value;
                }
            }
            _ => {
                self.io[offset] = value;
            }
        }
    }

    #[cfg(feature = "trace_hw_writes")]
    fn cpu_pc_for_trace(&self) -> u16 {
        self.last_pc_for_trace.unwrap_or(0)
    }

    #[inline]
    fn write_vce_port(&mut self, addr: u16, value: u8) {
        let sub = addr & 0x0003; // hardware decodes only A1..A0
        let mirror_bit2 = (addr & 0x0004) != 0;
        let free_pass = Self::env_vce_free_pass();
        let swap = std::env::var("PCE_VCE_SUPER_SWAP").is_ok() || Self::vce_ports_swapped();
        let force_data = Self::env_vce_force_data() || free_pass;
        let mirror_route = Self::env_vce_route_mirror_as_data();

        let is_data = if force_data {
            true
        } else if swap {
            (sub & 0x02) == 0
        } else {
            (sub & 0x02) != 0
        };
        // デバッグ用: 0x0404/0405 など bit2=1 側をデータポートとして扱うオプション
        let is_data = if !is_data && mirror_bit2 && mirror_route {
            true
        } else {
            is_data
        };

        let is_high = if free_pass {
            matches!(self.vce.write_phase, VcePhase::High)
        } else if force_data {
            // data強制時は write_phase に従う
            matches!(self.vce.write_phase, VcePhase::High)
        } else {
            (sub & 0x01) != 0
        };
        self.vce_write_count += 1;
        if is_data {
            self.vce_data_writes += 1;
            if is_high {
                self.vce.write_data_high(value);
            } else {
                self.vce.write_data_low(value);
            }
        } else {
            self.vce_control_writes += 1;
            if is_high {
                self.vce_last_control_high = value;
                if value > self.vce_last_control_high_max {
                    self.vce_last_control_high_max = value;
                }
                self.vce.write_control_high(value);
            } else {
                self.vce.write_control_low(value);
            }
        }
    }

    fn refresh_vdc_irq(&mut self) {
        // Force DS/DV after many hardware writes (debug aid) or when env is set.
        const FORCE_AFTER_WRITES: u64 = 5_000;
        #[cfg(debug_assertions)]
        {
            if self.debug_force_ds_after >= FORCE_AFTER_WRITES {
                self.vdc.raise_status(VDC_STATUS_DS | VDC_STATUS_DV);
            }
        }
        if std::env::var("PCE_FORCE_VDC_DSDV").is_ok() {
            self.vdc.raise_status(VDC_STATUS_DS | VDC_STATUS_DV);
        }
        // Debug: optionally force IRQ1 every refresh to unblock BIOS waits.
        if std::env::var("PCE_FORCE_IRQ1").is_ok() {
            self.interrupt_request |= IRQ_REQUEST_IRQ1;
        }
        // Debug: optionally force IRQ2 (timer/PSG line) as well.
        if std::env::var("PCE_FORCE_IRQ2").is_ok() {
            self.interrupt_request |= IRQ_REQUEST_IRQ2;
        }
        if self.vdc.irq_active() {
            self.interrupt_request |= IRQ_REQUEST_IRQ1;
        } else {
            self.interrupt_request &= !IRQ_REQUEST_IRQ1;
        }
    }

    fn handle_vdc_dcr(&mut self, mask: u8) {
        if mask & DCR_ENABLE_VRAM_DMA != 0 {
            self.perform_vram_dma();
        }
        if mask & (DCR_ENABLE_CRAM_DMA | DCR_ENABLE_CRAM_DMA_ALT) != 0 {
            self.vdc.schedule_cram_dma();
            if self.vdc.in_vblank {
                self.perform_cram_dma();
            }
        }
        if Self::env_force_cram_dma() {
            self.perform_cram_dma();
        }
        if mask & DCR_ENABLE_SATB_DMA != 0 {
            self.vdc.perform_satb_dma();
        }
        self.vdc.registers[0x0C] &= !(mask as u16
            & (DCR_ENABLE_VRAM_DMA | DCR_ENABLE_CRAM_DMA | DCR_ENABLE_SATB_DMA) as u16);
    }

    fn perform_cram_dma(&mut self) {
        let raw_length = self.vdc.registers[0x12];
        self.vdc.last_cram_source = self.vdc.marr & 0x7FFF;
        self.vdc.last_cram_length = raw_length;
        let mut words = raw_length as usize;
        if words == 0 {
            words = 0x1_0000;
        }
        words = words.min(0x200);

        let mut src = self.vdc.marr & 0x7FFF;
        let mut index = (self.vce.control as usize) & 0x01FF;

        for _ in 0..words {
            let word = *self.vdc.vram.get(src as usize).unwrap_or(&0);
            if let Some(slot) = self.vce.palette.get_mut(index) {
                *slot = word;
            }
            index = (index + 1) & 0x01FF;
            src = Vdc::advance_vram_addr(src, false);
        }

        self.vdc.marr = src & 0x7FFF;
        self.vdc.registers[0x01] = self.vdc.marr;
        self.vce.control = (self.vce.control & !0x01FF) | (index as u16);
        let busy_cycles = (words as u32).saturating_mul(VDC_DMA_WORD_CYCLES);
        self.vdc.set_busy(busy_cycles);
        self.vdc.raise_status(VDC_STATUS_DV);
        self.vdc.registers[0x0C] &= !(DCR_ENABLE_CRAM_DMA as u16);
        self.vdc.cram_pending = false;
    }

    fn perform_vram_dma(&mut self) {
        #[cfg(any(debug_assertions, feature = "trace_hw_writes"))]
        eprintln!(
            "  VDC VRAM DMA start ctrl={:04X} src={:04X} dst={:04X} len={:04X}",
            self.vdc.dma_control,
            self.vdc.dma_source,
            self.vdc.dma_destination,
            self.vdc.registers[0x12]
        );
        let mut words = self.vdc.registers[0x12] as u32;
        if words == 0 {
            words = 0x1_0000;
        }
        if words == 0 {
            return;
        }

        let src_dec = self.vdc.dma_control & DMA_CTRL_SRC_DEC != 0;
        let dst_dec = self.vdc.dma_control & DMA_CTRL_DST_DEC != 0;

        let mut src = self.vdc.dma_source;
        let mut dst = self.vdc.dma_destination & 0x7FFF;

        self.vdc.vram_dma_count = self.vdc.vram_dma_count.saturating_add(1);
        self.vdc.last_vram_dma_source = src;
        self.vdc.last_vram_dma_destination = dst;
        self.vdc.last_vram_dma_length = words.min(0xFFFF) as u16;

        for _ in 0..words {
            let value = self.dma_read_word(src);
            self.vdc.write_vram_dma_word(dst, value);

            src = if src_dec {
                src.wrapping_sub(2)
            } else {
                src.wrapping_add(2)
            };
            dst = Vdc::advance_vram_addr(dst, dst_dec);
        }

        self.vdc.dma_source = src;
        self.vdc.dma_destination = dst;
        self.vdc.registers[0x10] = self.vdc.dma_source;
        self.vdc.registers[0x11] = self.vdc.dma_destination;
        self.vdc.registers[0x12] = 0;

        #[cfg(any(debug_assertions, feature = "trace_hw_writes"))]
        eprintln!(
            "  VDC VRAM DMA end src={:04X} dst={:04X} len={:04X}",
            self.vdc.dma_source, self.vdc.dma_destination, self.vdc.last_vram_dma_length
        );

        let busy_cycles = words.saturating_mul(VDC_DMA_WORD_CYCLES);
        self.vdc.set_busy(busy_cycles);
        self.vdc.raise_status(VDC_STATUS_DV);

        // デバッグ用: VRAM DMA 完了時に VRAM 先頭から CRAM 512 ワードを強制ロード。
        if std::env::var("PCE_FORCE_CRAM_FROM_VRAM").is_ok() {
            for i in 0..0x200 {
                let word = self.vdc.vram.get(i).copied().unwrap_or(0);
                if let Some(slot) = self.vce.palette.get_mut(i) {
                    *slot = word;
                }
            }
            #[cfg(any(debug_assertions, feature = "trace_hw_writes"))]
            eprintln!("  DEBUG PCE_FORCE_CRAM_FROM_VRAM applied (first 512 words)");
        }
    }

    fn dma_read_word(&mut self, addr: u16) -> u16 {
        let lo = self.dma_read_byte(addr);
        let hi = self.dma_read_byte(addr.wrapping_add(1));
        u16::from_le_bytes([lo, hi])
    }

    fn dma_read_byte(&mut self, addr: u16) -> u8 {
        let (mapping, offset) = self.resolve(addr);
        match mapping {
            BankMapping::Ram { base } => self.ram.get(base + offset).copied().unwrap_or(0),
            BankMapping::Rom { base } => self.rom.get(base + offset).copied().unwrap_or(0),
            BankMapping::CartRam { base } => self.cart_ram.get(base + offset).copied().unwrap_or(0),
            BankMapping::Hardware => self.read(addr),
        }
    }

    fn enabled_irq_mask(&self) -> u8 {
        let mut mask = 0;
        if self.interrupt_disable & IRQ_DISABLE_IRQ2 == 0 {
            mask |= IRQ_REQUEST_IRQ2;
        }
        if self.interrupt_disable & IRQ_DISABLE_IRQ1 == 0 {
            mask |= IRQ_REQUEST_IRQ1;
        }
        if self.interrupt_disable & IRQ_DISABLE_TIMER == 0 {
            mask |= IRQ_REQUEST_TIMER;
        }
        mask
    }
}

#[derive(Clone, Copy, Debug)]
enum BankMapping {
    Ram { base: usize },
    Rom { base: usize },
    CartRam { base: usize },
    Hardware,
}

#[derive(Clone, Copy)]
enum ControlRegister {
    TimerCounter,
    TimerControl,
    IrqMask,
    IrqStatus,
}

#[derive(Clone, Copy)]
struct IoPort {
    output: u8,
    direction: u8,
    enable: u8,
    select: u8,
    input: u8,
}

#[derive(Clone, Copy)]
struct Timer {
    reload: u8,
    counter: u8,
    prescaler: u32,
    enabled: bool,
}

#[derive(Clone)]
struct Vdc {
    registers: [u16; VDC_REGISTER_COUNT],
    vram: Vec<u16>,
    satb: [u16; 0x100],
    selected: u8,
    latch_low: u8,
    write_phase: VdcWritePhase,
    read_phase: VdcReadPhase,
    read_buffer: u16,
    mawr: u16,
    marr: u16,
    status: u8,
    phi_scaled: u64,
    busy_cycles: u32,
    scanline: u16,
    dma_control: u16,
    dma_source: u16,
    dma_destination: u16,
    satb_source: u16,
    satb_pending: bool,
    in_vblank: bool,
    frame_trigger: bool,
    scroll_x: u16,
    scroll_y: u16,
    scroll_x_pending: u16,
    scroll_y_pending: u16,
    scroll_x_dirty: bool,
    scroll_y_dirty: bool,
    zoom_x: u16,
    zoom_y: u16,
    zoom_x_pending: u16,
    zoom_y_pending: u16,
    zoom_x_dirty: bool,
    zoom_y_dirty: bool,
    scroll_line_x: [u16; LINES_PER_FRAME as usize],
    scroll_line_y: [u16; LINES_PER_FRAME as usize],
    zoom_line_x: [u16; LINES_PER_FRAME as usize],
    zoom_line_y: [u16; LINES_PER_FRAME as usize],
    scroll_line_valid: [bool; LINES_PER_FRAME as usize],
    dcr_request: Option<u8>,
    cram_pending: bool,
    cram_dma_count: u64,
    control_write_count: u64,
    last_control_value: u16,
    last_cram_source: u16,
    last_cram_length: u16,
    vram_dma_count: u64,
    last_vram_dma_source: u16,
    last_vram_dma_destination: u16,
    last_vram_dma_length: u16,
    dcr_write_count: u64,
    last_dcr_value: u8,
    register_write_counts: [u64; VDC_REGISTER_COUNT],
    register_select_counts: [u64; VDC_REGISTER_COUNT],
    r05_low_writes: u64,
    r05_high_writes: u64,
    last_r05_low: u8,
    ignore_next_high_byte: bool,
    // Remember which register a low byte targeted so the paired high byte
    // commits to the same register even if ST0 is touched in between.
    pending_write_register: Option<u8>,
    #[cfg(feature = "trace_hw_writes")]
    pending_traced_register: Option<u8>,
    #[cfg(feature = "trace_hw_writes")]
    last_io_addr: u16,
    #[cfg(feature = "trace_hw_writes")]
    st0_hold_counter: u8,
    #[cfg(feature = "trace_hw_writes")]
    st0_hold_addr_hist: [u32; 0x100],
    st0_locked_until_commit: bool,
}

pub const VDC_STATUS_CR: u8 = 0x01;
pub const VDC_STATUS_OR: u8 = 0x02;
pub const VDC_STATUS_RCR: u8 = 0x04;
pub const VDC_STATUS_DS: u8 = 0x08;
pub const VDC_STATUS_DV: u8 = 0x10;
pub const VDC_STATUS_VBL: u8 = 0x20;
pub const VDC_STATUS_BUSY: u8 = 0x40;
const DMA_CTRL_IRQ_SATB: u16 = 0x0001;
const DMA_CTRL_IRQ_VRAM: u16 = 0x0002;
const DMA_CTRL_SRC_DEC: u16 = 0x0004;
const DMA_CTRL_DST_DEC: u16 = 0x0008;
const DMA_CTRL_SATB_AUTO: u16 = 0x0010;
const VDC_VISIBLE_LINES: u16 = 240;

impl Vdc {
    fn new() -> Self {
        let mut vdc = Self {
            registers: [0; VDC_REGISTER_COUNT],
            vram: vec![0; 0x8000],
            satb: [0; 0x100],
            selected: 0,
            latch_low: 0,
            write_phase: VdcWritePhase::Low,
            read_phase: VdcReadPhase::Low,
            read_buffer: 0,
            mawr: 0,
            marr: 0,
            status: VDC_STATUS_VBL | VDC_STATUS_DS, // start inside VBlank with SATB DMA idle
            phi_scaled: 0,
            busy_cycles: 0,
            scanline: LINES_PER_FRAME - 1,
            dma_control: 0,
            dma_source: 0,
            dma_destination: 0,
            satb_source: 0,
            satb_pending: false,
            in_vblank: true,
            frame_trigger: false,
            scroll_x: 0,
            scroll_y: 0,
            scroll_x_pending: 0,
            scroll_y_pending: 0,
            scroll_x_dirty: false,
            scroll_y_dirty: false,
            zoom_x: 0x0010,
            zoom_y: 0x0010,
            zoom_x_pending: 0x0010,
            zoom_y_pending: 0x0010,
            zoom_x_dirty: false,
            zoom_y_dirty: false,
            scroll_line_x: [0; LINES_PER_FRAME as usize],
            scroll_line_y: [0; LINES_PER_FRAME as usize],
            zoom_line_x: [0; LINES_PER_FRAME as usize],
            zoom_line_y: [0; LINES_PER_FRAME as usize],
            scroll_line_valid: [false; LINES_PER_FRAME as usize],
            dcr_request: None,
            cram_pending: false,
            cram_dma_count: 0,
            control_write_count: 0,
            last_control_value: 0,
            last_cram_source: 0,
            last_cram_length: 0,
            vram_dma_count: 0,
            last_vram_dma_source: 0,
            last_vram_dma_destination: 0,
            last_vram_dma_length: 0,
            dcr_write_count: 0,
            last_dcr_value: 0,
            register_write_counts: [0; VDC_REGISTER_COUNT],
            register_select_counts: [0; VDC_REGISTER_COUNT],
            r05_low_writes: 0,
            r05_high_writes: 0,
            last_r05_low: 0,
            ignore_next_high_byte: false,
            pending_write_register: None,
            #[cfg(feature = "trace_hw_writes")]
            pending_traced_register: None,
            #[cfg(feature = "trace_hw_writes")]
            last_io_addr: 0,
            #[cfg(feature = "trace_hw_writes")]
            st0_hold_counter: 0,
            #[cfg(feature = "trace_hw_writes")]
            st0_hold_addr_hist: [0; 0x100],
            st0_locked_until_commit: false,
        };
        vdc.registers[0x04] = VDC_CTRL_ENABLE_BACKGROUND_LEGACY | VDC_CTRL_ENABLE_SPRITES_LEGACY;
        vdc.registers[0x05] = vdc.registers[0x04];
        vdc.last_control_value = vdc.registers[0x04];
        vdc.registers[0x09] = 0x0010; // default to 64x32 virtual map
        vdc.registers[0x0A] = 0x0010;
        vdc.registers[0x0B] = 0x0010;
        vdc.refresh_activity_flags();
        // Debug: optionally force status bits at power-on to unblock BIOS waits.
        if let Some(mask) = std::env::var("PCE_FORCE_VDC_STATUS")
            .ok()
            .and_then(|s| u8::from_str_radix(&s, 16).ok())
        {
            vdc.status |= mask;
        }
        // 初期化直後は BUSY を確実に落としておく（リセット直後の BIOS 待ちループ対策）
        vdc.status &= !VDC_STATUS_BUSY;
        vdc
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.vram.fill(0);
        self.satb.fill(0);
        self.selected = 0;
        self.latch_low = 0;
        self.write_phase = VdcWritePhase::Low;
        self.read_phase = VdcReadPhase::Low;
        self.read_buffer = 0;
        self.mawr = 0;
        self.marr = 0;
        self.status = VDC_STATUS_VBL | VDC_STATUS_DS;
        self.phi_scaled = 0;
        self.busy_cycles = 0;
        self.scanline = LINES_PER_FRAME - 1;
        self.dma_control = 0;
        self.dma_source = 0;
        self.dma_destination = 0;
        self.satb_source = 0;
        self.satb_pending = false;
        self.in_vblank = true;
        self.frame_trigger = false;
        self.registers[0x09] = 0x0010;
        self.refresh_activity_flags();
        self.status &= !VDC_STATUS_BUSY;
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.scroll_x_pending = 0;
        self.scroll_y_pending = 0;
        self.scroll_x_dirty = false;
        self.scroll_y_dirty = false;
        self.zoom_x = 0x0010;
        self.zoom_y = 0x0010;
        self.zoom_x_pending = 0x0010;
        self.zoom_y_pending = 0x0010;
        self.zoom_x_dirty = false;
        self.zoom_y_dirty = false;
        self.scroll_line_x = [0; LINES_PER_FRAME as usize];
        self.scroll_line_y = [0; LINES_PER_FRAME as usize];
        self.zoom_line_x = [0; LINES_PER_FRAME as usize];
        self.zoom_line_y = [0; LINES_PER_FRAME as usize];
        self.scroll_line_valid = [false; LINES_PER_FRAME as usize];
        self.dcr_request = None;
        self.cram_pending = false;
        self.cram_dma_count = 0;
        self.control_write_count = 0;
        self.registers[0x04] = VDC_CTRL_ENABLE_BACKGROUND_LEGACY | VDC_CTRL_ENABLE_SPRITES_LEGACY;
        self.registers[0x05] = self.registers[0x04];
        self.last_control_value = self.registers[0x04];
        self.last_cram_source = 0;
        self.last_cram_length = 0;
        self.vram_dma_count = 0;
        self.last_vram_dma_source = 0;
        self.last_vram_dma_destination = 0;
        self.last_vram_dma_length = 0;
        self.dcr_write_count = 0;
        self.last_dcr_value = 0;
        self.register_write_counts = [0; VDC_REGISTER_COUNT];
        self.register_select_counts = [0; VDC_REGISTER_COUNT];
        self.r05_low_writes = 0;
        self.r05_high_writes = 0;
        self.last_r05_low = 0;
        self.pending_write_register = None;
        self.registers[0x0A] = 0x0010;
        self.registers[0x0B] = 0x0010;
        self.ignore_next_high_byte = false;
    }

    fn read_status(&mut self) -> u8 {
        self.refresh_activity_flags();
        let value = self.status;
        let preserved = self.status & VDC_STATUS_BUSY;
        self.status = preserved;
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC status -> {:02X} (VBL={} DS={} DV={} BUSY={} busy_cycles={})",
            value,
            (value & VDC_STATUS_VBL) != 0,
            (value & VDC_STATUS_DS) != 0,
            (value & VDC_STATUS_DV) != 0,
            (value & VDC_STATUS_BUSY) != 0,
            self.busy_cycles
        );
        value
    }

    #[allow(dead_code)]
    fn raise_status(&mut self, mask: u8) {
        self.status |= mask;
    }

    fn status_bits(&self) -> u8 {
        self.status
    }

    fn control_write_count(&self) -> u64 {
        self.control_write_count
    }

    fn last_control_value(&self) -> u16 {
        self.last_control_value
    }

    fn r05_low_writes(&self) -> u64 {
        self.r05_low_writes
    }

    fn r05_high_writes(&self) -> u64 {
        self.r05_high_writes
    }

    fn last_r05_low(&self) -> u8 {
        self.last_r05_low
    }

    fn satb_pending(&self) -> bool {
        self.satb_pending
    }

    fn satb_source(&self) -> u16 {
        self.satb_source
    }

    fn clear_sprite_overflow(&mut self) {
        self.status &= !VDC_STATUS_OR;
    }

    fn irq_active(&self) -> bool {
        let mask = self.enabled_status_mask() | self.enabled_dma_status_mask();
        (self.status & mask) != 0
    }

    fn enabled_status_mask(&self) -> u8 {
        let ctrl = self.control();
        let mut mask = 0;
        if ctrl & 0x0001 != 0 {
            mask |= VDC_STATUS_CR;
        }
        if ctrl & 0x0002 != 0 {
            mask |= VDC_STATUS_OR;
        }
        if ctrl & 0x0004 != 0 {
            mask |= VDC_STATUS_RCR;
        }
        if ctrl & 0x0008 != 0 {
            mask |= VDC_STATUS_VBL;
        }
        mask
    }

    fn enabled_dma_status_mask(&self) -> u8 {
        let mut mask = 0;
        if self.dma_control & DMA_CTRL_IRQ_SATB != 0 {
            mask |= VDC_STATUS_DS;
        }
        if self.dma_control & DMA_CTRL_IRQ_VRAM != 0 {
            mask |= VDC_STATUS_DV;
        }
        mask
    }

    fn control(&self) -> u16 {
        self.registers[0x04]
    }

    fn tick(&mut self, phi_cycles: u32) -> bool {
        if phi_cycles == 0 {
            return false;
        }

        self.consume_busy(phi_cycles);

        let frame_cycles = VDC_VBLANK_INTERVAL as u64;
        self.phi_scaled = self
            .phi_scaled
            .saturating_add(phi_cycles as u64 * LINES_PER_FRAME as u64);

        let mut irq_recalc = false;
        while self.phi_scaled >= frame_cycles {
            self.phi_scaled -= frame_cycles;
            let wrapped = self.advance_scanline();
            if wrapped {
                irq_recalc = true;
            }

            let rcr_target = self.registers[0x06] & 0x03FF;
            if self.scanline == rcr_target {
                self.raise_status(VDC_STATUS_RCR);
                irq_recalc = true;
            }

            if self.scanline == VDC_VISIBLE_LINES {
                self.in_vblank = true;
                self.raise_status(VDC_STATUS_VBL);
                self.refresh_activity_flags();
                irq_recalc = true;
                if self.handle_vblank_start() {
                    irq_recalc = true;
                }
                self.frame_trigger = true;
            }
        }

        irq_recalc
    }

    fn frame_ready(&self) -> bool {
        self.frame_trigger
    }

    fn clear_frame_trigger(&mut self) {
        self.frame_trigger = false;
    }

    fn set_busy(&mut self, cycles: u32) {
        let divisor = Bus::env_vdc_busy_divisor().max(1);
        let scaled = if divisor == 1 { cycles } else { cycles / divisor };
        self.busy_cycles = self.busy_cycles.max(scaled);
        self.refresh_activity_flags();
    }

    fn consume_busy(&mut self, phi_cycles: u32) {
        if self.busy_cycles > 0 {
            if phi_cycles >= self.busy_cycles {
                self.busy_cycles = 0;
            } else {
                self.busy_cycles -= phi_cycles;
            }
        }
        self.refresh_activity_flags();
    }

    fn refresh_activity_flags(&mut self) {
        if self.busy_cycles > 0 {
            self.status |= VDC_STATUS_BUSY;
        } else {
            self.status &= !VDC_STATUS_BUSY;
        }
    }

    fn write_port(&mut self, port: usize, value: u8) {
        match port {
            0 => self.write_select(value),
            1 => self.write_data_port(value),
            2 => self.write_data_high_direct(value),
            _ => {}
        }
    }

    fn read_port(&mut self, port: usize) -> u8 {
        match port {
            0 => self.read_status(),
            1 => self.read_data_port(),
            2 => self.read_data_high(),
            _ => 0,
        }
    }

    fn selected_register(&self) -> u8 {
        self.map_register_index(self.selected & 0x1F)
    }

    fn map_register_index(&self, raw: u8) -> u8 {
        match raw {
            0x03 => 0x04, // CR (control) -> internal alias at 0x04/0x05
            _ => raw,
        }
    }

    fn register(&self, index: usize) -> Option<u16> {
        self.registers.get(index).copied()
    }

    fn register_write_count(&self, index: usize) -> u64 {
        self.register_write_counts.get(index).copied().unwrap_or(0)
    }

    fn register_select_count(&self, index: usize) -> u64 {
        self.register_select_counts.get(index).copied().unwrap_or(0)
    }

    fn write_select(&mut self, value: u8) {
        if self.st0_locked_until_commit && matches!(self.write_phase, VdcWritePhase::High) {
            #[cfg(feature = "trace_hw_writes")]
            eprintln!(
                "  VDC select ignored while locked (pending={:?} phase={:?})",
                self.pending_write_register, self.write_phase
            );
            return;
        }
        let new_sel = value & 0x1F;
        // Changing the selected register should invalidate any in-flight
        // low-byte latch to avoid pairing a new selection with an old low byte.
        // However, the BIOS often sprays zeros across the IO window; to avoid
        // losing a just-written selector, keep it alive until either another
        // ST0 write arrives or a data byte commits.
        if self.pending_write_register.is_some() && self.pending_write_register != Some(new_sel) {
            // keep latch; don't clear here
        } else {
            self.pending_write_register = None;
        }
        #[cfg(feature = "trace_hw_writes")]
        if new_sel == 0x05 {
            eprintln!("  TRACE select R05 (pc={:04X})", 0);
        }
        if self.map_register_index(new_sel) == self.map_register_index(self.selected) {
            // Avoid clobbering an in-flight data write when the ROM spams the same ST0 value.
            #[cfg(feature = "trace_hw_writes")]
            eprintln!(
                "  VDC select (dedup) {:02X} pending={:?} phase={:?}",
                new_sel, self.pending_write_register, self.write_phase
            );
            self.write_phase = VdcWritePhase::Low;
            self.ignore_next_high_byte = false;
            return;
        }
        self.selected = new_sel;
        self.write_phase = VdcWritePhase::Low;
        self.ignore_next_high_byte = false;
        let index = self.map_register_index(self.selected) as usize;
        if let Some(count) = self.register_select_counts.get_mut(index) {
            *count = count.saturating_add(1);
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC select {:02X} pending={:?} phase={:?}",
            self.selected, self.pending_write_register, self.write_phase
        );
    }

    fn write_data_low(&mut self, value: u8) {
        self.latch_low = value;
        self.pending_write_register = Some(self.selected_register());
        self.st0_locked_until_commit = true;
        if matches!(self.selected_register(), 0x04 | 0x05) {
            self.r05_low_writes = self.r05_low_writes.saturating_add(1);
            self.last_r05_low = value;
        }
        #[cfg(feature = "trace_hw_writes")]
        {
            let reg = self.selected_register();
            if matches!(reg, 0x04 | 0x05) {
                eprintln!("  TRACE R05 low {:02X}", value);
            } else if matches!(reg, 0x10 | 0x11 | 0x12) {
                eprintln!("  TRACE DMA reg {:02X} low {:02X}", reg, value);
            }
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC low byte {:02X} latched for R{:02X} pending={:?} phase={:?}",
            value,
            self.selected_register(),
            self.pending_write_register,
            self.write_phase
        );
        let index = self.selected_register() as usize;
        if matches!(index, 0x0C) {
            self.commit_register_write(index, value as u16);
            self.write_phase = VdcWritePhase::High;
            self.ignore_next_high_byte = true;
            self.st0_locked_until_commit = false;
            #[cfg(feature = "trace_hw_writes")]
            {
                self.st0_hold_counter = 0;
            }
        } else if index == 0x04 || index == 0x05 {
            // CR writes often arrive as a single byte to flip display enables.
            let existing = self.registers.get(index).copied().unwrap_or(0);
            let combined = (existing & 0xFF00) | value as u16;
            self.commit_register_write(index, combined);
            self.write_phase = VdcWritePhase::High;
            self.ignore_next_high_byte = false;
        } else {
            self.write_phase = VdcWritePhase::High;
            self.ignore_next_high_byte = false;
        }
    }

    fn write_data_high(&mut self, value: u8) {
        let target_reg = self
            .pending_write_register
            .unwrap_or_else(|| self.selected_register());
        // Prefer the latched low byte when a prior write captured one (even if
        // ST0 was re-written in between); otherwise, fall back to the current
        // register value to avoid clobbering the low byte on high-only writes.
        let use_latch = matches!(self.write_phase, VdcWritePhase::High)
            && self.pending_write_register.is_some();
        if use_latch && self.ignore_next_high_byte {
            self.write_phase = VdcWritePhase::Low;
            self.ignore_next_high_byte = false;
            self.pending_write_register = None;
            return;
        }
        let low = if use_latch {
            self.latch_low
        } else {
            let index = (self.selected & 0x1F) as usize;
            self.registers
                .get(index)
                .copied()
                .unwrap_or(0)
                .to_le_bytes()[0]
        };
        let combined = u16::from_le_bytes([low, value]);
        let index = self.pending_write_register.take().unwrap_or(target_reg) as usize;
        self.st0_locked_until_commit = false;
        #[cfg(feature = "trace_hw_writes")]
        {
            self.st0_hold_counter = 0;
        }
        if index == 0x04 || index == 0x05 {
            self.r05_high_writes = self.r05_high_writes.saturating_add(1);
        }
        #[cfg(feature = "trace_hw_writes")]
        {
            if index == 0x04 || index == 0x05 {
                eprintln!("  TRACE R05 high {:02X} commit {:04X}", value, combined);
            } else if matches!(index, 0x10 | 0x11 | 0x12) {
                eprintln!("  TRACE DMA reg {:02X} high {:02X} commit {:04X}", index, value, combined);
            }
            self.debug_log_select_and_value(index as u8, combined);
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC high byte {:02X} -> commit R{:02X} = {:04X} (selected={:02X} pending={:?} phase={:?})",
            value,
            index,
            combined,
            self.selected_register(),
            self.pending_write_register,
            self.write_phase
        );
        self.commit_register_write(index, combined);
        self.write_phase = VdcWritePhase::Low;
        if std::env::var("PCE_HOLD_DSDV").is_ok() {
            self.status |= VDC_STATUS_DS | VDC_STATUS_DV;
        }
    }

    fn write_data_port(&mut self, value: u8) {
        // If the CPU streams data without setting the expected phase, treat the first byte
        // as a low write to ensure the pair commits (helps BIOS 3-byte ST sequences).
        if !matches!(self.write_phase, VdcWritePhase::Low | VdcWritePhase::High) {
            self.write_phase = VdcWritePhase::Low;
            self.ignore_next_high_byte = false;
            self.pending_write_register = None;
        }

        match self.write_phase {
            VdcWritePhase::Low => self.write_data_low(value),
            VdcWritePhase::High => self.write_data_high(value),
        }

        #[cfg(feature = "trace_hw_writes")]
        if value != 0 {
            eprintln!(
                "  VDC DATA non-zero write sel={:02X} phase={:?} val={:02X} addr={:04X}",
                self.selected_register(),
                self.write_phase,
                value,
                self.last_io_addr
            );
        }
    }

    fn write_data_high_direct(&mut self, value: u8) {
        if !matches!(self.write_phase, VdcWritePhase::High) {
            self.write_phase = VdcWritePhase::High;
            self.ignore_next_high_byte = false;
        }
        self.write_data_high(value);
    }

    fn read_data_port(&mut self) -> u8 {
        match self.read_phase {
            VdcReadPhase::Low => self.read_data_low(),
            VdcReadPhase::High => self.read_data_high(),
        }
    }

    #[cfg(feature = "trace_hw_writes")]
    fn debug_log_select_and_value(&self, reg: u8, value: u16) {
        if matches!(reg, 0x04 | 0x05 | 0x10 | 0x11 | 0x12) {
            eprintln!("  TRACE commit R{:02X} = {:04X}", reg, value);
        }
    }

    fn handle_dcr_write(&mut self, value: u16) {
        // DCR is an 8-bit register. Some BIOS code appears to poke it using
        // only the high byte path; if the low byte is zero and the high byte
        // is non-zero, treat that as the intended value.
        let masked = if (value & 0x00FF) == 0 {
            (value >> 8) & 0x00FF
        } else {
            value & 0x00FF
        };
        self.registers[0x0C] = masked;
        self.dcr_request = Some(masked as u8);
        self.dcr_write_count = self.dcr_write_count.saturating_add(1);
        self.last_dcr_value = masked as u8;
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC DCR <= {:02X} (MAWR {:04X} MARR {:04X} DMA src {:04X} dst {:04X} len {:04X})",
            masked,
            self.mawr,
            self.marr,
            self.registers[0x10],
            self.registers[0x11],
            self.registers[0x12]
        );
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("  VDC DCR <= {:02X}", masked);
    }

    fn take_dcr_request(&mut self) -> Option<u8> {
        self.dcr_request.take()
    }

    fn commit_register_write(&mut self, index: usize, combined: u16) {
        #[cfg(feature = "trace_hw_writes")]
        {
            eprintln!(
                "  VDC write R{:02X} = {:04X} (sel={:02X})",
                index,
                combined,
                self.selected_register()
            );
            if index == 0x05 {
                eprintln!("  TRACE R05 commit {:04X}", combined);
            }
        }
        if index < self.registers.len() {
            let stored = if matches!(index, 0x00 | 0x01) {
                combined & 0x7FFF
            } else {
                combined
            };
            self.registers[index] = stored;
            if let Some(count) = self.register_write_counts.get_mut(index) {
                *count = count.saturating_add(1);
            }
        }
        match index {
            0x00 => {
                self.mawr = combined & 0x7FFF;
                self.registers[0x00] = self.mawr;
            }
            0x01 => {
                self.marr = combined & 0x7FFF;
                self.registers[0x01] = self.marr;
                self.prefetch_read();
                self.read_phase = VdcReadPhase::Low;
            }
            0x02 => self.write_vram(combined),
            0x04 | 0x05 => {
                // Mirror control into both slots so legacy/tests remain stable.
                self.registers[0x04] = combined;
                self.registers[0x05] = combined;
                self.control_write_count = self.control_write_count.saturating_add(1);
                self.last_control_value = combined;
                #[cfg(feature = "trace_hw_writes")]
                eprintln!("  VDC control <= {:04X}", combined);
            }
            0x07 => {
                let masked = combined & 0x03FF;
                self.registers[0x07] = masked;
                self.scroll_x_pending = masked;
                self.scroll_x_dirty = true;
            }
            0x08 => {
                let masked = combined & 0x01FF;
                self.registers[0x08] = masked;
                self.scroll_y_pending = masked;
                self.scroll_y_dirty = true;
            }
            0x0A => {
                let masked = combined & 0x001F;
                self.registers[0x0A] = masked;
                self.zoom_x_pending = masked;
                self.zoom_x_dirty = true;
            }
            0x0B => {
                let masked = combined & 0x001F;
                self.registers[0x0B] = masked;
                self.zoom_y_pending = masked;
                self.zoom_y_dirty = true;
            }
            0x0C => {
                #[cfg(feature = "trace_hw_writes")]
                eprintln!("  VDC write R0C = {:04X}", combined);
                self.registers[0x0C] = combined;
                self.handle_dcr_write(combined);
            }
            0x0F => self.write_dma_control(combined),
            0x10 => self.write_dma_source(combined),
            0x11 => self.write_dma_destination(combined),
            0x12 => self.write_dma_length(combined),
            0x13 | 0x14 => self.write_satb_source(index, combined),
            _ => {}
        }
    }

    fn schedule_cram_dma(&mut self) {
        self.cram_pending = true;
        self.cram_dma_count += 1;
        self.last_cram_source = self.marr & 0x7FFF;
        self.last_cram_length = self.registers[0x12];
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC CRAM DMA scheduled (pending len {:04X}) source {:04X} (MAWR {:04X})",
            self.registers[0x12],
            self.last_cram_source,
            self.marr & 0x7FFF
        );
    }

    fn write_dma_control(&mut self, value: u16) {
        let masked = value & 0x001F;
        self.dma_control = masked;
        self.registers[0x0F] = masked;
        // Writing DMA control normally acknowledges both DMA-complete flags.
        if std::env::var("PCE_HOLD_DSDV").is_err() {
            self.status &= !(VDC_STATUS_DS | VDC_STATUS_DV);
        }
        if masked & DMA_CTRL_SATB_AUTO == 0 {
            self.satb_pending = false;
        }
    }

    fn write_dma_source(&mut self, value: u16) {
        self.dma_source = value;
        self.registers[0x10] = value;
    }

    fn write_dma_destination(&mut self, value: u16) {
        let masked = value & 0x7FFF;
        self.dma_destination = masked;
        self.registers[0x11] = masked;
    }

    fn write_dma_length(&mut self, value: u16) {
        self.registers[0x12] = value;
    }

    fn write_satb_source(&mut self, index: usize, value: u16) {
        let masked = value & 0x7FFF;
        self.satb_source = masked;
        if let Some(slot) = self.registers.get_mut(index) {
            *slot = masked;
        }
        let auto = (self.dma_control & DMA_CTRL_SATB_AUTO) != 0;
        self.satb_pending = auto;
        // The hardware latches the source address and primes a transfer that
        // completes on the next vertical blanking interval. The BIOS expects
        // the DS flag to raise promptly after writing to SATB, so perform the
        // copy immediately while still allowing auto-transfer to re-run each
        // frame when enabled.
        self.perform_satb_dma();
    }

    fn perform_satb_dma(&mut self) {
        let base = (self.satb_source & 0x7FFF) as usize;
        for i in 0..self.satb.len() {
            let idx = (base + i) & 0x7FFF;
            self.satb[i] = *self.vram.get(idx).unwrap_or(&0);
        }
        let busy_cycles = (self.satb.len() as u32).saturating_mul(VDC_DMA_WORD_CYCLES);
        self.set_busy(busy_cycles);
        self.raise_status(VDC_STATUS_DS);
        self.satb_pending = (self.dma_control & DMA_CTRL_SATB_AUTO) != 0;
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VDC SATB DMA complete (source {:04X}) -> status {:02X}",
            self.satb_source, self.status
        );
    }

    fn handle_vblank_start(&mut self) -> bool {
        if !self.satb_pending {
            return false;
        }
        self.perform_satb_dma();
        true
    }

    fn advance_vram_addr(addr: u16, decrement: bool) -> u16 {
        let next = if decrement {
            addr.wrapping_sub(1)
        } else {
            addr.wrapping_add(1)
        };
        next & 0x7FFF
    }

    fn write_vram(&mut self, value: u16) {
        let idx = (self.mawr as usize) & 0x7FFF;
        if let Some(slot) = self.vram.get_mut(idx) {
            *slot = value;
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("    VRAM[{:04X}] = {:04X}", self.mawr & 0x7FFF, value);
        self.set_busy(VDC_BUSY_ACCESS_CYCLES);
        self.mawr = (self.mawr.wrapping_add(self.increment_step())) & 0x7FFF;
        self.registers[0x00] = self.mawr;
        self.registers[0x02] = value;
    }

    fn write_vram_dma_word(&mut self, addr: u16, value: u16) {
        let idx = (addr as usize) & 0x7FFF;
        if let Some(slot) = self.vram.get_mut(idx) {
            *slot = value;
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("    VRAM DMA[{:04X}] = {:04X}", addr & 0x7FFF, value);
    }

    fn read_data_low(&mut self) -> u8 {
        if self.read_phase == VdcReadPhase::Low {
            self.prefetch_read();
        }
        self.read_phase = VdcReadPhase::High;
        (self.read_buffer & 0x00FF) as u8
    }

    fn read_data_high(&mut self) -> u8 {
        if self.read_phase == VdcReadPhase::Low {
            self.prefetch_read();
        }
        let value = (self.read_buffer >> 8) as u8;
        self.advance_read_address();
        self.read_phase = VdcReadPhase::Low;
        value
    }

    fn prefetch_read(&mut self) {
        let idx = (self.marr as usize) & 0x7FFF;
        self.read_buffer = *self.vram.get(idx).unwrap_or(&0);
        self.set_busy(VDC_BUSY_ACCESS_CYCLES);
        self.registers[0x02] = self.read_buffer;
    }

    fn advance_read_address(&mut self) {
        self.marr = (self.marr.wrapping_add(self.increment_step())) & 0x7FFF;
        self.registers[0x01] = self.marr;
    }

    fn increment_step(&self) -> u16 {
        match (self.control() >> 11) & 0b11 {
            0 => 1,
            1 => 32,
            2 => 64,
            3 => 128,
            _ => 1,
        }
    }

    fn map_dimensions(&self) -> (usize, usize) {
        let mwr = self.registers[0x09];
        let width_code = ((mwr >> 4) & 0x03) as usize;
        let width_tiles = match width_code {
            0 => 32,
            1 => 64,
            _ => 128,
        };
        let height_tiles = if (mwr >> 6) & 0x01 == 0 { 32 } else { 64 };
        (width_tiles, height_tiles)
    }

    fn map_base_address(&self) -> usize {
        let mwr = self.registers[0x09];
        let base_index = ((mwr >> 8) & 0x0F) as usize;
        (base_index << 10) & 0x7FFF
    }

    fn map_entry_address(&self, tile_row: usize, tile_col: usize) -> usize {
        let (map_width, map_height) = self.map_dimensions();
        let width = map_width.max(1);
        let height = map_height.max(1);
        let row = tile_row % height;
        let col = tile_col % width;
        let block_cols = (width + 31) / 32;
        let block_row = row / 32;
        let block_col = col / 32;
        let block_index = block_row * block_cols + block_col;
        let block_base = (self.map_base_address() + block_index * 0x400) & 0x7FFF;
        let local_row = row % 32;
        let local_col = col % 32;
        (block_base + local_row * 32 + local_col) & 0x7FFF
    }

    #[cfg(test)]
    fn map_entry_address_for_test(&self, tile_row: usize, tile_col: usize) -> usize {
        self.map_entry_address(tile_row, tile_col)
    }

    fn apply_pending_scroll(&mut self) {
        if self.scroll_x_dirty {
            self.scroll_x = self.scroll_x_pending & 0x03FF;
            self.scroll_x_dirty = false;
        }
        if self.scroll_y_dirty {
            self.scroll_y = self.scroll_y_pending & 0x01FF;
            self.scroll_y_dirty = false;
        }
    }

    fn apply_pending_zoom(&mut self) {
        if self.zoom_x_dirty {
            self.zoom_x = self.zoom_x_pending & 0x001F;
            self.zoom_x_dirty = false;
        }
        if self.zoom_y_dirty {
            self.zoom_y = self.zoom_y_pending & 0x001F;
            self.zoom_y_dirty = false;
        }
    }

    fn latch_line_state(&mut self, line: usize) {
        self.apply_pending_scroll();
        self.apply_pending_zoom();
        let idx = line % self.scroll_line_x.len();
        self.scroll_line_x[idx] = self.scroll_x;
        self.scroll_line_y[idx] = self.scroll_y;
        self.zoom_line_x[idx] = self.zoom_x;
        self.zoom_line_y[idx] = self.zoom_y;
        self.scroll_line_valid[idx] = true;
    }

    fn ensure_line_state(&mut self, line: usize) {
        if line >= self.scroll_line_x.len() {
            self.apply_pending_scroll();
            self.apply_pending_zoom();
            return;
        }
        if !self.scroll_line_valid[line] {
            self.apply_pending_scroll();
            self.apply_pending_zoom();
            self.scroll_line_x[line] = self.scroll_x;
            self.scroll_line_y[line] = self.scroll_y;
            self.zoom_line_x[line] = self.zoom_x;
            self.zoom_line_y[line] = self.zoom_y;
            self.scroll_line_valid[line] = true;
        }
    }

    fn scroll_values_for_line(&mut self, line: usize) -> (usize, usize) {
        self.ensure_line_state(line);
        if line < self.scroll_line_x.len() {
            (
                self.scroll_line_x[line] as usize,
                self.scroll_line_y[line] as usize,
            )
        } else {
            (self.scroll_x as usize, self.scroll_y as usize)
        }
    }

    fn zoom_values_for_line(&mut self, line: usize) -> (u16, u16) {
        self.ensure_line_state(line);
        if line < self.zoom_line_x.len() {
            (self.zoom_line_x[line], self.zoom_line_y[line])
        } else {
            (self.zoom_x, self.zoom_y)
        }
    }

    fn advance_scanline(&mut self) -> bool {
        self.scanline = self.scanline.wrapping_add(1);
        let mut wrapped = false;
        if self.scanline >= LINES_PER_FRAME {
            self.scanline = 0;
            self.in_vblank = false;
            self.scroll_line_valid.fill(false);
            self.refresh_activity_flags();
            wrapped = true;
        }
        self.latch_line_state(self.scanline as usize);
        wrapped
    }

    #[cfg(test)]
    fn advance_scanline_for_test(&mut self) {
        self.advance_scanline();
    }

    fn zoom_step_value(raw: u16) -> usize {
        let value = (raw & 0x001F) as usize;
        value.max(1).min(32)
    }

    #[cfg(test)]
    fn scroll_for_scanline(&mut self) -> (usize, usize) {
        self.apply_pending_scroll();
        (self.scroll_x as usize, self.scroll_y as usize)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VdcWritePhase {
    Low,
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VdcReadPhase {
    Low,
    High,
}

const PSG_REG_COUNT: usize = 32;
const PSG_CHANNEL_COUNT: usize = 6;
const PSG_WAVE_SIZE: usize = 32;
const PSG_REG_TIMER_LO: usize = 0x18;
const PSG_REG_TIMER_HI: usize = 0x19;
const PSG_REG_TIMER_CTRL: usize = 0x1A;
const PSG_CTRL_ENABLE: u8 = 0x01;
const PSG_CTRL_IRQ_ENABLE: u8 = 0x02;
const PSG_STATUS_IRQ: u8 = 0x80;

#[derive(Clone, Copy, Default)]
struct PsgChannel {
    frequency: u16,
    waveform_index: u8,
    volume: u8,
    phase: u32,
    wave_pos: u8,
}

#[derive(Clone)]
struct Psg {
    regs: [u8; PSG_REG_COUNT],
    select: u8,
    accumulator: u32,
    irq_pending: bool,
    channels: [PsgChannel; PSG_CHANNEL_COUNT],
    waveform_ram: [u8; PSG_CHANNEL_COUNT * PSG_WAVE_SIZE],
}

impl Psg {
    fn new() -> Self {
        Self {
            regs: [0; PSG_REG_COUNT],
            select: 0,
            accumulator: 0,
            irq_pending: false,
            channels: [PsgChannel::default(); PSG_CHANNEL_COUNT],
            waveform_ram: [0; PSG_CHANNEL_COUNT * PSG_WAVE_SIZE],
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn write_address(&mut self, value: u8) {
        self.select = value;
    }

    fn write_data(&mut self, value: u8) {
        let index = self.select as usize;
        if index < PSG_REG_COUNT {
            self.regs[index] = value;
            if index == PSG_REG_TIMER_LO || index == PSG_REG_TIMER_HI {
                self.accumulator = 0;
            }
            if index == PSG_REG_TIMER_CTRL && value & PSG_CTRL_ENABLE == 0 {
                self.irq_pending = false;
            }
            self.update_channel_state(index, value);
        }
        if index >= PSG_REG_COUNT {
            self.write_wave_ram(index - PSG_REG_COUNT, value);
        }
        self.select = self.select.wrapping_add(1);
    }

    fn read_address(&self) -> u8 {
        self.select
    }

    fn read_data(&mut self) -> u8 {
        let index = self.select as usize;
        let value = if index < PSG_REG_COUNT {
            self.regs[index]
        } else {
            let wave_index = index - PSG_REG_COUNT;
            self.waveform_ram[wave_index % self.waveform_ram.len()]
        };
        self.select = self.select.wrapping_add(1);
        value
    }

    fn read_status(&mut self) -> u8 {
        let mut status = 0;
        if self.irq_pending {
            status |= PSG_STATUS_IRQ;
        }
        status
    }

    fn timer_period(&self) -> u16 {
        let lo = self.regs[PSG_REG_TIMER_LO] as u16;
        let hi = self.regs[PSG_REG_TIMER_HI] as u16;
        (hi << 8) | lo
    }

    fn enabled(&self) -> bool {
        let ctrl = self.regs[PSG_REG_TIMER_CTRL];
        self.timer_period() != 0 && (ctrl & PSG_CTRL_ENABLE != 0)
    }

    fn update_channel_state(&mut self, index: usize, value: u8) {
        let channel = index >> 2;
        if channel >= PSG_CHANNEL_COUNT {
            return;
        }
        let reg_in_channel = index & 0x03;
        let ch = &mut self.channels[channel];
        match reg_in_channel {
            0x00 => {
                ch.frequency = (ch.frequency & 0x0F00) | value as u16;
                ch.phase = 0;
            }
            0x01 => {
                ch.frequency = (ch.frequency & 0x00FF) | (((value & 0x0F) as u16) << 8);
                ch.phase = 0;
            }
            0x02 => {
                ch.waveform_index = value & 0x1F;
                ch.wave_pos = ch.waveform_index;
            }
            0x03 => {
                ch.volume = value & 0x1F;
            }
            _ => {}
        }
    }

    fn tick(&mut self, cycles: u32) -> bool {
        if !self.enabled() {
            return false;
        }
        if self.irq_pending {
            return false;
        }

        self.accumulator = self.accumulator.saturating_add(cycles);
        let period = self.timer_period() as u32;
        if period == 0 {
            return false;
        }
        if self.accumulator >= period {
            self.accumulator %= period.max(1);
            if self.regs[PSG_REG_TIMER_CTRL] & PSG_CTRL_IRQ_ENABLE != 0 {
                self.irq_pending = true;
                return true;
            }
        }
        false
    }

    fn acknowledge(&mut self) {
        self.irq_pending = false;
    }

    fn generate_sample(&mut self) -> i16 {
        self.advance_waveforms();
        let mut mix: i32 = 0;
        for (channel, state) in self.channels.iter().enumerate() {
            mix += self.sample_channel(channel, state) as i32;
        }
        (mix / PSG_CHANNEL_COUNT as i32) as i16
    }

    fn advance_waveforms(&mut self) {
        for ch in &mut self.channels {
            if ch.frequency == 0 {
                continue;
            }
            let phase = ch.phase.wrapping_add(ch.frequency as u32);
            let step = (phase >> 12) as u8;
            ch.phase = phase & 0x0FFF;
            if step != 0 {
                ch.wave_pos = (ch.wave_pos.wrapping_add(step)) & (PSG_WAVE_SIZE as u8 - 1);
            }
        }
    }

    fn sample_channel(&self, channel: usize, state: &PsgChannel) -> i16 {
        if state.frequency == 0 {
            return 0;
        }
        let base = channel * PSG_WAVE_SIZE;
        let offset = ((state.waveform_index as usize + state.wave_pos as usize)
            & (PSG_WAVE_SIZE - 1)) as usize;
        let wave_index = base + offset;
        let raw = self.waveform_ram[wave_index] as i16 - 0x10;
        raw * state.volume as i16
    }

    fn write_wave_ram(&mut self, addr: usize, value: u8) {
        let index = addr % self.waveform_ram.len();
        self.waveform_ram[index] = value & 0x1F;
    }
}

#[derive(Clone)]
struct Vce {
    palette: [u16; 0x200],
    control: u16,
    data_latch: u16,
    write_phase: VcePhase,
    read_phase: VcePhase,
    brightness: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VcePhase {
    Low,
    High,
}

impl Vce {
    fn new() -> Self {
        Self {
            palette: [0; 0x200],
            control: 0,
            data_latch: 0,
            write_phase: VcePhase::Low,
            read_phase: VcePhase::Low,
            brightness: 0x0F,
        }
    }

    fn reset(&mut self) {
        self.palette.fill(0);
        self.control = 0;
        self.data_latch = 0;
        self.write_phase = VcePhase::Low;
        self.read_phase = VcePhase::Low;
        self.brightness = 0x0F;
    }

    fn index(&self) -> usize {
        (self.control as usize) & 0x01FF
    }

    fn write_control_low(&mut self, value: u8) {
        self.control = (self.control & 0xFF00) | value as u16;
        self.read_phase = VcePhase::Low;
        self.write_phase = VcePhase::Low;
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("  VCE control low <= {:02X}", value);
    }

    fn write_control_high(&mut self, value: u8) {
        self.control = ((value as u16) << 8) | (self.control & 0x00FF);
        self.read_phase = VcePhase::Low;
        self.write_phase = VcePhase::Low;
        self.brightness = (value >> 4) & 0x0F;
        #[cfg(feature = "trace_hw_writes")]
        eprintln!(
            "  VCE control high <= {:02X} (brightness {:X})",
            value, self.brightness
        );
    }

    fn read_control_low(&self) -> u8 {
        (self.control & 0x00FF) as u8
    }

    fn read_control_high(&self) -> u8 {
        (self.control >> 8) as u8
    }

    fn write_data_low(&mut self, value: u8) {
        self.data_latch = (self.data_latch & 0xFF00) | value as u16;
        self.write_phase = VcePhase::High;
        if cfg!(debug_assertions) {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static LOGGED: AtomicUsize = AtomicUsize::new(0);
            let n = LOGGED.fetch_add(1, Ordering::Relaxed);
            if value != 0 || n < 64 {
                if n < 64 {
                    eprintln!("  VCE data low <= {:02X}", value);
                }
            }
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("  VCE data low <= {:02X}", value);
    }

    fn write_data_high(&mut self, value: u8) {
        if self.write_phase != VcePhase::High {
            // 想定外の順序（high が先）で来たときは low とみなしてラッチし、
            // 次のバイトを high として待つ。これで low が常に 0 になる症状を回避する。
            self.data_latch = (self.data_latch & 0xFF00) | value as u16;
            self.write_phase = VcePhase::High;
            return;
        }
        self.data_latch = (self.data_latch & 0x00FF) | ((value as u16) << 8);
        let idx = self.index();
        if let Some(slot) = self.palette.get_mut(idx) {
            *slot = self.data_latch;
        }
        if cfg!(debug_assertions) {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static LOGGED: AtomicUsize = AtomicUsize::new(0);
            let n = LOGGED.fetch_add(1, Ordering::Relaxed);
            if self.data_latch != 0 || n < 256 {
                if n < 256 {
                    eprintln!(
                        "  VCE palette write idx {:03X} = {:04X}",
                        idx, self.data_latch
                    );
                }
            }
        }
        #[cfg(feature = "trace_hw_writes")]
        eprintln!("  VCE palette[{idx:03X}] = {:04X}", self.data_latch);
        self.increment_index();
        self.write_phase = VcePhase::Low;
    }

    fn read_data_low(&mut self) -> u8 {
        if self.read_phase == VcePhase::Low {
            self.data_latch = self.palette.get(self.index()).copied().unwrap_or(0);
        }
        self.read_phase = VcePhase::High;
        (self.data_latch & 0x00FF) as u8
    }

    fn read_data_high(&mut self) -> u8 {
        if self.read_phase == VcePhase::Low {
            self.data_latch = self.palette.get(self.index()).copied().unwrap_or(0);
        }
        let value = (self.data_latch >> 8) as u8;
        self.increment_index();
        self.read_phase = VcePhase::Low;
        value
    }

    fn increment_index(&mut self) {
        let next = (self.index() + 1) & 0x01FF;
        self.control = (self.control & !0x01FF) | (next as u16);
    }

    #[inline]
    fn brightness_override() -> Option<u8> {
        use std::sync::OnceLock;
        static OVERRIDE: OnceLock<Option<u8>> = OnceLock::new();
        *OVERRIDE.get_or_init(|| {
            std::env::var("PCE_FORCE_BRIGHTNESS")
                .ok()
                .and_then(|s| u8::from_str_radix(&s, 16).ok())
                .map(|v| v & 0x0F)
        })
    }

    #[cfg(test)]
    fn palette_word(&self, index: usize) -> u16 {
        self.palette.get(index).copied().unwrap_or(0)
    }

    fn palette_rgb(&self, index: usize) -> u32 {
        let raw = self.palette.get(index).copied().unwrap_or(0);
        let blue = (raw & 0x000F) as u8;
        let green = ((raw >> 4) & 0x000F) as u8;
        let red = ((raw >> 8) & 0x000F) as u8;

        let scale = Self::brightness_override()
            .map(|v| v as u16)
            .unwrap_or(self.brightness as u16);
        let component = |value: u8| -> u8 {
            if scale == 0 {
                return 0;
            }
            let expanded = (value as u16 * 255) / 0x0F;
            let scaled = (expanded * scale) / 0x0F;
            scaled.min(255) as u8
        };

        let r = component(red);
        let g = component(green);
        let b = component(blue);
        ((r as u32) << 16) | ((g as u32) << 8) | b as u32
    }
}

impl Timer {
    fn new() -> Self {
        Self {
            reload: 0,
            counter: 0,
            prescaler: 0,
            enabled: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn write_reload(&mut self, value: u8) {
        self.reload = value & 0x7F;
    }

    fn read_counter(&self) -> u8 {
        self.counter & 0x7F
    }

    fn write_control(&mut self, value: u8) {
        let start = value & TIMER_CONTROL_START != 0;
        if start && !self.enabled {
            self.enabled = true;
            self.counter = self.reload;
            self.prescaler = 0;
        } else if !start {
            self.enabled = false;
        }
    }

    fn control(&self) -> u8 {
        if self.enabled { TIMER_CONTROL_START } else { 0 }
    }

    fn tick(&mut self, cycles: u32, high_speed: bool) -> bool {
        if !self.enabled {
            return false;
        }

        let divider = if high_speed { 1024 } else { 256 };
        self.prescaler += cycles;
        let mut fired = false;

        while self.prescaler >= divider as u32 {
            self.prescaler -= divider as u32;
            if self.counter == 0 {
                self.counter = self.reload;
                fired = true;
            } else {
                self.counter = self.counter.wrapping_sub(1) & 0x7F;
            }
        }

        fired
    }
}

impl IoPort {
    fn new() -> Self {
        Self {
            output: 0,
            direction: 0,
            enable: 0,
            select: 0,
            input: 0xFF,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn read(&self, offset: usize) -> Option<u8> {
        match offset & 0x03FF {
            0x0000 => Some(self.output),
            0x0002 => Some(self.direction),
            0x0004 => Some(self.latched_input()),
            0x0005 => Some(self.enable),
            0x0006 => Some(self.select),
            _ => None,
        }
    }

    fn write(&mut self, offset: usize, value: u8) -> bool {
        match offset & 0x03FF {
            0x0000 => {
                self.output = value;
                true
            }
            0x0002 => {
                self.direction = value;
                true
            }
            0x0004 => {
                self.input = value;
                true
            }
            0x0005 => {
                self.enable = value;
                true
            }
            0x0006 => {
                self.select = value;
                true
            }
            _ => false,
        }
    }

    fn latched_input(&self) -> u8 {
        (self.input & !self.direction) | (self.output & self.direction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: 実機では A2=1 がデータ、A2=0 がアドレス（制御）とされる資料もある。
    // ここでは制御=0x0400/01、データ=0x0402/03 とするが、切替検証しやすいようにまとめておく。
    const VCE_CONTROL_ADDR: u16 = 0x0400;
    const VCE_CONTROL_HIGH_ADDR: u16 = 0x0401;
    const VCE_DATA_ADDR: u16 = 0x0402;
    const VCE_DATA_HIGH_ADDR: u16 = 0x0403;
    const PSG_ADDR_REG: u16 = 0x0800;
    const PSG_WRITE_REG: u16 = 0x0801;
    const PSG_READ_REG: u16 = 0x0802;
    const PSG_STATUS_REG: u16 = 0x0803;
    const JOYPAD_BASE_ADDR: u16 = 0x1000;
    const IRQ_TIMER_BASE: u16 = 0x1400;
    const CPU_IRQ_MASK: u16 = 0xFF12;
    const CPU_IRQ_STATUS: u16 = 0xFF13;
    const VDC_CTRL_DISPLAY_FULL: u16 = VDC_CTRL_ENABLE_BACKGROUND
        | VDC_CTRL_ENABLE_BACKGROUND_LEGACY
        | VDC_CTRL_ENABLE_SPRITES
        | VDC_CTRL_ENABLE_SPRITES_LEGACY;

    fn set_vdc_control(bus: &mut Bus, value: u16) {
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, (value & 0x00FF) as u8);
        bus.write_st_port(2, (value >> 8) as u8);
    }

    fn prepare_bus_for_zoom() -> Bus {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        const MAP_WIDTH: usize = 32;
        for col in 0..MAP_WIDTH {
            let tile_id = 0x100 + col;
            let palette_bank = (col & 0x0F) as usize;
            bus.vdc.vram[col] = ((tile_id as u16) & 0x07FF) | ((palette_bank as u16) << 12);
            let base = (tile_id * 16) & 0x7FFF;
            for row in 0..8 {
                bus.vdc.vram[(base + row) & 0x7FFF] = 0x00FF;
                bus.vdc.vram[(base + row + 8) & 0x7FFF] = 0x0000;
            }
        }

        for bank in 0..16 {
            let colour = (bank as u16) * 0x041;
            bus.vce.palette[(bank << 4) | 1] = colour;
        }

        bus
    }

    fn render_zoom_pair(port: u8, zoom_value: u8) -> ([u32; FRAME_WIDTH], [u32; FRAME_WIDTH]) {
        let mut baseline = prepare_bus_for_zoom();
        baseline.render_frame_from_vram();
        let mut zoomed = prepare_bus_for_zoom();
        zoomed.write_st_port(0, port);
        zoomed.write_st_port(1, zoom_value);
        zoomed.write_st_port(2, 0x00);
        zoomed.render_frame_from_vram();

        let mut base_line = [0u32; FRAME_WIDTH];
        let mut zoom_line = [0u32; FRAME_WIDTH];
        base_line.copy_from_slice(&baseline.framebuffer[0..FRAME_WIDTH]);
        zoom_line.copy_from_slice(&zoomed.framebuffer[0..FRAME_WIDTH]);
        (base_line, zoom_line)
    }

    fn prepare_bus_for_vertical_zoom() -> Bus {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        const MAP_WIDTH: usize = 32;
        for row in 0..32 {
            let tile_id = 0x200 + row * MAP_WIDTH;
            let palette_bank = (row & 0x0F) as usize;
            for col in 0..MAP_WIDTH {
                let idx = row * MAP_WIDTH + col;
                bus.vdc.vram[idx] = ((tile_id as u16) & 0x07FF) | ((palette_bank as u16) << 12);
            }
            let base = (tile_id * 16) & 0x7FFF;
            for line in 0..8 {
                bus.vdc.vram[(base + line) & 0x7FFF] = 0x00FF;
                bus.vdc.vram[(base + line + 8) & 0x7FFF] = 0x0000;
            }
        }

        for bank in 0..16 {
            let colour = 0x0100 | ((bank as u16) * 0x021);
            bus.vce.palette[(bank << 4) | 1] = colour;
        }

        bus
    }

    fn render_vertical_zoom_pair(port: u8, zoom_value: u8) -> (Vec<u32>, Vec<u32>) {
        let mut baseline = prepare_bus_for_vertical_zoom();
        baseline.render_frame_from_vram();
        let mut zoomed = prepare_bus_for_vertical_zoom();
        zoomed.write_st_port(0, port);
        zoomed.write_st_port(1, zoom_value);
        zoomed.write_st_port(2, 0x00);
        zoomed.render_frame_from_vram();
        (baseline.framebuffer.clone(), zoomed.framebuffer.clone())
    }

    #[test]
    fn load_and_bank_switch_rom() {
        let mut bus = Bus::new();
        bus.load(0x0000, &[0xAA, 0xBB]);
        assert_eq!(bus.read(0x0000), 0xAA);

        bus.load_rom_image(vec![0x10; PAGE_SIZE * 2]);
        bus.map_bank_to_rom(4, 1);
        assert_eq!(bus.read(0x8000), 0x10);

        bus.write(0x8000, 0x77); // ignored because ROM
        assert_eq!(bus.read(0x8000), 0x10);

        bus.map_bank_to_ram(4, 0);
        bus.write(0x8000, 0x12);
        assert_eq!(bus.read(0x8000), 0x12);
    }

    #[test]
    fn mpr_mirrors_apply_across_high_page() {
        let mut bus = Bus::new();
        bus.load_rom_image(vec![0x55; PAGE_SIZE * 2]);

        // 0xFF95 mirrors MPR5
        bus.write(0xFF95, (bus.total_ram_pages() + 1) as u8);
        assert_eq!(bus.mpr(5), (bus.total_ram_pages() + 1) as u8);

        // ROM page 1 is filled with 0x55
        assert_eq!(bus.read(0xA000), 0x55);

        // Reading from a mirror location returns the same register value.
        assert_eq!(bus.read(0xFFAD), bus.mpr(5));
    }

    #[test]
    fn io_port_direction_masks_input() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);
        bus.set_joypad_input(0x5A);
        bus.write(JOYPAD_BASE_ADDR + 0x02, 0x0F);
        bus.write(JOYPAD_BASE_ADDR, 0xA5);
        let latched = bus.read(JOYPAD_BASE_ADDR + 0x04);
        assert_eq!(latched, (0x5A & 0xF0) | (0xA5 & 0x0F));
    }

    #[test]
    fn st_ports_store_values() {
        let mut bus = Bus::new();
        bus.write_st_port(0, 0x12);
        bus.write_st_port(1, 0x34);
        bus.write_st_port(2, 0x56);
        assert_eq!(bus.st_port(0), 0x12);
        assert_eq!(bus.st_port(1), 0x34);
        assert_eq!(bus.st_port(2), 0x56);
    }

    #[test]
    fn io_registers_round_trip_and_reset() {
        let mut bus = Bus::new();
        assert_eq!(bus.read(0xFF20), 0);
        assert_eq!(bus.read(0xFF7F), 0);

        bus.write(0xFF20, 0xAA);
        assert_eq!(bus.read(0xFF20), 0xAA);
        bus.write(0xFF7F, 0x55);
        assert_eq!(bus.read(0xFF7F), 0x55);

        bus.write_io(HW_CPU_CTRL_BASE + 0x30, 0x42);
        assert_eq!(bus.read(0xFF30), 0x42);

        bus.clear();
        assert_eq!(bus.read(0xFF20), 0x00);
        assert_eq!(bus.read(0xFF30), 0x00);
        assert_eq!(bus.read(0xFF7F), 0x00);
    }

    #[test]
    fn timer_borrow_sets_request_bit() {
        let mut bus = Bus::new();
        bus.write(0xFF10, 0x02); // reload value
        bus.write(0xFF11, TIMER_CONTROL_START);

        let fired = bus.tick(1024u32 * 3, true);
        assert!(fired);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_TIMER, IRQ_REQUEST_TIMER);

        bus.write(0xFF13, IRQ_REQUEST_TIMER);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_TIMER, 0);
    }

    #[test]
    fn hardware_page_irq_registers_alias_cpu_space() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(IRQ_TIMER_BASE + 0x02, 0xFF);
        assert_eq!(
            bus.read(CPU_IRQ_MASK),
            IRQ_DISABLE_IRQ2 | IRQ_DISABLE_IRQ1 | IRQ_DISABLE_TIMER
        );

        bus.write(CPU_IRQ_MASK, 0x00);
        bus.write(IRQ_TIMER_BASE + 0x03, IRQ_REQUEST_TIMER);
        assert_eq!(bus.read(CPU_IRQ_STATUS) & IRQ_REQUEST_TIMER, 0);
    }

    #[test]
    fn cart_ram_banks_map_into_memory_space() {
        let mut bus = Bus::new();
        bus.configure_cart_ram(PAGE_SIZE * 2);

        let cart_base = 0x80u8;
        bus.set_mpr(2, cart_base);
        bus.write(0x4000, 0x5A);
        assert_eq!(bus.cart_ram[0], 0x5A);
        assert_eq!(bus.read(0x4000), 0x5A);

        bus.set_mpr(2, cart_base + 1);
        bus.write(0x4000, 0xCC);
        assert_eq!(bus.cart_ram[PAGE_SIZE], 0xCC);
        assert_eq!(bus.read(0x4000), 0xCC);

        bus.set_mpr(2, cart_base);
        assert_eq!(bus.read(0x4000), 0x5A);
    }

    #[test]
    fn cart_ram_load_and_snapshot_round_trip() {
        let mut bus = Bus::new();
        bus.configure_cart_ram(PAGE_SIZE);
        let pattern = vec![0xAB; PAGE_SIZE];
        assert!(bus.load_cart_ram(&pattern).is_ok());
        assert_eq!(bus.cart_ram().unwrap()[0], 0xAB);
        let cart_base = 0x80u8;
        bus.set_mpr(2, cart_base);
        let cart_addr = 0x4000u16;
        assert_eq!(bus.read(cart_addr), 0xAB);

        if let Some(data) = bus.cart_ram_mut() {
            data.fill(0x11);
        }
        assert_eq!(bus.read(cart_addr), 0x11);
    }

    #[test]
    fn sprite_priority_respects_background_mask() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);
        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        const BG_TILE_ID: usize = 200;
        const SPRITE_TILE_ID: usize = 201;
        const BG_PALETTE_BANK: usize = 1;
        const SPRITE_PALETTE_BANK: usize = 2;

        for entry in bus.vdc.vram.iter_mut().take(32 * 32) {
            *entry = ((BG_TILE_ID as u16) & 0x07FF) | ((BG_PALETTE_BANK as u16) << 12);
        }

        let bg_base = BG_TILE_ID * 16;
        for row in 0..8 {
            bus.vdc.vram[bg_base + row] = 0xFFFF;
            bus.vdc.vram[bg_base + 8 + row] = 0xFFFF;
        }

        let sprite_base = SPRITE_TILE_ID * 16;
        for row in 0..8 {
            bus.vdc.vram[sprite_base + row] = 0x0080;
            bus.vdc.vram[sprite_base + 8 + row] = 0x0000;
        }

        bus.vce.palette[0x1F] = 0x001F;
        bus.vce.palette[0x21] = 0x03E0;

        bus.render_frame_from_vram();
        let bg_colour = bus.framebuffer[0];
        assert_ne!(bg_colour, 0);
        assert!(bus.bg_opaque[0]);

        let satb_index = 0;
        let y_word = ((0 + 64) & 0x03FF) as u16;
        let x_word = ((0 + 32) & 0x03FF) as u16;
        bus.vdc.satb[satb_index] = y_word;
        bus.vdc.satb[satb_index + 1] = SPRITE_TILE_ID as u16;
        bus.vdc.satb[satb_index + 2] = 0x0080 | (SPRITE_PALETTE_BANK as u16);
        bus.vdc.satb[satb_index + 3] = x_word;

        bus.render_frame_from_vram();
        assert_eq!(bus.framebuffer[0], bg_colour);

        bus.vdc.satb[satb_index + 2] &= !0x0080;
        bus.render_frame_from_vram();
        let sprite_colour = bus.vce.palette_rgb(0x21);
        assert_eq!(bus.framebuffer[0], sprite_colour);
    }

    fn write_constant_sprite_tile(bus: &mut Bus, tile_index: usize, value: u8) {
        let base = (tile_index * 16) & 0x7FFF;
        let plane0 = if value & 0x01 != 0 { 0x00FF } else { 0x0000 };
        let plane1 = if value & 0x02 != 0 { 0xFF00 } else { 0x0000 };
        let plane2 = if value & 0x04 != 0 { 0x00FF } else { 0x0000 };
        let plane3 = if value & 0x08 != 0 { 0xFF00 } else { 0x0000 };
        for row in 0..8 {
            bus.vdc.vram[(base + row) & 0x7FFF] = plane0 | plane1;
            bus.vdc.vram[(base + row + 8) & 0x7FFF] = plane2 | plane3;
        }
    }

    #[test]
    fn sprites_render_when_background_disabled() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.vce.palette[0x00] = 0x0000;
        bus.vce.palette[0x01] = 0x7C00;

        write_constant_sprite_tile(&mut bus, 0, 0x01);

        let sprite_y = 32;
        let sprite_x = 24;
        bus.vdc.satb[0] = ((sprite_y + 64) & 0x03FF) as u16;
        bus.vdc.satb[1] = 0;
        bus.vdc.satb[2] = 0x0000;
        bus.vdc.satb[3] = ((sprite_x + 32) & 0x03FF) as u16;

        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x40);
        bus.write_st_port(2, 0x00);

        bus.render_frame_from_vram();

        let background_colour = bus.vce.palette_rgb(0x00);
        assert_eq!(bus.framebuffer[0], background_colour);

        let sprite_index = sprite_y as usize * FRAME_WIDTH + sprite_x as usize;
        let sprite_colour = bus.vce.palette_rgb(0x01);
        assert_eq!(bus.framebuffer[sprite_index], sprite_colour);
        assert!(
            bus.framebuffer.iter().any(|&pixel| pixel == sprite_colour),
            "expected sprite colour to appear in framebuffer"
        );
    }

    #[test]
    fn sprite_double_width_draws_all_columns() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        for (idx, &palette_value) in [0x01u16, 0x111u16, 0x222u16, 0x333u16].iter().enumerate() {
            bus.vce.palette[(idx + 1) as usize] = palette_value;
        }

        for (tile, value) in (0..4).zip(1u8..) {
            write_constant_sprite_tile(&mut bus, tile, value);
        }

        let sprite_base = 0;
        let sprite_y = 32;
        let sprite_x = 24;
        bus.vdc.satb[sprite_base] = ((sprite_y + 64) & 0x03FF) as u16;
        bus.vdc.satb[sprite_base + 1] = 0;
        bus.vdc.satb[sprite_base + 2] = 0x0100;
        bus.vdc.satb[sprite_base + 3] = ((sprite_x + 32) & 0x03FF) as u16;

        bus.render_frame_from_vram();

        let row_start = sprite_y * FRAME_WIDTH + sprite_x;
        let colours = [
            bus.framebuffer[row_start],
            bus.framebuffer[row_start + 8],
            bus.framebuffer[row_start + 16],
            bus.framebuffer[row_start + 24],
        ];
        for (idx, &colour) in colours.iter().enumerate() {
            let expected = bus.vce.palette_rgb((idx + 1) as usize);
            assert_eq!(colour, expected, "column {} did not use tile {}", idx, idx);
        }
    }

    #[test]
    fn sprite_scanline_overflow_sets_status() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        const TILE_ID: usize = 0x400;
        let tile_base = TILE_ID * 16;
        for row in 0..8 {
            bus.vdc.vram[(tile_base + row) & 0x7FFF] = 0xFFFF;
            bus.vdc.vram[(tile_base + row + 8) & 0x7FFF] = 0xFFFF;
        }

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        let y_pos = 48;
        for sprite in 0..17 {
            let base = sprite * 4;
            let x_pos = sprite as i32 * 8;
            bus.vdc.satb[base] = ((y_pos + 64) & 0x03FF) as u16;
            bus.vdc.satb[base + 1] = TILE_ID as u16;
            bus.vdc.satb[base + 2] = 0x0000;
            bus.vdc.satb[base + 3] = ((x_pos + 32) & 0x03FF) as u16;
        }

        bus.render_frame_from_vram();
        let max_count = bus
            .sprite_line_counts_for_test()
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        assert_eq!(max_count, 16);
        assert_ne!(bus.vdc.status_bits() & VDC_STATUS_OR, 0);

        let bg_colour = bus.vce.palette_rgb(0);
        let seventeenth_x = 16 * 8;
        let pixel = bus.framebuffer[y_pos * FRAME_WIDTH + seventeenth_x];
        assert_eq!(pixel, bg_colour);

        let overflow_sprite = 16 * 4;
        bus.vdc.satb[overflow_sprite] = 0;
        bus.vdc.satb[overflow_sprite + 1] = 0;
        bus.vdc.satb[overflow_sprite + 2] = 0;
        bus.vdc.satb[overflow_sprite + 3] = 0;

        bus.render_frame_from_vram();
        assert_eq!(bus.vdc.status_bits() & VDC_STATUS_OR, 0);

        let pixel_after = bus.framebuffer[y_pos * FRAME_WIDTH + seventeenth_x];
        assert_eq!(pixel_after, bg_colour);
    }

    #[test]
    fn sprite_size_scaling_plots_full_extent() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        const BASE_TILE: usize = 0x300;
        const WIDTH_UNITS: usize = 2;
        const HEIGHT_UNITS: usize = 2;
        const WIDTH_TILES: usize = WIDTH_UNITS * 2;
        const HEIGHT_TILES: usize = HEIGHT_UNITS * 2;

        for tile in 0..(WIDTH_TILES * HEIGHT_TILES) {
            let offset = (BASE_TILE + tile) * 16;
            for row in 0..8 {
                bus.vdc.vram[(offset + row) & 0x7FFF] = 0xFFFF;
                bus.vdc.vram[(offset + row + 8) & 0x7FFF] = 0xFFFF;
            }
        }

        let sprite_colour = 0x7C00;
        bus.vce.palette[0x2F] = sprite_colour;

        let x_pos = 40;
        let y_pos = 32;
        let satb_index = 0;
        bus.vdc.satb[satb_index] = ((y_pos + 64) & 0x03FF) as u16 | 0x1000;
        bus.vdc.satb[satb_index + 1] = BASE_TILE as u16;
        bus.vdc.satb[satb_index + 2] = 0x0100 | 0x0002;
        bus.vdc.satb[satb_index + 3] = ((x_pos + 32) & 0x03FF) as u16;

        bus.render_frame_from_vram();

        let colour = bus.vce.palette_rgb(0x2F);
        let idx = (y_pos + HEIGHT_UNITS * 16 - 1) * FRAME_WIDTH + (x_pos + WIDTH_UNITS * 16 - 1);
        assert_eq!(bus.framebuffer[idx], colour);
    }

    #[test]
    fn sprite_quad_height_plots_bottom_row() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        const BASE_TILE: usize = 0x320;
        const TILES_WIDE: usize = 2;
        const TILES_HIGH: usize = 8;

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        for tile in 0..(TILES_WIDE * TILES_HIGH) {
            let offset = (BASE_TILE + tile) * 16;
            for row in 0..8 {
                bus.vdc.vram[(offset + row) & 0x7FFF] = 0xFFFF;
                bus.vdc.vram[(offset + row + 8) & 0x7FFF] = 0xFFFF;
            }
        }

        let sprite_colour = 0x03FF;
        bus.vce.palette[0x0F] = sprite_colour;

        let x_pos = 24;
        let y_pos = 40;
        let satb_index = 0;
        bus.vdc.satb[satb_index] = ((y_pos + 64) & 0x03FF) as u16 | 0x3000;
        bus.vdc.satb[satb_index + 1] = BASE_TILE as u16;
        bus.vdc.satb[satb_index + 2] = 0x0000;
        bus.vdc.satb[satb_index + 3] = ((x_pos + 32) & 0x03FF) as u16;

        bus.render_frame_from_vram();

        let expected = bus.vce.palette_rgb(0x0F);
        let drawn_pixels = bus
            .framebuffer
            .iter()
            .filter(|&&pixel| pixel == expected)
            .count();
        assert!(drawn_pixels > 0);
        let top_row = &bus.framebuffer[y_pos * FRAME_WIDTH..(y_pos + 1) * FRAME_WIDTH];
        assert!(top_row.iter().any(|&pixel| pixel == expected));
        let bottom_row = &bus.framebuffer[(y_pos + 63) * FRAME_WIDTH..(y_pos + 64) * FRAME_WIDTH];
        assert!(bottom_row.iter().any(|&pixel| pixel == expected));
    }

    #[test]
    fn scroll_registers_latch_on_scanline_boundary() {
        let mut vdc = Vdc::new();
        let (x0, y0) = vdc.scroll_for_scanline();
        assert_eq!(x0, 0);
        assert_eq!(y0, 0);

        vdc.write_select(0x07);
        vdc.write_data_low(0x34);
        vdc.write_data_high(0x12);
        let (x1, y1) = vdc.scroll_for_scanline();
        assert_eq!(x1, 0x1234 & 0x03FF);
        assert_eq!(y1, 0);

        vdc.write_select(0x08);
        vdc.write_data_low(0x78);
        vdc.write_data_high(0x05);
        let (x2, y2) = vdc.scroll_for_scanline();
        assert_eq!(x2, x1);
        assert_eq!(y2, 0x0578 & 0x01FF);

        let (x3, y3) = vdc.scroll_for_scanline();
        assert_eq!(x3, x2);
        assert_eq!(y3, y2);
    }

    #[test]
    fn scroll_writes_apply_on_next_visible_scanline() {
        let mut vdc = Vdc::new();
        vdc.advance_scanline_for_test();
        let (x0, _) = vdc.scroll_values_for_line(0);
        assert_eq!(x0, 0);

        vdc.write_select(0x07);
        vdc.write_data_low(0x34);
        vdc.write_data_high(0x12);

        let (x_still, _) = vdc.scroll_values_for_line(0);
        assert_eq!(x_still, 0);

        vdc.advance_scanline_for_test();
        let (x1, _) = vdc.scroll_values_for_line(1);
        assert_eq!(x1, 0x1234 & 0x03FF);

        let (x_now, _) = vdc.scroll_for_scanline();
        assert_eq!(x_now, 0x1234 & 0x03FF);
    }

    #[test]
    fn background_horizontal_zoom_scales_source() {
        let mut baseline = prepare_bus_for_zoom();
        baseline.render_frame_from_vram();
        let base0 = baseline.framebuffer[0];
        let base8 = baseline.framebuffer[8];
        let base16 = baseline.framebuffer[16];
        assert_ne!(base0, base8);
        assert_ne!(base8, base16);

        let mut zoomed = prepare_bus_for_zoom();
        zoomed.write_st_port(0, 0x0A);
        zoomed.write_st_port(1, 0x08);
        zoomed.write_st_port(2, 0x00);
        zoomed.render_frame_from_vram();
        let zoom0 = zoomed.framebuffer[0];
        let zoom16 = zoomed.framebuffer[16];
        let zoom32 = zoomed.framebuffer[32];
        assert_eq!(zoom0, base0);
        assert_eq!(zoom16, base8);
        assert_eq!(zoom32, base16);
    }

    #[test]
    fn background_horizontal_zoom_shrinks_source() {
        let (baseline, zoomed) = render_zoom_pair(0x0A, 0x18);
        assert_eq!(zoomed[0], baseline[0]);
        assert_eq!(zoomed[16], baseline[24]);
    }

    #[test]
    fn background_horizontal_zoom_extreme_zoom_in() {
        let (baseline, zoomed) = render_zoom_pair(0x0A, 0x01);
        let colour = baseline[0];
        for x in 0..16 {
            assert_eq!(zoomed[x], colour);
        }
    }

    #[test]
    fn background_horizontal_zoom_extreme_shrink() {
        let (baseline, zoomed) = render_zoom_pair(0x0A, 0x1F);
        assert_eq!(zoomed[0], baseline[0]);
        assert_eq!(zoomed[8], baseline[15]);
        assert_eq!(zoomed[16], baseline[31]);
    }

    #[test]
    fn background_tile_flip_bits_are_respected() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        const TILE_ID: usize = 0x180;
        let tile_entry = (TILE_ID as u16) & 0x03FF;
        bus.vdc.vram[0] = tile_entry;
        bus.vdc.vram[1] = tile_entry | 0x0400;
        bus.vdc.vram[64] = tile_entry | 0x0800;
        bus.vdc.vram[65] = tile_entry | 0x0C00;

        let tile_base = (TILE_ID * 16) & 0x7FFF;
        bus.vdc.vram[(tile_base) & 0x7FFF] = 0x0080;
        for row in 1..8 {
            bus.vdc.vram[(tile_base + row) & 0x7FFF] = 0;
        }
        for row in 0..8 {
            bus.vdc.vram[(tile_base + row + 8) & 0x7FFF] = 0;
        }

        bus.vce.palette[0x01] = 0x7C00;

        bus.render_frame_from_vram();
        let colour = bus.vce.palette_rgb(0x01);
        let bg = bus.vce.palette_rgb(0x00);

        assert_eq!(bus.framebuffer[0], colour);
        assert_eq!(bus.framebuffer[8], bg);
        assert_eq!(bus.framebuffer[15], colour);
        let v_top = 16 * FRAME_WIDTH;
        let v_bottom = (16 + 7) * FRAME_WIDTH;
        assert_eq!(bus.framebuffer[v_top], bg);
        assert_eq!(bus.framebuffer[v_bottom], colour);
        let hv_index = (16 + 7) * FRAME_WIDTH + 15;
        assert_eq!(bus.framebuffer[hv_index], colour);
        assert_eq!(bus.framebuffer[(16 + 7) * FRAME_WIDTH + 8], bg);
    }

    #[test]
    fn background_priority_overrides_sprite_pixels() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        bus.write(VCE_CONTROL_ADDR, 0x10);
        bus.write(VCE_DATA_ADDR, 0x3F);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        bus.write(VCE_CONTROL_ADDR, 0x20);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x3F);

        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x10);
        bus.write_st_port(2, 0x00);

        let tile_index = 0x0100u16;
        let priority_entry = tile_index | 0x1000 | 0x8000;
        let addr_priority = bus.vdc.map_entry_address_for_test(0, 0) as u16;
        write_vram_word(&mut bus, addr_priority, priority_entry);

        let addr_plain = bus.vdc.map_entry_address_for_test(0, 1) as u16;
        write_vram_word(&mut bus, addr_plain, tile_index | 0x1000);

        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            write_vram_word(&mut bus, tile_addr + offset as u16, 0x0000);
        }

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        bus.render_frame_from_vram();
        assert!(bus.bg_priority[0]);
        assert!(!bus.bg_priority[8]);
    }

    #[test]
    fn background_vertical_zoom_scales_source() {
        let (baseline, zoomed) = render_vertical_zoom_pair(0x0B, 0x08);
        assert_ne!(baseline[0], baseline[8 * FRAME_WIDTH]);
        assert_ne!(baseline[8 * FRAME_WIDTH], baseline[16 * FRAME_WIDTH]);
        assert_eq!(zoomed[0], baseline[0]);
        assert_eq!(zoomed[16 * FRAME_WIDTH], baseline[8 * FRAME_WIDTH]);
        assert_eq!(zoomed[32 * FRAME_WIDTH], baseline[16 * FRAME_WIDTH]);
    }

    #[test]
    fn background_vertical_zoom_extreme_zoom_in() {
        let (baseline, zoomed) = render_vertical_zoom_pair(0x0B, 0x01);
        let colour = baseline[0];
        for y in 0..16 {
            assert_eq!(zoomed[y * FRAME_WIDTH], colour);
        }
    }

    #[test]
    fn background_vertical_zoom_extreme_shrink() {
        let (baseline, zoomed) = render_vertical_zoom_pair(0x0B, 0x1F);
        assert_eq!(zoomed[0], baseline[0]);
        assert_eq!(zoomed[8 * FRAME_WIDTH], baseline[15 * FRAME_WIDTH]);
        assert_eq!(zoomed[16 * FRAME_WIDTH], baseline[31 * FRAME_WIDTH]);
    }

    #[test]
    fn timer_disable_masks_irq_line() {
        let mut bus = Bus::new();
        bus.write(0xFF10, 0x01);
        bus.write(0xFF12, IRQ_DISABLE_TIMER);
        bus.write(0xFF11, TIMER_CONTROL_START);

        let fired = bus.tick(1024u32 * 2, true);
        assert!(!fired);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_TIMER, IRQ_REQUEST_TIMER);

        bus.write(0xFF12, 0x00);
        assert!(bus.tick(0, true));
        bus.write(0xFF13, IRQ_REQUEST_TIMER);
        assert!(!bus.tick(0, true));
    }

    #[test]
    fn timer_uses_slow_clock_divider() {
        let mut bus = Bus::new();
        bus.write(0xFF10, 0x00);
        bus.write(0xFF11, TIMER_CONTROL_START);

        let fired = bus.tick(256u32, false);
        assert!(fired);
    }

    #[test]
    fn hardware_page_routes_vdc_registers() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write_st_port(0, 0x05); // select control register
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);

        assert_eq!(bus.vdc_register(5), Some(0x0008));
    }

    #[test]
    fn io_space_mirror_routes_vdc_and_vce() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);
        bus.set_mpr(1, 0xFF);

        // VCE palette write via 0x2000-mirrored address space.
        bus.write(0x2400, 0x00); // control low
        bus.write(0x2401, 0x00); // control high (also sets brightness)
        bus.write(0x2402, 0x56); // data low
        bus.write(0x2403, 0x34); // data high
        assert_eq!(bus.vce_palette_word(0x0000), 0x3456);

        // VDC register select/data via mirrored offsets inside 0x0000-0x03FF.
        bus.write(0x2201, 0x05); // select control register (odd address mirror)
        assert_eq!(bus.st_port(0), 0x05);

        // Use a higher-offset mirror (0x2202/0x2203) to exercise the 0x100-spaced mirrors.
        bus.write(0x2202, 0xAA); // low byte (ST1 mirror)
        bus.write(0x2203, 0x00); // high byte via ST2 mirror
        assert_eq!(bus.vdc_register(5), Some(0x00AA));
    }

    #[test]
    fn hardware_page_status_read_clears_irq() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Enable VBlank interrupt and raise the status flag.
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);
        bus.vdc_set_status_for_test(VDC_STATUS_VBL);
        assert_ne!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);

        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_VBL != 0);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
        assert_eq!(bus.read_io(0x00) & VDC_STATUS_VBL, 0);
    }

    #[test]
    fn vce_palette_write_and_read_round_trip() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Select palette index 0x0010.
        bus.write(VCE_CONTROL_ADDR, 0x10);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0x00);

        bus.write(VCE_DATA_ADDR, 0x34);
        bus.write(VCE_DATA_HIGH_ADDR, 0x12);

        assert_eq!(bus.vce_palette_word(0x0010), 0x1234);

        // Reading back should return the stored value and advance the index.
        bus.write(VCE_CONTROL_ADDR, 0x10);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0x00);
        let lo = bus.read(VCE_DATA_ADDR);
        let hi = bus.read(VCE_DATA_HIGH_ADDR);
        assert_eq!(lo, 0x34);
        assert_eq!(hi, 0x12);
        assert_eq!(bus.vce_palette_word(0x0011), 0);
    }

    #[test]
    fn vce_sequential_writes_auto_increment_index() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0x00);

        for i in 0..4u16 {
            let value = 0x1000 | i;
            bus.write(VCE_DATA_ADDR, (value & 0x00FF) as u8);
            bus.write(VCE_DATA_HIGH_ADDR, (value >> 8) as u8);
        }

        assert_eq!(bus.vce_palette_word(0), 0x1000);
        assert_eq!(bus.vce_palette_word(1), 0x1001);
        assert_eq!(bus.vce_palette_word(2), 0x1002);
        assert_eq!(bus.vce_palette_word(3), 0x1003);
    }

    #[test]
    fn hardware_page_psg_accesses_data_ports() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);
        bus.write(PSG_ADDR_REG, 0x02);
        bus.write(PSG_WRITE_REG, 0x7F);
        bus.write(PSG_ADDR_REG, 0x02);
        assert_eq!(bus.read(PSG_READ_REG), 0x7F);
        assert_eq!(bus.read(PSG_STATUS_REG) & PSG_STATUS_IRQ, 0);
    }

    #[test]
    fn vce_palette_rgb_applies_brightness_and_channels() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Set brightness to mid-level (0x8) and index to zero.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0x80);

        // Write palette word with R=0x8, G=0xA, B=0xF (raw 4-bit values).
        let raw = (0x8 << 8) | (0xA << 4) | 0xF;
        bus.write(VCE_DATA_ADDR, (raw & 0xFF) as u8);
        bus.write(VCE_DATA_HIGH_ADDR, (raw >> 8) as u8);

        let rgb = bus.vce_palette_rgb(0);
        let r = ((rgb >> 16) & 0xFF) as u8;
        let g = ((rgb >> 8) & 0xFF) as u8;
        let b = (rgb & 0xFF) as u8;

        assert!(r > 0 && r < 255);
        assert!(g > r, "expected green component to dominate");
        assert!(b >= g, "blue should be largest for value 7");
    }

    #[cfg(test)]
    fn write_vram_word(bus: &mut Bus, addr: u16, value: u16) {
        bus.write_st_port(0, 0x00);
        bus.write_st_port(1, (addr & 0x00FF) as u8);
        bus.write_st_port(2, ((addr >> 8) & 0x7F) as u8);
        bus.write_st_port(0, 0x02);
        bus.write_st_port(1, (value & 0x00FF) as u8);
        bus.write_st_port(2, (value >> 8) as u8);
    }

    #[cfg(test)]
    fn fetch_frame(bus: &mut Bus, steps: u32) -> Vec<u32> {
        for _ in 0..(steps.saturating_mul(2)) {
            bus.tick(1, true);
            if let Some(frame) = bus.take_frame() {
                return frame;
            }
        }
        panic!("expected frame output");
    }

    #[test]
    fn render_blank_frame_uses_palette_zero() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Write a vivid palette entry at index 0.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0); // brightness max
        let raw_colour = 0x01FF; // full intensity (R=7,G=7,B=7)
        bus.write(VCE_DATA_ADDR, (raw_colour & 0x00FF) as u8);
        bus.write(VCE_DATA_HIGH_ADDR, (raw_colour >> 8) as u8);

        // Enable VBlank IRQ so tick processing advances display timing.
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);

        // Run long enough to hit VBlank.
        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        for _ in 0..steps {
            bus.tick(1, true);
        }

        let frame = bus.take_frame().expect("expected frame after VBlank");
        assert_eq!(frame.len(), FRAME_WIDTH * FRAME_HEIGHT);
        assert!(frame.iter().all(|&pixel| pixel == frame[0]));
        assert!(frame[0] != 0);
    }

    #[test]
    fn render_frame_uses_vram_palette_indices() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Configure brightness to max.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        // Palette index 0 -> background colour.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        // Palette index 0x10 -> black, 0x11 -> bright red.
        bus.write(VCE_CONTROL_ADDR, 0x10);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x11);
        bus.write(VCE_DATA_ADDR, 0x38); // red max
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        // Write tile map entry at VRAM 0 pointing to tile index 0x0100 with palette bank 1.
        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(0, 0) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        // Write a simple tile at tile index 0x0100: first pixel uses colour 1, others 0.
        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        // Enable background and configure scroll.
        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x10);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x07); // X scroll
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x08); // Y scroll
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x88);
        bus.write_st_port(2, 0x80);
        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        for _ in 0..steps {
            bus.tick(1, true);
        }

        let frame = bus.take_frame().expect("expected frame");
        assert_eq!(frame.len(), FRAME_WIDTH * FRAME_HEIGHT);
        let colour1 = bus.vce_palette_rgb(0x11);
        let colour0 = bus.vce_palette_rgb(0x00);
        assert_eq!(frame[0], colour1);
        assert_eq!(frame[1], colour0);
    }

    #[test]
    fn render_frame_respects_map_size_and_scroll() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Configure brightness and palette entries.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x10);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x11);
        bus.write(VCE_DATA_ADDR, 0x38);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        // Configure virtual map to 64x32 and scroll so tile column 40 appears at x=0.
        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x10);
        bus.write_st_port(2, 0x00);
        let scroll_x = 40 * TILE_WIDTH as u16;
        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, (scroll_x & 0xFF) as u8);
        bus.write_st_port(2, ((scroll_x >> 8) & 0x03) as u8);
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        // Write map entry for column 40 with palette bank 1.
        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(0, 40) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        // Tile pattern data for tile 0x0100.
        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x88);
        bus.write_st_port(2, 0x80);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        for _ in 0..steps {
            bus.tick(1, true);
        }

        let frame = bus.take_frame().expect("expected frame");
        assert_eq!(frame.len(), FRAME_WIDTH * FRAME_HEIGHT);
        let colour1 = bus.vce_palette_rgb(0x11);
        let colour0 = bus.vce_palette_rgb(0x00);
        assert_eq!(frame[0], colour1);
        assert_eq!(frame[1], colour0);
    }

    #[test]
    fn render_frame_honours_map_base_offset() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x11);
        bus.write(VCE_DATA_ADDR, 0x38);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x50);
        bus.write_st_port(2, 0x0A);

        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(0, 0) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        set_vdc_control(&mut bus, VDC_CTRL_DISPLAY_FULL);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        let frame = fetch_frame(&mut bus, steps);
        let colour = bus.vce_palette_rgb(0x11);
        assert_eq!(frame[0], colour);
    }

    #[test]
    fn render_frame_respects_cg_mode_restricted_planes() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        // Configure palettes: index 0 = background, 0x14 = visible colour.
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x14);
        bus.write(VCE_DATA_ADDR, 0x38); // bright red
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        // Map tile 0x0100 at origin using palette bank 1.
        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(0, 0) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        // Tile data: only plane2 bit set so colour index = 4.
        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0000);
        write_vram_word(&mut bus, tile_addr + 8, 0x0080);
        for offset in 1..16 {
            if offset == 8 {
                continue;
            }
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        // Scroll to origin and enable background.
        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        // Use restricted CG mode with CM=0 (only CG0 valid).
        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x03);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x88);
        bus.write_st_port(2, 0x80);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        let frame = fetch_frame(&mut bus, steps);
        let bg_colour = bus.vce_palette_rgb(0x00);
        assert_eq!(
            frame[0], bg_colour,
            "plane2 data should be ignored when CM=0"
        );

        // Switch to CM=1 and rerun a frame; plane2 data should now be visible.
        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x83);
        bus.write_st_port(2, 0x00);

        let frame_cm1 = fetch_frame(&mut bus, steps);
        let colour_plane2 = bus.vce_palette_rgb(0x14);
        assert_eq!(
            frame_cm1[0], colour_plane2,
            "plane2 data should produce colour when CM=1"
        );
    }

    #[test]
    fn render_frame_wraps_horizontally_on_64x64_map() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x11);
        bus.write(VCE_DATA_ADDR, 0x38);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x50);
        bus.write_st_port(2, 0x00);

        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(0, 63) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        let scroll_x = 63 * TILE_WIDTH as u16;
        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, (scroll_x & 0xFF) as u8);
        bus.write_st_port(2, ((scroll_x >> 8) & 0x03) as u8);
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x88);
        bus.write_st_port(2, 0x80);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        let frame = fetch_frame(&mut bus, steps);
        let expected = bus.vce_palette_rgb(0x11);
        assert_eq!(
            frame[0], expected,
            "scrolled column 63 should appear at x=0"
        );
        assert_eq!(
            frame[TILE_WIDTH],
            bus.vce_palette_rgb(0x00),
            "next column should wrap to column 0 background"
        );
    }

    #[test]
    fn render_frame_wraps_vertically_on_64x64_map() {
        let mut bus = Bus::new();
        bus.set_mpr(0, 0xFF);

        bus.write(VCE_CONTROL_ADDR, 0x00);
        bus.write(VCE_CONTROL_HIGH_ADDR, 0xF0);
        bus.write(VCE_DATA_ADDR, 0x00);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);
        bus.write(VCE_CONTROL_ADDR, 0x11);
        bus.write(VCE_DATA_ADDR, 0x38);
        bus.write(VCE_DATA_HIGH_ADDR, 0x00);

        bus.write_st_port(0, 0x09);
        bus.write_st_port(1, 0x50);
        bus.write_st_port(2, 0x00);

        let tile_index: u16 = 0x0100;
        let map_entry = tile_index | 0x1000;
        let map_addr = bus.vdc.map_entry_address_for_test(63, 0) as u16;
        write_vram_word(&mut bus, map_addr, map_entry);

        let tile_addr = tile_index * 16;
        write_vram_word(&mut bus, tile_addr, 0x0080);
        for offset in 1..16 {
            let addr = tile_addr.wrapping_add(offset as u16);
            write_vram_word(&mut bus, addr, 0x0000);
        }

        bus.write_st_port(0, 0x07);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        let scroll_y = 63 * TILE_HEIGHT as u16;
        bus.write_st_port(0, 0x08);
        bus.write_st_port(1, (scroll_y & 0xFF) as u8);
        bus.write_st_port(2, ((scroll_y >> 8) & 0x01) as u8);

        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x88);
        bus.write_st_port(2, 0x80);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let steps = line_cycles * (VDC_VISIBLE_LINES as u32 + 1);
        let frame = fetch_frame(&mut bus, steps);
        assert_eq!(
            frame[0],
            bus.vce_palette_rgb(0x11),
            "scrolled row 63 should appear at y=0"
        );
        assert_eq!(
            frame[FRAME_WIDTH * TILE_HEIGHT],
            bus.vce_palette_rgb(0x00),
            "next row should wrap to row 0 background"
        );
    }

    #[test]
    fn vdc_vblank_flag_clears_during_display() {
        let mut bus = Bus::new();
        bus.read_io(0x00); // clear initial flags.

        let mut seen_high = false;
        let mut saw_cleared_after = false;
        for _ in 0..(LINES_PER_FRAME as usize * 4) {
            bus.tick(500, true);
            let status = bus.read_io(0x00);
            if status & VDC_STATUS_VBL != 0 {
                seen_high = true;
            } else if seen_high {
                saw_cleared_after = true;
                break;
            }
        }
        assert!(seen_high, "VBlank status bit never asserted");
        assert!(
            saw_cleared_after,
            "VBlank status bit never cleared after asserting"
        );
    }

    #[test]
    fn vdc_vblank_flag_returns_after_display() {
        let mut bus = Bus::new();
        bus.read_io(0x00); // clear initial flags.

        let mut phase = 0;
        let mut seen_second_high = false;
        for _ in 0..(LINES_PER_FRAME as usize * 4) {
            bus.tick(500, true);
            let status = bus.read_io(0x00);
            match phase {
                0 => {
                    if status & VDC_STATUS_VBL != 0 {
                        phase = 1;
                    }
                }
                1 => {
                    if status & VDC_STATUS_VBL == 0 {
                        phase = 2;
                    }
                }
                _ => {
                    if status & VDC_STATUS_VBL != 0 {
                        seen_second_high = true;
                        break;
                    }
                }
            }
        }
        assert!(
            seen_second_high,
            "VBlank status bit never asserted again after clearing"
        );
    }

    #[test]
    fn vdc_register_write_sequence() {
        let mut bus = Bus::new();
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x04);
        assert_eq!(bus.vdc_register(0), Some(0x0400));

        bus.write_st_port(0, 0x02); // VRAM data
        bus.write_st_port(0, 0x02); // select VRAM data port
        bus.write_st_port(1, 0x34);
        bus.write_st_port(2, 0x12);
        assert_eq!(bus.vdc_vram_word(0x0400), 0x1234);
        assert_eq!(bus.vdc_register(0), Some(0x0401));

        // Subsequent data write should auto-increment MAWR
        bus.write_st_port(1, 0x78);
        bus.write_st_port(2, 0x56);
        assert_eq!(bus.vdc_vram_word(0x0401), 0x5678);
        assert_eq!(bus.vdc_register(0), Some(0x0402));
    }

    #[test]
    fn vdc_status_initial_vblank_and_clear() {
        let mut bus = Bus::new();
        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_VBL != 0);
        let status_after = bus.read_io(0x00);
        assert_eq!(status_after & VDC_STATUS_VBL, 0);
    }

    #[test]
    fn vdc_vblank_irq_raises_when_enabled() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);
        // Clear the initial VBlank state.
        bus.read_io(0x00);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);

        // Enable VBlank IRQ (bit 3 of control register).
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);

        for _ in 0..400 {
            bus.tick(200, false);
        }

        assert!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1 != 0);
        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_VBL != 0);
        bus.acknowledge_irq(IRQ_REQUEST_IRQ1);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_status_interrupt_respects_control() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);

        // Enable VBlank IRQ (bit 3 of control register).
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);

        bus.vdc_set_status_for_test(VDC_STATUS_VBL);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_IRQ1, IRQ_REQUEST_IRQ1);

        let status = bus.read(0x2000);
        assert_eq!(status & VDC_STATUS_VBL, VDC_STATUS_VBL);
        bus.write(0xFF13, IRQ_REQUEST_IRQ1);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_IRQ1, 0);

        // Disable VBlank interrupt and ensure no IRQ is raised.
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        bus.vdc_set_status_for_test(VDC_STATUS_VBL);
        bus.write(0xFF13, IRQ_REQUEST_IRQ1);
        bus.vdc_set_status_for_test(VDC_STATUS_VBL);
        assert_eq!(bus.read(0xFF13) & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_vram_increment_uses_control_bits() {
        let mut bus = Bus::new();

        bus.write_st_port(0, 0x00); // MAWR = 0
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        // Set increment mode to 32 (INC field = 01b at bits 12..11).
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x08);

        bus.write_st_port(0, 0x02); // VRAM data write
        bus.write_st_port(1, 0xAA);
        bus.write_st_port(2, 0x55);
        assert_eq!(bus.vdc_vram_word(0x0000), 0x55AA);
        assert_eq!(bus.vdc_register(0), Some(0x0020));

        bus.write_st_port(1, 0xBB);
        bus.write_st_port(2, 0x66);
        assert_eq!(bus.vdc_vram_word(0x0020), 0x66BB);
        assert_eq!(bus.vdc_register(0), Some(0x0040));
    }

    #[test]
    fn vdc_vram_reads_prefetch_and_increment() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);

        // Populate VRAM with two words.
        bus.write_st_port(0, 0x00); // MAWR = 0
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x02);
        bus.write_st_port(1, 0x34);
        bus.write_st_port(2, 0x12);
        bus.write_st_port(1, 0x78);
        bus.write_st_port(2, 0x56);

        assert_eq!(bus.vdc_vram_word(0x0000), 0x1234);
        assert_eq!(bus.vdc_vram_word(0x0001), 0x5678);
        assert_eq!(bus.vdc_register(0), Some(0x0002));

        // Point VRR to zero.
        bus.write_st_port(0, 0x01); // MARR
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        let lo = bus.read(0x2002);
        assert_eq!(lo, 0x34);
        assert_eq!(bus.vdc_register(1), Some(0x0000));

        let hi = bus.read(0x2003);
        assert_eq!(hi, 0x12);
        assert_eq!(bus.vdc_register(1), Some(0x0001));

        let next_lo = bus.read(0x2002);
        assert_eq!(next_lo, 0x78);
        let next_hi = bus.read(0x2003);
        assert_eq!(next_hi, 0x56);
        assert_eq!(bus.vdc_register(1), Some(0x0002));
    }

    #[test]
    fn vdc_satb_dma_copies_sprite_table_and_sets_interrupt() {
        let mut bus = Bus::new();
        // Clear initial VBlank flag.
        bus.read_io(0x00);

        // Seed VRAM at $0200 with sprite attribute data.
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x02);
        bus.write_st_port(0, 0x02); // VRAM data write
        for &word in &[0x1234u16, 0x5678, 0x9ABC, 0xDEF0] {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Enable SATB DMA IRQ and schedule a transfer from $0200.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, DMA_CTRL_IRQ_SATB as u8);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x14);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x02);

        // Run enough cycles to hit the next VBlank and service the DMA.
        for _ in 0..4 {
            bus.tick(200_000, true);
        }

        assert_eq!(bus.vdc_satb_word(0), 0x1234);
        assert_eq!(bus.vdc_satb_word(1), 0x5678);
        assert_eq!(bus.vdc_satb_word(2), 0x9ABC);
        assert_eq!(bus.vdc_satb_word(3), 0xDEF0);

        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );
        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_DS != 0);
        bus.acknowledge_irq(IRQ_REQUEST_IRQ1);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_satb_dma_sets_ds_when_source_written() {
        let mut bus = Bus::new();
        bus.read_io(0x00); // clear initial DS/VBlank bits

        const SATB_SOURCE: u16 = 0x0200;
        let sample = [0xAAAAu16, 0xBBBB, 0xCCCC, 0xDDDD];

        // Populate VRAM at $0200 with sample sprite attributes.
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, (SATB_SOURCE & 0x00FF) as u8);
        bus.write_st_port(2, (SATB_SOURCE >> 8) as u8);
        bus.write_st_port(0, 0x02); // VRAM data write
        for &word in &sample {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Writing the SATB source should latch and copy immediately.
        bus.write_st_port(0, 0x13);
        bus.write_st_port(1, (SATB_SOURCE & 0x00FF) as u8);
        bus.write_st_port(2, (SATB_SOURCE >> 8) as u8);

        for (idx, &expected) in sample.iter().enumerate() {
            assert_eq!(
                bus.vdc_satb_word(idx),
                expected,
                "SATB entry {idx} did not match VRAM word"
            );
        }
        assert_ne!(bus.vdc_status_bits() & VDC_STATUS_DS, 0);
    }

    #[test]
    fn vdc_cram_dma_transfers_palette_from_vram() {
        let mut bus = Bus::new();
        bus.read_io(0x00); // clear initial status bits

        const VRAM_BASE: u16 = 0x0500;
        let words = [0x0011u16, 0x2233, 0x4455, 0x6677];

        // Seed VRAM at $0500 with palette words.
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, (VRAM_BASE & 0x00FF) as u8);
        bus.write_st_port(2, (VRAM_BASE >> 8) as u8);
        bus.write_st_port(0, 0x02);
        for &word in &words {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Point the VRAM read address at the same base for CRAM DMA.
        bus.write_st_port(0, 0x01); // MARR
        bus.write_st_port(1, (VRAM_BASE & 0x00FF) as u8);
        bus.write_st_port(2, (VRAM_BASE >> 8) as u8);

        // Request four words for the upcoming CRAM DMA.
        bus.vdc.registers[0x12] = 0x0004;
        // Kick the CRAM DMA (bit 1 of DCR).
        bus.write_st_port(0, 0x0C);
        bus.write_st_port(1, DCR_ENABLE_CRAM_DMA);
        bus.write_st_port(2, 0x00);

        for _ in 0..4 {
            bus.tick(200_000, true);
        }

        for (idx, &expected) in words.iter().enumerate() {
            assert_eq!(
                bus.vce_palette_word(idx),
                expected,
                "palette entry {idx} did not match VRAM word"
            );
        }
        assert_eq!(bus.vdc_register(0x00), Some(VRAM_BASE + words.len() as u16));
        assert_eq!(
            bus.read_io(VCE_CONTROL_ADDR as usize) & 0xFF,
            words.len() as u8
        );
        assert_ne!(bus.vdc_status_bits() & VDC_STATUS_DV, 0);
    }

    #[test]
    fn vdc_vram_dma_copies_words_and_raises_status() {
        let mut bus = Bus::new();
        // Ensure source addresses point to RAM, not the hardware page.
        bus.set_mpr(0, 0xF8);
        bus.read_io(0x00); // clear initial VBlank

        const SOURCE: u16 = 0x0200;
        let words = [0x0AA0u16, 0x0BB1, 0x0CC2];
        for (index, &word) in words.iter().enumerate() {
            let base = SOURCE.wrapping_add((index as u16) * 2);
            bus.write(base, (word & 0x00FF) as u8);
            bus.write(base.wrapping_add(1), (word >> 8) as u8);
        }

        // Configure VRAM DMA: enable IRQ, set source/destination, and trigger length=3.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, DMA_CTRL_IRQ_VRAM as u8);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x10);
        bus.write_st_port(1, (SOURCE & 0x00FF) as u8);
        bus.write_st_port(2, (SOURCE >> 8) as u8);

        bus.write_st_port(0, 0x11);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x05);

        bus.write_st_port(0, 0x12);
        bus.write_st_port(1, words.len() as u8);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x0C);
        bus.write_st_port(1, DCR_ENABLE_VRAM_DMA);
        bus.write_st_port(2, 0x00);

        assert_eq!(bus.vdc_vram_word(0x0500), 0x0AA0);
        assert_eq!(bus.vdc_vram_word(0x0501), 0x0BB1);
        assert_eq!(bus.vdc_vram_word(0x0502), 0x0CC2);
        assert_eq!(
            bus.vdc_register(0x10),
            Some(SOURCE.wrapping_add((words.len() as u16) * 2))
        );
        assert_eq!(bus.vdc_register(0x11), Some(0x0503));

        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );
        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_DV != 0);
        bus.acknowledge_irq(IRQ_REQUEST_IRQ1);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_dma_status_clears_after_status_read() {
        let mut bus = Bus::new();
        bus.read_io(0x00); // clear initial VBlank

        // Configure VRAM DMA with IRQ enabled and execute a single-word copy.
        const SOURCE: u16 = 0x0100;
        bus.write(SOURCE, 0xAD);
        bus.write(SOURCE.wrapping_add(1), 0xDE);

        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, DMA_CTRL_IRQ_VRAM as u8);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x10);
        bus.write_st_port(1, (SOURCE & 0x00FF) as u8);
        bus.write_st_port(2, (SOURCE >> 8) as u8);
        bus.write_st_port(0, 0x11);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x02);
        bus.write_st_port(0, 0x12);
        bus.write_st_port(1, 0x01);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x0C);
        bus.write_st_port(1, DCR_ENABLE_VRAM_DMA);
        bus.write_st_port(2, 0x00);

        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );
        let status = bus.read_io(0x00);
        assert!(status & VDC_STATUS_DV != 0);
        assert_eq!(bus.read_io(0x00) & VDC_STATUS_DV, 0);
    }

    #[test]
    fn vdc_dma_status_clears_on_control_write() {
        let mut bus = Bus::new();
        bus.read_io(0x00);

        const SOURCE: u16 = 0x0400;
        bus.write(SOURCE, 0x34);
        bus.write(SOURCE.wrapping_add(1), 0x12);

        // Trigger VRAM DMA.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, DMA_CTRL_IRQ_VRAM as u8);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x10);
        bus.write_st_port(1, (SOURCE & 0x00FF) as u8);
        bus.write_st_port(2, (SOURCE >> 8) as u8);
        bus.write_st_port(0, 0x11);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);
        bus.write_st_port(0, 0x12);
        bus.write_st_port(1, 0x01);
        bus.write_st_port(2, 0x00);

        bus.write_st_port(0, 0x0C);
        bus.write_st_port(1, DCR_ENABLE_VRAM_DMA);
        bus.write_st_port(2, 0x00);

        assert_eq!(
            bus.pending_interrupts() & IRQ_REQUEST_IRQ1,
            IRQ_REQUEST_IRQ1
        );

        // Writing control with zero should acknowledge the flag and drop the IRQ.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x00);

        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
        assert_eq!(bus.read_io(0x00) & VDC_STATUS_DV, 0);
    }

    #[test]
    fn vdc_satb_auto_transfer_stops_when_disabled() {
        let mut bus = Bus::new();
        bus.read_io(0x00);

        // Seed VRAM at $0300 with initial sprite words.
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);
        bus.write_st_port(0, 0x02);
        let first_words = [0xAAAAu16, 0xBBBB];
        for &word in &first_words {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Enable SATB DMA with auto-transfer and IRQs.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, (DMA_CTRL_IRQ_SATB | DMA_CTRL_SATB_AUTO) as u8);
        bus.write_st_port(2, 0x00);

        // Point SATB DMA at $0300.
        bus.write_st_port(0, 0x14);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);

        // Run until VBlank triggers the auto SATB DMA.
        for _ in 0..4 {
            bus.tick(200_000, true);
        }

        assert_eq!(bus.vdc_satb_word(0), 0xAAAA);
        assert_eq!(bus.vdc_satb_word(1), 0xBBBB);

        // Acknowledge the interrupt while keeping auto-transfer enabled.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, (DMA_CTRL_IRQ_SATB | DMA_CTRL_SATB_AUTO) as u8);
        bus.write_st_port(2, 0x00);

        // Change VRAM words to a new pattern.
        bus.write_st_port(0, 0x00);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);
        bus.write_st_port(0, 0x02);
        let second_words = [0xCCCCu16, 0xDDDD];
        for &word in &second_words {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Disable auto-transfer (also acknowledges any pending flag).
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, DMA_CTRL_IRQ_SATB as u8);
        bus.write_st_port(2, 0x00);

        // Next frame should not pull new SATB data.
        for _ in 0..4 {
            bus.tick(200_000, true);
        }

        assert_eq!(bus.vdc_satb_word(0), 0xAAAA);
        assert_eq!(bus.vdc_satb_word(1), 0xBBBB);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_satb_auto_transfer_repeats_when_enabled() {
        let mut bus = Bus::new();
        bus.read_io(0x00);

        // Seed VRAM at $0300 with an initial pattern.
        bus.write_st_port(0, 0x00); // MAWR
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);
        bus.write_st_port(0, 0x02);
        let initial_words = [0x1111u16, 0x2222];
        for &word in &initial_words {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Enable SATB auto-transfer with IRQs.
        bus.write_st_port(0, 0x0F);
        bus.write_st_port(1, (DMA_CTRL_IRQ_SATB | DMA_CTRL_SATB_AUTO) as u8);
        bus.write_st_port(2, 0x00);

        // Point SATB DMA at $0300. Copy occurs immediately.
        bus.write_st_port(0, 0x14);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);

        assert_eq!(bus.vdc_satb_word(0), 0x1111);
        assert_eq!(bus.vdc_satb_word(1), 0x2222);
        assert_ne!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);

        // Acknowledge the initial interrupt and clear DS.
        bus.acknowledge_irq(IRQ_REQUEST_IRQ1);
        bus.read_io(0x00);

        // Overwrite VRAM with a new pattern; auto-transfer should pick it up on next VBlank.
        bus.write_st_port(0, 0x00);
        bus.write_st_port(1, 0x00);
        bus.write_st_port(2, 0x03);
        bus.write_st_port(0, 0x02);
        let updated_words = [0x3333u16, 0x4444];
        for &word in &updated_words {
            bus.write_st_port(1, (word & 0x00FF) as u8);
            bus.write_st_port(2, (word >> 8) as u8);
        }

        // Advance enough cycles to cover another frame; auto-transfer should fire.
        for _ in 0..4 {
            bus.tick(200_000, true);
        }

        assert_eq!(bus.vdc_satb_word(0), 0x3333);
        assert_eq!(bus.vdc_satb_word(1), 0x4444);
        assert_ne!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_rcr_irq_sets_irq1() {
        let mut bus = Bus::new();
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x04);
        bus.write_st_port(2, 0x00);
        bus.write_st_port(0, 0x06);
        bus.write_st_port(1, 0x05);
        bus.write_st_port(2, 0x00);

        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
        for _ in 0..100_000 {
            if bus.pending_interrupts() & IRQ_REQUEST_IRQ1 != 0 {
                break;
            }
            bus.tick(1, true);
        }
        assert_ne!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
        bus.acknowledge_irq(IRQ_REQUEST_IRQ1);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_vblank_irq_fires_via_tick() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);

        // Enable VBlank IRQ.
        bus.write_st_port(0, 0x05);
        bus.write_st_port(1, 0x08);
        bus.write_st_port(2, 0x00);

        // Clear any pending VBlank from power-on state.
        bus.read_io(0x00);
        bus.write(0xFF13, IRQ_REQUEST_IRQ1);
        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let visible_lines = VDC_VISIBLE_LINES as u32;
        let min_expected = line_cycles * visible_lines.saturating_sub(1);
        let max_expected = line_cycles * visible_lines.saturating_add(1);

        let mut trigger_iter = None;
        for iter in 0..(VDC_VBLANK_INTERVAL * 2) {
            if bus.tick(1, true) {
                trigger_iter = Some(iter);
                break;
            }
        }
        let trigger_iter =
            trigger_iter.expect("VBlank IRQ did not trigger within two frame intervals");
        assert!(
            trigger_iter >= min_expected && trigger_iter <= max_expected,
            "VBlank IRQ fired outside expected window: iter={trigger_iter}, min={min_expected}, max={max_expected}"
        );
        assert_ne!(bus.read(0xFF13) & IRQ_REQUEST_IRQ1, 0);
        let status = bus.read(0x2000);
        assert!(status & VDC_STATUS_VBL != 0);
        bus.write(0xFF13, IRQ_REQUEST_IRQ1);

        // Low-speed mode should need 4x cycles (fresh bus to reset accumulator).
        let mut slow_bus = Bus::new();
        slow_bus.set_mpr(1, 0xFF);
        slow_bus.write_st_port(0, 0x05);
        slow_bus.write_st_port(1, 0x08);
        slow_bus.write_st_port(2, 0x00);
        slow_bus.read_io(0x00);
        slow_bus.write(0xFF13, IRQ_REQUEST_IRQ1);
        let mut trigger_iter_slow = None;
        for iter in 0..(max_expected * 2) {
            if slow_bus.tick(1, false) {
                trigger_iter_slow = Some(iter);
                break;
            }
        }
        let trigger_iter_slow =
            trigger_iter_slow.expect("VBlank IRQ (slow clock) did not trigger within window");
        let slow_phi = trigger_iter_slow * 4;
        assert!(
            slow_phi >= min_expected && slow_phi <= max_expected,
            "Slow-clock VBlank IRQ fired outside expected window: cycles={} min={} max={}",
            slow_phi,
            min_expected,
            max_expected
        );
        assert_ne!(slow_bus.read(0xFF13) & IRQ_REQUEST_IRQ1, 0);
    }

    #[test]
    fn vdc_rcr_flag_clears_on_status_read() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);
        bus.write_st_port(0, 0x06);
        bus.write_st_port(1, 0x02);
        bus.write_st_port(2, 0x00);

        let line_cycles =
            (VDC_VBLANK_INTERVAL + LINES_PER_FRAME as u32 - 1) / LINES_PER_FRAME as u32;
        let target_line = 0x0002usize;
        for _ in 0..=target_line {
            bus.tick(line_cycles, true);
        }

        let status = bus.read(0x2000);
        assert!(status & VDC_STATUS_RCR != 0);
        let status_after = bus.read(0x2000);
        assert_eq!(status_after & VDC_STATUS_RCR, 0);
    }

    #[test]
    fn vdc_busy_flag_counts_down() {
        let mut bus = Bus::new();
        bus.set_mpr(1, 0xFF);
        bus.write_st_port(0, 0x02);
        bus.write_st_port(1, 0xAA);
        bus.write_st_port(2, 0x55);

        let status = bus.read(0x2000);
        assert!(status & VDC_STATUS_BUSY != 0);

        bus.tick(VDC_BUSY_ACCESS_CYCLES * 2, true);
        let cleared = bus.read(0x2000);
        assert_eq!(cleared & VDC_STATUS_BUSY, 0);
    }

    #[test]
    fn psg_irq2_triggers_when_enabled() {
        let mut bus = Bus::new();
        bus.write(0xFF60, PSG_REG_TIMER_LO as u8);
        bus.write(0xFF61, 0x20);
        bus.write(0xFF60, PSG_REG_TIMER_HI as u8);
        bus.write(0xFF61, 0x00);
        bus.write(0xFF60, PSG_REG_TIMER_CTRL as u8);
        bus.write(0xFF61, PSG_CTRL_ENABLE | PSG_CTRL_IRQ_ENABLE);

        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ2, 0);
        for _ in 0..0x20 {
            bus.tick(1, true);
        }
        assert_ne!(bus.pending_interrupts() & IRQ_REQUEST_IRQ2, 0);
        bus.acknowledge_irq(IRQ_REQUEST_IRQ2);
        assert_eq!(bus.pending_interrupts() & IRQ_REQUEST_IRQ2, 0);
    }

    #[test]
    fn psg_sample_uses_waveform_ram() {
        let mut bus = Bus::new();

        bus.write(0xFF60, 0x00);
        bus.write(0xFF61, 0x10);
        bus.write(0xFF61, 0x01);
        bus.write(0xFF61, 0x00);
        bus.write(0xFF61, 0x1F);

        bus.write(0xFF60, PSG_REG_COUNT as u8);
        bus.write(0xFF61, 0x1F);

        bus.write(0xFF60, PSG_REG_TIMER_LO as u8);
        bus.write(0xFF61, 0x20);
        bus.write(0xFF60, PSG_REG_TIMER_HI as u8);
        bus.write(0xFF61, 0x00);
        bus.write(0xFF60, PSG_REG_TIMER_CTRL as u8);
        bus.write(0xFF61, PSG_CTRL_ENABLE);

        for _ in 0..(PHI_CYCLES_PER_SAMPLE * 4) {
            bus.tick(1, true);
        }
        let samples = bus.take_audio_samples();
        assert!(samples.iter().any(|s| *s > 0));
    }
}
