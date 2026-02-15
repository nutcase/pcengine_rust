use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;

    // === Test 1: With BIOS font hack disabled ===
    println!("=== Test 1: BIOS font hack DISABLED ===");
    {
        let mut emu = Emulator::new();
        emu.load_hucard(&rom)?;
        emu.reset();

        // Clear BIOS font store so restore won't fire
        emu.bus.vdc_clear_bios_font_store();

        let mut frames = 0;
        while frames < 150 {
            emu.tick();
            if let Some(_f) = emu.take_frame() {
                frames += 1;
            }
        }

        println!("After 150 frames (no BIOS font hack):");
        dump_font_tiles(&emu, &rom);
    }

    // === Test 2: With BIOS font hack enabled (default) ===
    println!("\n=== Test 2: BIOS font hack ENABLED (default) ===");
    {
        let mut emu = Emulator::new();
        emu.load_hucard(&rom)?;
        emu.reset();
        // Don't clear BIOS font store - let the hack work

        let mut frames = 0;
        while frames < 150 {
            emu.tick();
            if let Some(_f) = emu.take_frame() {
                frames += 1;
            }
        }

        println!("After 150 frames (with BIOS font hack):");
        dump_font_tiles(&emu, &rom);
    }

    // === Test 3: Check expected data from ROM ===
    println!("\n=== Expected font data from ROM 0x2E400 ===");
    println!("(bank 0x17, offset $0400, as VDC word pairs)");
    for tile_idx in 0..3 {
        let rom_offset = 0x2E400 + tile_idx * 16;
        println!(
            "\nTile #{} (VRAM tile 0x{:03X}):",
            tile_idx,
            0x130 + tile_idx
        );
        println!("  ROM bytes:");
        print!("    ");
        for i in 0..16 {
            print!("{:02X} ", rom[rom_offset + i]);
        }
        println!();

        // How the routine writes these:
        // Read pairs [low, high] → STA $0002 (low), STA $0003 (high)
        // VDC sees: write_data_low(low), then write_data_high_direct(high)
        // Combined word = [low, high] = low | (high << 8)
        println!("  As VDC words (planes 0-1):");
        for word_idx in 0..8 {
            let lo = rom[rom_offset + word_idx * 2] as u16;
            let hi = rom[rom_offset + word_idx * 2 + 1] as u16;
            let word = lo | (hi << 8);
            print!("    [{word_idx}] = 0x{word:04X} (p0={lo:02X} p1={hi:02X})");
            if word_idx == 3 {
                println!();
            }
        }
        println!();

        // Render as 2bpp character
        println!("  Rendered (2bpp):");
        for row in 0..8 {
            let p0 = rom[rom_offset + row * 2];
            let p1 = rom[rom_offset + row * 2 + 1];
            print!("    ");
            for bit in (0..8).rev() {
                let v = ((p0 >> bit) & 1) | (((p1 >> bit) & 1) << 1);
                match v {
                    0 => print!("."),
                    1 => print!("o"),
                    2 => print!("*"),
                    3 => print!("#"),
                    _ => print!("?"),
                }
            }
            println!();
        }
    }

    Ok(())
}

fn dump_font_tiles(emu: &Emulator, rom: &[u8]) {
    // Check tiles 0x130-0x135 (first 6 font characters)
    for tile_idx in 0..6usize {
        let tile_id = 0x130 + tile_idx;
        let vram_base = tile_id * 16;
        print!("  Tile 0x{:03X} (VRAM 0x{:04X}): ", tile_id, vram_base);

        let mut all_zero = true;
        let mut words = Vec::new();
        for w in 0..16 {
            let word = emu.bus.vdc_vram_word((vram_base + w) as u16);
            words.push(word);
            if word != 0 {
                all_zero = false;
            }
        }

        if all_zero {
            println!("ALL ZERO");
        } else {
            // Show first 8 words (planes 0-1)
            for w in 0..8 {
                print!("{:04X} ", words[w]);
            }
            println!();

            // Check if it matches ROM data
            let rom_offset = 0x2E400 + tile_idx * 16;
            if rom_offset + 16 <= rom.len() {
                let mut matches_rom = true;
                for w in 0..8 {
                    let lo = rom[rom_offset + w * 2] as u16;
                    let hi = rom[rom_offset + w * 2 + 1] as u16;
                    let expected = lo | (hi << 8);
                    if words[w] != expected {
                        matches_rom = false;
                    }
                }
                if matches_rom {
                    println!("           → MATCHES ROM font data!");
                } else {
                    println!("           → Does NOT match ROM font data");
                    // Show expected
                    print!("           Expected: ");
                    for w in 0..8 {
                        let lo = rom[rom_offset + w * 2] as u16;
                        let hi = rom[rom_offset + w * 2 + 1] as u16;
                        let expected = lo | (hi << 8);
                        print!("{:04X} ", expected);
                    }
                    println!();
                }
            }

            // Render plane 0 as bitmap
            println!("           Plane 0:");
            for row in 0..8 {
                let p0 = (words[row] & 0xFF) as u8;
                print!("             ");
                for bit in (0..8).rev() {
                    if p0 & (1 << bit) != 0 {
                        print!("#");
                    } else {
                        print!(".");
                    }
                }
                println!();
            }
        }
    }

    // Also check tile 0x150 ('P') = tile index 32 from start
    println!("\n  Tile 0x150 (VRAM 0x1500, should be 'P' or similar):");
    let vram_base = 0x150 * 16;
    let mut all_zero = true;
    for w in 0..8 {
        let word = emu.bus.vdc_vram_word((vram_base + w) as u16);
        if word != 0 {
            all_zero = false;
        }
        print!("  {:04X}", word);
    }
    println!();
    if all_zero {
        println!("  ALL ZERO");
    }
}
