use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font
    emu.bus.vdc_clear_bios_font_store();
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Enable VRAM write logging for first 50 writes
    emu.bus.vdc_enable_write_log(50);

    let mut frames = 0;
    while frames < 10 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
        }
    }

    // Print the logged writes
    let log = emu.bus.vdc_take_write_log();
    println!("First {} VRAM writes:", log.len());
    for (i, (addr, value)) in log.iter().enumerate() {
        println!("  #{:3}: VRAM[0x{:04X}] = 0x{:04X}", i, addr, value);
    }

    // Also check what's written to VRAM in the 0x1200-0x1800 range at frame 10
    println!("\n=== VRAM font area at frame 10 ===");
    let mut nonzero = 0;
    for addr in (0x1200u16..0x1800).step_by(16) {
        let tile_id = addr / 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(addr + i) == 0);
        if !all_zero {
            nonzero += 1;
            if nonzero <= 5 {
                let w0 = emu.bus.vdc_vram_word(addr);
                let w1 = emu.bus.vdc_vram_word(addr + 1);
                println!(
                    "  Tile 0x{:03X} (VRAM 0x{:04X}): w0=0x{:04X} w1=0x{:04X}",
                    tile_id, addr, w0, w1
                );
            }
        }
    }
    println!("Non-zero tiles in 0x120-0x17F: {}", nonzero);

    Ok(())
}
