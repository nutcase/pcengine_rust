use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;

    emu.reset();

    // Clear the stored BIOS font so restore_bios_font_tiles() does nothing
    emu.bus.vdc_clear_bios_font_store();

    // Zero out font tile area to see what the GAME writes
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Track VRAM writes to font tile area
    emu.bus.vdc_set_write_range(0x1200, 0x1800);

    let mut frames = 0;
    let mut prev_write_count = 0u64;

    while frames < 200 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;

            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;

            if new_writes > 0 || frames <= 3 || frames == 136 || frames == 200 {
                println!(
                    "F{:3}: +{:5} writes (total {:6})",
                    frames, new_writes, write_count
                );
            }

            prev_write_count = write_count;
        }
    }

    // Dump what's actually in font tiles at frame 200 (without any restore)
    println!("\n=== Raw VRAM at font tiles (NO restore) ===");
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x131, '1'),
        (0x141, 'A'),
        (0x142, 'B'),
        (0x148, 'H'),
        (0x149, 'I'),
        (0x14F, 'O'),
        (0x150, 'P'),
        (0x152, 'R'),
        (0x153, 'S'),
        (0x154, 'T'),
        (0x155, 'U'),
        (0x14E, 'N'),
        (0x15C, '!'),
        (0x140, ' '),
        (0x13D, '©'),
    ] {
        let base = tile_id as usize * 16;
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
            // Show plane 0 (low byte of words 0-7) as bitmap
            for row in 0..8 {
                let w = emu.bus.vdc_vram_word(base as u16 + row as u16);
                let p0 = w & 0xFF;
                for bit in (0..8).rev() {
                    print!("{}", if (p0 >> bit) & 1 == 1 { "#" } else { "." });
                }
                if row < 7 {
                    print!("|");
                }
            }
            println!();
        }
    }

    // Also count how many font tiles are non-zero
    let mut nonzero_count = 0;
    for tile_id in 0x120u16..0x180 {
        let base = tile_id as usize * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        if !all_zero {
            nonzero_count += 1;
        }
    }
    println!("\nNon-zero tiles in 0x120-0x17F: {}/96", nonzero_count);

    Ok(())
}
