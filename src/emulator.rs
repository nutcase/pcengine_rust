use crate::bus::{Bus, IRQ_REQUEST_TIMER, PAGE_SIZE};
use crate::cpu::{Cpu, FLAG_INTERRUPT_DISABLE, FLAG_T};
use std::error::Error;

const HUCARD_HEADER_SIZE: usize = 512;
const HUCARD_MAGIC_LO: u8 = 0xAA;
const HUCARD_MAGIC_HI: u8 = 0xBB;
const HUCARD_TYPE_PCE: u8 = 0x02;

#[derive(Clone, Copy, Debug)]
struct HucardHeader {
    rom_pages: u16,
    flags: u8,
}

impl HucardHeader {
    fn parse(image: &[u8]) -> Option<Self> {
        if image.len() < HUCARD_HEADER_SIZE {
            return None;
        }
        let header = &image[..HUCARD_HEADER_SIZE];
        if header[8] != HUCARD_MAGIC_LO || header[9] != HUCARD_MAGIC_HI {
            return None;
        }
        if header[10] != HUCARD_TYPE_PCE {
            return None;
        }
        let rom_pages = u16::from_le_bytes([header[0], header[1]]);
        if rom_pages == 0 {
            return None;
        }
        let flags = header[2];
        Some(Self { rom_pages, flags })
    }

    fn backup_ram_bytes(&self) -> usize {
        match (self.flags >> 2) & 0x03 {
            0 => 0,
            1 => 16 * 1024,
            2 => 64 * 1024,
            _ => 256 * 1024,
        }
    }

    fn recommends_mode0(&self) -> bool {
        self.flags & 0x80 != 0
    }

    fn uses_reset_vector(&self) -> bool {
        self.flags & 0x02 != 0
    }

    fn recommended_layout(&self, pages: usize) -> Option<[usize; NUM_HUCARD_WINDOW_BANKS]> {
        if pages == 0 {
            return None;
        }
        let mut layout = [0; NUM_HUCARD_WINDOW_BANKS];
        if self.recommends_mode0() {
            for (slot, bank) in layout.iter_mut().enumerate() {
                *bank = slot % pages;
            }
        } else {
            let start = pages.saturating_sub(NUM_HUCARD_WINDOW_BANKS);
            for (slot, bank) in layout.iter_mut().enumerate() {
                *bank = (start + slot) % pages;
            }
        }
        Some(layout)
    }

    fn rom_size_bytes(&self) -> usize {
        self.rom_pages as usize * PAGE_SIZE
    }
}

struct ParsedHuCard {
    rom: Vec<u8>,
    header: Option<HucardHeader>,
}

impl ParsedHuCard {
    fn from_bytes(image: &[u8]) -> Result<Self, Box<dyn Error>> {
        if let Some(header) = HucardHeader::parse(image) {
            let mut rom = image[HUCARD_HEADER_SIZE..].to_vec();
            let expected = header.rom_size_bytes();
            if expected == 0 {
                return Err("HuCard header reports empty ROM".into());
            }
            if rom.len() < expected {
                rom.resize(expected, 0xFF);
            } else if rom.len() > expected {
                rom.truncate(expected);
            }
            if rom.is_empty() {
                return Err("HuCard payload is empty".into());
            }
            Ok(Self {
                rom,
                header: Some(header),
            })
        } else {
            if image.is_empty() {
                return Err("HuCard image is empty".into());
            }
            let mut rom = image.to_vec();
            let remainder = rom.len() % PAGE_SIZE;
            if remainder != 0 {
                rom.resize(rom.len() + (PAGE_SIZE - remainder), 0xFF);
            }
            if rom.is_empty() {
                return Err("HuCard payload is empty".into());
            }
            Ok(Self { rom, header: None })
        }
    }
}

pub struct Emulator {
    pub cpu: Cpu,
    pub bus: Bus,
    cycles: u64,
    audio_buffer: Vec<i16>,
    audio_batch_size: usize,
}

impl Emulator {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(),
            cycles: 0,
            audio_buffer: Vec::new(),
            audio_batch_size: 1024,
        }
    }

    /// Load a program into memory and wire the reset vector to it.
    pub fn load_program(&mut self, start: u16, data: &[u8]) {
        self.bus.load(start, data);
        self.bus.write(0xFFFC, (start & 0x00FF) as u8);
        self.bus.write(0xFFFD, (start >> 8) as u8);
    }

    /// Load a HuCard `.pce` image, handling optional 512-byte headers and
    /// mapping the upper MPR banks so the reset vector points into ROM.
    pub fn load_hucard(&mut self, image: &[u8]) -> Result<(), Box<dyn Error>> {
        let parsed = ParsedHuCard::from_bytes(image)?;
        let ParsedHuCard { rom, header } = parsed;
        self.bus = Bus::new();
        self.audio_buffer.clear();
        let backup_bytes = header
            .as_ref()
            .map(|descriptor| descriptor.backup_ram_bytes())
            .unwrap_or(0);
        debug_assert!(
            header.is_none() || backup_bytes == header.as_ref().unwrap().backup_ram_bytes()
        );
        self.bus.configure_cart_ram(backup_bytes);
        self.bus.load_rom_image(rom);

        let pages = self.bus.rom_page_count();
        if pages == 0 {
            return Err("HuCard contains no ROM banks".into());
        }

        let mut mapped = false;
        if let Some(ref descriptor) = header {
            if let Some(layout) = descriptor.recommended_layout(pages) {
                mapped = self.apply_header_layout(&layout, descriptor);
            }
        }

        if !mapped {
            self.map_boot_window(pages);
        }

        Ok(())
    }

    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
        self.seed_cpu_stack();
        self.cycles = 0;
    }

    pub fn tick(&mut self) -> u8 {
        let cycles = self.cpu.step(&mut self.bus);
        #[cfg(feature = "trace_hw_writes")]
        self.bus.set_last_pc_for_trace(self.cpu.pc);
        let mut bus_cycles = cycles as u32;
        if cycles == 0 && self.cpu.is_waiting() {
            bus_cycles = 1;
        }
        if cycles > 0 {
            self.cycles += cycles as u64;
        } else if self.cpu.is_waiting() {
            self.cycles += 1;
        }
        self.bus.tick(bus_cycles, self.cpu.clock_high_speed);
        let mut chunk = self.bus.take_audio_samples();
        if !chunk.is_empty() {
            self.audio_buffer.append(&mut chunk);
        }
        cycles
    }

    pub fn request_irq(&mut self) {
        self.bus.raise_irq(IRQ_REQUEST_TIMER);
    }

    pub fn request_nmi(&mut self) {
        self.cpu.request_nmi();
    }

    /// Run until BRK is encountered or until the optional cycle limit is hit.
    pub fn run_until_halt(&mut self, cycle_budget: Option<u64>) {
        while !self.cpu.halted {
            let cycles = self.tick() as u64;
            if let Some(budget) = cycle_budget {
                if self.cycles >= budget {
                    break;
                }
                if cycles == 0 && !self.cpu.is_waiting() {
                    break;
                }
            }
        }
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn take_audio_samples(&mut self) -> Option<Vec<i16>> {
        if self.audio_buffer.len() < self.audio_batch_size {
            return None;
        }
        Some(
            self.audio_buffer
                .drain(..self.audio_batch_size)
                .collect::<Vec<_>>(),
        )
    }

    pub fn take_frame(&mut self) -> Option<Vec<u32>> {
        self.bus.take_frame()
    }

    pub fn framebuffer(&self) -> &[u32] {
        self.bus.framebuffer()
    }

    pub fn backup_ram(&self) -> Option<&[u8]> {
        self.bus.cart_ram()
    }

    pub fn backup_ram_mut(&mut self) -> Option<&mut [u8]> {
        self.bus.cart_ram_mut()
    }

    pub fn load_backup_ram(&mut self, data: &[u8]) -> Result<(), Box<dyn Error>> {
        self.bus
            .load_cart_ram(data)
            .map_err(|err| Box::<dyn Error>::from(err.to_string()))?;
        Ok(())
    }

    pub fn save_backup_ram(&self) -> Option<Vec<u8>> {
        self.bus.cart_ram().map(|ram| ram.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::PAGE_SIZE;

    #[test]
    fn emulator_runs_simple_program() {
        let mut emu = Emulator::new();
        let program = [0xA9, 0x0F, 0x85, 0x10, 0x00];

        emu.load_program(0xC000, &program);
        emu.reset();
        emu.run_until_halt(Some(20));

        assert_eq!(emu.bus.read(0x0010), 0x0F);
        assert!(emu.cpu.halted);
    }

    #[test]
    fn load_hucard_maps_reset_vector() {
        let mut rom = vec![0u8; PAGE_SIZE * 4];
        let vec_offset = PAGE_SIZE - 4;
        rom[vec_offset] = 0x34;
        rom[vec_offset + 1] = 0xE2;
        let entry = 0xE234usize - 0xE000usize;
        rom[entry] = 0xA9; // LDA #$99
        rom[entry + 1] = 0x99;
        rom[entry + 2] = 0x00; // BRK
        let entry_ptr = PAGE_SIZE - 8;
        rom[entry_ptr] = 0x34;
        rom[entry_ptr + 1] = 0xE2;
        let mut emu = Emulator::new();
        emu.load_hucard(&rom).unwrap();
        emu.reset();

        assert_eq!(emu.cpu.pc, 0xE234);
    }

    #[test]
    fn load_hucard_falls_back_when_high_banks_empty() {
        let mut rom = vec![0u8; PAGE_SIZE * 16];
        let vec_offset = (15 * PAGE_SIZE) + (PAGE_SIZE - 4);
        rom[vec_offset] = 0x78;
        rom[vec_offset + 1] = 0xF6;
        let entry = (15 * PAGE_SIZE) + (0xF678 - 0xE000) as usize;
        rom[entry] = 0xA9; // LDA #$01
        rom[entry + 1] = 0x01;
        rom[entry + 2] = 0x00; // BRK
        let entry_ptr = (15 * PAGE_SIZE) + (PAGE_SIZE - 8);
        rom[entry_ptr] = 0x78;
        rom[entry_ptr + 1] = 0xF6;

        let mut emu = Emulator::new();
        emu.load_hucard(&rom).unwrap();
        emu.reset();

        assert_eq!(emu.cpu.pc, 0xF678);
    }

    #[test]
    fn load_hucard_with_magic_griffin_header_sets_cart_ram() {
        let rom_pages = 4u16;
        let mut image = vec![0u8; HUCARD_HEADER_SIZE + (rom_pages as usize * PAGE_SIZE)];
        image[0] = (rom_pages & 0x00FF) as u8;
        image[1] = (rom_pages >> 8) as u8;
        image[2] = 0x84; // Mode 0 entry, 16 KiB backup RAM.
        image[8] = HUCARD_MAGIC_LO;
        image[9] = HUCARD_MAGIC_HI;
        image[10] = HUCARD_TYPE_PCE;

        let header = HucardHeader::parse(&image).unwrap();
        assert_eq!(header.flags, 0x84);
        assert_eq!(header.backup_ram_bytes(), 16 * 1024);

        let payload = &mut image[HUCARD_HEADER_SIZE..];
        let reset_offset = payload.len() - 4;
        payload[reset_offset] = 0x00;
        payload[reset_offset + 1] = 0x80;
        payload[0] = 0x00; // BRK to halt once execution reaches entry point.

        let mut emu = Emulator::new();
        emu.load_hucard(&image).unwrap();
        assert_eq!(emu.bus.cart_ram_size(), header.backup_ram_bytes());
        emu.reset();

        assert_eq!(emu.bus.cart_ram_size(), 16 * 1024);
        assert_eq!(emu.cpu.pc, 0x8000);
        assert_eq!(emu.bus.read_u16(0xFFFC), 0x8000);
        assert_eq!(emu.bus.read(0x8000), 0x00);
    }

    #[test]
    fn backup_ram_round_trip_via_emulator_api() {
        let rom = vec![0xFF; PAGE_SIZE * 8];
        let mut emu = Emulator::new();
        emu.load_hucard(&rom).unwrap();
        assert!(emu.backup_ram().is_none());
        assert!(emu.load_backup_ram(&[]).is_err());

        // Configure backup RAM explicitly and exercise APIs.
        emu.bus.configure_cart_ram(PAGE_SIZE);
        let snapshot = vec![0xC3; PAGE_SIZE];
        emu.load_backup_ram(&snapshot).unwrap();
        assert_eq!(emu.save_backup_ram().unwrap()[0], 0xC3);
        assert_eq!(emu.bus.cart_ram().unwrap()[..16], snapshot[..16]);
    }

    #[test]
    fn wai_unblocks_when_timer_irq_fires() {
        let program = [
            0xA9, 0x04, // LDA #$04 (timer reload)
            0x8D, 0x10, 0xFF, // STA $FF10
            0xA9, 0x01, // LDA #$01 (start timer)
            0x8D, 0x11, 0xFF, // STA $FF11
            0x58, // CLI
            0xCB, // WAI
            0x00, // BRK
            // IRQ handler immediately after the main routine:
            0xAD, 0x00, 0x40, // LDA $4000
            0x69, 0x01, // ADC #$01
            0x8D, 0x00, 0x40, // STA $4000
            0x40, // RTI
        ];

        let mut emu = Emulator::new();
        emu.load_program(0x8000, &program);
        emu.bus.write_u16(0xFFFE, 0x800D);
        emu.reset();

        emu.run_until_halt(Some(2_000));

        assert!(emu.bus.read(0x4000) > 0);
    }
}

const NUM_HUCARD_WINDOW_BANKS: usize = 4;

impl Emulator {
    fn apply_header_layout(
        &mut self,
        layout: &[usize; NUM_HUCARD_WINDOW_BANKS],
        header: &HucardHeader,
    ) -> bool {
        for (slot, bank) in layout.iter().enumerate() {
            self.bus.map_bank_to_rom(4 + slot, *bank);
        }
        let vector = self.bus.read_u16(0xFFFC);
        if header.uses_reset_vector() {
            is_valid_reset_vector(vector)
        } else if header.recommends_mode0() {
            vector >= 0x8000 && vector != 0xFFFF
        } else {
            vector != 0 && vector != 0xFFFF
        }
    }

    fn map_boot_window(&mut self, pages: usize) {
        if pages == 0 {
            return;
        }

        let mut reset_bank = None;
        for bank in 0..pages {
            self.bus.map_bank_to_rom(7, bank);
            let vector = self.bus.read_u16(0xFFFC);
            if is_valid_reset_vector(vector) {
                reset_bank = Some(bank);
                break;
            }
        }

        let reset_bank = reset_bank.unwrap_or_else(|| pages.saturating_sub(1));
        // Map a contiguous 8‑bank window ending with the bank that contains the
        // reset vector. This mirrors the HuC6280 power‑on layout (banks 0–7 ->
        // pages 0–7) while still tolerating larger images whose reset vector may
        // live elsewhere.
        let base = (reset_bank + pages + 1 - NUM_HUCARD_WINDOW_BANKS) % pages;
        for slot in 0..NUM_HUCARD_WINDOW_BANKS {
            let rom_bank = (base + slot) % pages;
            let mpr_slot = 4 + slot; // banks 4–7
            self.bus.map_bank_to_rom(mpr_slot, rom_bank);
        }
    }
}

fn is_valid_reset_vector(vector: u16) -> bool {
    (0x8000..=0xFFFD).contains(&vector) && vector != 0xFFFF
}

impl Emulator {
    fn seed_cpu_stack(&mut self) {
        let reset_pc = self.bus.read_u16(0xFFFC);
        if self.bus.read(reset_pc) != 0x40 {
            return;
        }

        let mut entry = self.bus.read_u16(0xFFF8);
        if !is_valid_reset_vector(entry) || self.bus.read(entry) == 0x00 {
            entry = reset_pc.wrapping_add(1);
        }
        let (pcl, pch) = (entry as u8, (entry >> 8) as u8);

        // Emulate BIOS hand-off: preload stack so RTI reads status/PC.
        let status = FLAG_INTERRUPT_DISABLE | FLAG_T;

        // BIOS caches the current VDC status byte at $0000 before resuming cart code.
        let vdc_status = self.bus.read_io(0x00);
        self.bus.write(0x0000, vdc_status);
        self.bus.write(0x0028, vdc_status);

        // Arrange the stack so RTI restores the desired PC and status.
        // RTI pops: status, PCL, PCH. Mimic the hardware state just after the IRQ push.
        self.bus.write(0x01FA, status);
        self.bus.write(0x01FB, pcl);
        self.bus.write(0x01FC, pch);
        self.cpu.sp = 0xF9;
    }
}
