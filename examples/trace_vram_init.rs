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

    // Track writes to ALL of VRAM (full range)
    emu.bus.vdc_set_write_range(0x0000, 0x7FFF);

    let mut frames = 0;
    let mut prev_write_count = 0u64;

    while frames < 150 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;
            if new_writes > 0 || frames <= 3 || frames == 150 {
                println!(
                    "F{:3}: +{:6} writes (total {:8})",
                    frames, new_writes, write_count
                );
            }
            prev_write_count = write_count;
        }
    }

    Ok(())
}
