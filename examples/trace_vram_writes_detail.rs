use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font store
    emu.bus.vdc_clear_bios_font_store();

    // Enable detailed VRAM write log (large buffer)
    // Track ALL writes, not just a range
    emu.bus.vdc_enable_write_log(5000);

    // Run until MPR2 changes to 0x17 (font loading starts)
    let mut frames = 0;
    let mut last_mpr2 = emu.bus.mpr(2);
    let mut font_loading_started = false;

    while frames < 150 {
        emu.tick();

        let mpr2 = emu.bus.mpr(2);
        if mpr2 != last_mpr2 {
            println!(
                "Frame ~{}: MPR2 changed {:02X} -> {:02X}",
                frames, last_mpr2, mpr2
            );
            last_mpr2 = mpr2;
            if mpr2 == 0x17 && !font_loading_started {
                font_loading_started = true;
                // Reset write log to capture only font-related writes
                let _ = emu.bus.vdc_take_write_log();
                emu.bus.vdc_enable_write_log(3000);
            }
        }

        if let Some(_f) = emu.take_frame() {
            frames += 1;
        }
    }

    // Get the write log
    let log = emu.bus.vdc_take_write_log();
    println!("\nTotal VRAM writes logged: {}", log.len());

    // Show writes to tile area 0x1300-0x1700
    let font_writes: Vec<_> = log
        .iter()
        .filter(|&&(addr, _)| addr >= 0x1300 && addr < 0x1700)
        .collect();
    println!("Writes to font area (0x1300-0x16FF): {}", font_writes.len());

    // Show first 80 writes to font area with their MAWR addresses
    println!("\nFirst 80 font area writes:");
    for (i, &&(addr, val)) in font_writes.iter().enumerate().take(80) {
        let tile = addr / 16;
        let word_in_tile = addr % 16;
        print!(
            "  #{:3}: MAWR=0x{:04X} val=0x{:04X} (tile 0x{:03X} word {:2})",
            i, addr, val, tile, word_in_tile
        );

        // For planes 0-1 (words 0-7), check if value matches ROM data
        if word_in_tile < 8 {
            let rom_tile_idx = (tile as usize).wrapping_sub(0x130);
            let rom_offset = 0x2E400 + rom_tile_idx * 16 + (word_in_tile as usize) * 2;
            if rom_offset + 1 < rom.len() {
                let lo = rom[rom_offset] as u16;
                let hi = rom[rom_offset + 1] as u16;
                let expected = lo | (hi << 8);
                if val == expected {
                    print!(" [ROM match]");
                } else if val == 0 {
                    print!(" [zero]");
                }
            }
        } else if val == 0 {
            print!(" [zero plane 2-3]");
        }
        println!();
    }

    // Check the sequence of MAWR addresses for the first 40 writes
    println!("\nMAWR address sequence (first 40 font writes):");
    for chunk in font_writes.chunks(16).take(3) {
        let addrs: Vec<String> = chunk
            .iter()
            .map(|&&(addr, _)| format!("{:04X}", addr))
            .collect();
        println!("  {}", addrs.join(" "));
    }

    // Also look at what's around MAWR $1310 (where tile 0x131 should be written)
    println!("\nAll writes near tile 0x131 (MAWR 0x1310-0x131F):");
    for (i, &(addr, val)) in log.iter().enumerate() {
        if addr >= 0x1310 && addr < 0x1320 {
            println!("  Log #{}: MAWR=0x{:04X} val=0x{:04X}", i, addr, val);
        }
    }

    // Check increment step
    println!("\nVDC increment step analysis:");
    if font_writes.len() >= 2 {
        let a0 = font_writes[0].0;
        let a1 = font_writes[1].0;
        println!("  First write: MAWR=0x{:04X}", a0);
        println!("  Second write: MAWR=0x{:04X}", a1);
        println!("  Difference: {}", a1 as i32 - a0 as i32);
    }

    Ok(())
}
