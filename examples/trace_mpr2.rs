use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Print initial MPR state
    println!("After reset:");
    for i in 0..8 {
        println!("  MPR{}: bank 0x{:02X}", i, emu.bus.mpr(i));
    }

    // Run 20 ticks to get past reset
    for _ in 0..100 {
        emu.tick();
    }
    println!("\nAfter 100 ticks:");
    for i in 0..8 {
        println!("  MPR{}: bank 0x{:02X}", i, emu.bus.mpr(i));
    }

    // DON'T clear font store - let the natural behavior happen
    // But disable font restore so we see what the game does
    emu.bus.vdc_clear_bios_font_store();

    // Track all VRAM writes to font area
    emu.bus.vdc_set_write_range(0x1200, 0x1800);
    emu.bus.vdc_enable_write_log(10000);

    let mut frames = 0;
    let mut last_write_count = 0u64;
    let mut last_mpr2 = emu.bus.mpr(2);

    while frames < 200 {
        emu.tick();

        // Check if MPR2 changed
        let mpr2 = emu.bus.mpr(2);
        if mpr2 != last_mpr2 {
            println!(
                "MPR2 changed: 0x{:02X} -> 0x{:02X} (frame ~{})",
                last_mpr2, mpr2, frames
            );
            last_mpr2 = mpr2;
        }

        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let wc = emu.bus.vdc_write_range_count();
            if wc > last_write_count {
                println!(
                    "Frame {:3}: {} new VRAM writes to font area (MPR2=0x{:02X})",
                    frames,
                    wc - last_write_count,
                    emu.bus.mpr(2)
                );
                last_write_count = wc;
            }
        }
    }

    // Show the write log
    let log = emu.bus.vdc_take_write_log();
    let font_writes: Vec<_> = log
        .iter()
        .filter(|&&(addr, _)| addr >= 0x1200 && addr < 0x1800)
        .collect();
    println!(
        "\nFont area writes: {} (from {} total VRAM writes)",
        font_writes.len(),
        log.len()
    );

    // Check VDC register write counts
    println!(
        "\nVDC register 0x00 (MAWR) write count: {}",
        emu.bus.vdc_register_write_count(0x00)
    );
    println!(
        "VDC register 0x02 (VWR) write count: {}",
        emu.bus.vdc_register_write_count(0x02)
    );

    // Check specific VRAM content
    println!("\nVRAM tile 0x130 ('0') words:");
    for i in 0..16 {
        let addr = 0x1300 + i;
        print!("  [{:2}] = 0x{:04X}", i, emu.bus.vdc_vram_word(addr));
        if i == 7 {
            println!();
        }
    }
    println!();

    Ok(())
}
