use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font store so we can see what the game does naturally
    emu.bus.vdc_clear_bios_font_store();
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Enable write logging to font area
    emu.bus.vdc_enable_write_log(5000);

    let mut frames = 0;
    while frames < 150 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
        }
    }

    let log = emu.bus.vdc_take_write_log();
    let font_writes: Vec<_> = log
        .iter()
        .filter(|&&(addr, _)| addr >= 0x1200 && addr < 0x1800)
        .collect();
    println!(
        "VRAM writes to font area (0x1200-0x17FF): {}",
        font_writes.len()
    );
    for (i, &&(addr, val)) in font_writes.iter().enumerate().take(50) {
        let tile_id = addr / 16;
        println!(
            "  #{}: VRAM[0x{:04X}]=0x{:04X} (tile 0x{:03X})",
            i, addr, val, tile_id
        );
    }

    Ok(())
}
