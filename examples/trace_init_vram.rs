use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font AND zero out ALL of the VRAM that our BIOS font wrote to
    emu.bus.vdc_clear_bios_font_store();
    // Zero VRAM from 0x1200 to 0x1800 (where we loaded font tiles 0x120-0x17F)
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Track writes to the FULL font range
    emu.bus.vdc_set_write_range(0x1300, 0x15D0);

    // Run ONE tick at a time and check for writes more granularly
    let mut total_ticks = 0u64;
    let mut frames = 0;
    let mut prev_write_count = 0u64;
    let mut first_write_tick = None;

    while frames < 150 {
        emu.tick();
        total_ticks += 1;

        let write_count = emu.bus.vdc_write_range_count();
        if write_count > prev_write_count && first_write_tick.is_none() {
            first_write_tick = Some(total_ticks);
            println!("First write at tick {} (frame ~{})", total_ticks, frames);
        }

        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;
            if new_writes > 0 || frames <= 10 {
                // Show H tile status
                let h_base = 0x148u16 * 16; // tile 'H'
                let h_w0 = emu.bus.vdc_vram_word(h_base);
                println!(
                    "F{:3}: +{:5} writes (total {:6}) H_w0=0x{:04X}",
                    frames, new_writes, write_count, h_w0
                );
            }
            prev_write_count = write_count;
        }
    }

    // Now check VRAM state at frame 150
    println!("\n=== Font tiles at frame 150 (pure game data) ===");
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
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
            // Show all 4 planes for first 2 rows
            let w0 = emu.bus.vdc_vram_word(base as u16);
            let w1 = emu.bus.vdc_vram_word(base as u16 + 1);
            let w8 = emu.bus.vdc_vram_word(base as u16 + 8);
            let w9 = emu.bus.vdc_vram_word(base as u16 + 9);
            print!("w0={:04X} w1={:04X} w8={:04X} w9={:04X} | ", w0, w1, w8, w9);

            // Decode plane 0 only for visual
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

    // Search ROM for 4bpp planar font: look for the pattern where lo==hi in
    // word pairs, and words 0-7 == words 8-15, forming recognizable characters
    println!("\n=== Searching ROM for 4bpp planar font data ===");
    // For a 4bpp font where all planes are identical (white on black),
    // each tile would be 32 bytes where bytes come in pairs (lo=hi)
    // and the first 16 bytes equal the second 16 bytes.
    // Search for this pattern matching 'H' shape
    let rom_data = &rom;
    let mut found = 0;
    for i in (0..rom_data.len().saturating_sub(32)).step_by(2) {
        // Check if this looks like a 4bpp 'H' tile
        // 32 bytes = 16 words, words stored as little-endian
        let mut words = [0u16; 16];
        for w in 0..16 {
            words[w] = u16::from_le_bytes([rom_data[i + w * 2], rom_data[i + w * 2 + 1]]);
        }
        // Check: all words have lo == hi (all planes same)
        let all_same_planes = words.iter().all(|&w| (w & 0xFF) == ((w >> 8) & 0xFF));
        if !all_same_planes {
            continue;
        }
        // Check: words 0-7 == words 8-15
        let planes_match = (0..8).all(|r| words[r] == words[r + 8]);
        if !planes_match {
            continue;
        }
        // Extract 1bpp pattern
        let pat: Vec<u8> = (0..8).map(|r| (words[r] & 0xFF) as u8).collect();
        // Check if it looks like 'H': symmetric vertical bars + horizontal bar
        if pat[7] != 0 {
            continue;
        }
        if pat[0] == 0 {
            continue;
        }
        if pat[0] != pat[1] || pat[0] != pat[2] {
            continue;
        }
        if pat[0] != pat[4] || pat[0] != pat[5] || pat[0] != pat[6] {
            continue;
        }
        if pat[3] <= pat[0] {
            continue;
        }
        if pat[0].count_ones() < 2 {
            continue;
        }

        println!("  Possible 4bpp 'H' tile at ROM offset 0x{:05X}", i);
        print!("    Pattern: ");
        for row in 0..8 {
            for bit in (0..8).rev() {
                print!("{}", if (pat[row] >> bit) & 1 == 1 { "#" } else { "." });
            }
            if row < 7 {
                print!("|");
            }
        }
        println!();
        found += 1;
        if found >= 5 {
            break;
        }
    }
    if found == 0 {
        println!("  No 4bpp H tiles found");
    }

    Ok(())
}
