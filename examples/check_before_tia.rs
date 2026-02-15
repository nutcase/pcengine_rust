use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font and zero VRAM font area
    emu.bus.vdc_clear_bios_font_store();
    for addr in 0x1000u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Track writes to BROADER font area
    emu.bus.vdc_set_write_range(0x1000, 0x1800);

    let mut frames = 0;
    let mut prev_write_count = 0u64;

    while frames < 140 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let wc = emu.bus.vdc_write_range_count();
            let new = wc - prev_write_count;

            // Check at frame 5 (after init), 10, 20, 135, 136, 137
            if new > 0 || frames == 5 || frames == 10 || frames == 135 || frames == 137 {
                // Check specific font tiles
                let h_base = 0x148u16 * 16;
                let h_w0 = emu.bus.vdc_vram_word(h_base);
                let h_w8 = emu.bus.vdc_vram_word(h_base + 8);
                let zero_base = 0x130u16 * 16;
                let z_w0 = emu.bus.vdc_vram_word(zero_base);

                println!(
                    "F{:3}: +{:5} writes (total {:6}) H=({:04X},{:04X}) 0=({:04X})",
                    frames, new, wc, h_w0, h_w8, z_w0
                );
            }
            prev_write_count = wc;
        }
    }

    // Show font tiles at frame 135 (before TIA) vs 137 (after TIA)
    println!("\n=== Checking key tiles at frame 140 ===");
    for &(tile_id, ch) in &[
        (0x120u16, '?'),
        (0x130, '0'),
        (0x141, 'A'),
        (0x148, 'H'),
        (0x150, 'P'),
        (0x155, 'U'),
        (0x15C, '!'),
        (0x140, '@'),
    ] {
        let base = tile_id as usize * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
            // Show plane 0 (lo byte words 0-7) and plane 2 (lo byte words 8-15)
            print!("p01=");
            for row in 0..8 {
                let w = emu.bus.vdc_vram_word(base as u16 + row as u16);
                let p0 = w & 0xFF;
                for bit in (0..8).rev() {
                    print!("{}", if (p0 >> bit) & 1 == 1 { "#" } else { "." });
                }
                print!("|");
            }
            print!(" p23=");
            for row in 0..8 {
                let w = emu.bus.vdc_vram_word(base as u16 + 8 + row as u16);
                let p2 = w & 0xFF;
                for bit in (0..8).rev() {
                    print!("{}", if (p2 >> bit) & 1 == 1 { "#" } else { "." });
                }
                print!("|");
            }
            println!();
        }
    }

    // Now check what's at tiles 0x100-0x11F (before font range)
    println!("\n=== Tiles 0x100-0x10F (pre-font area) ===");
    for tile_id in 0x100u16..0x110 {
        let base = tile_id as usize * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        if !all_zero {
            let w0 = emu.bus.vdc_vram_word(base as u16);
            let w8 = emu.bus.vdc_vram_word(base as u16 + 8);
            print!("  Tile 0x{:03X}: w0={:04X} w8={:04X} p2=", tile_id, w0, w8);
            for row in 0..8 {
                let w = emu.bus.vdc_vram_word(base as u16 + 8 + row as u16);
                let p2 = w & 0xFF;
                for bit in (0..8).rev() {
                    print!("{}", if (p2 >> bit) & 1 == 1 { "#" } else { "." });
                }
                print!("|");
            }
            println!();
        }
    }

    Ok(())
}
