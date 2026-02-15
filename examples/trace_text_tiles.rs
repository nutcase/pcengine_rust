use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Track specific font tile VRAM words
    // Tile 'H' (0x148) at VRAM 0x1480-0x148F
    // Tile 'P' (0x150) at VRAM 0x1500-0x150F
    let watched_tiles: &[(u16, char)] = &[
        (0x148, 'H'),
        (0x150, 'P'),
        (0x130, '0'),
        (0x140, '@'), // space
    ];

    // Set write range tracking to the text tile VRAM area
    emu.bus.vdc_set_write_range(0x1300, 0x15D0);

    let mut frames = 0;
    let mut prev_h_word = 0u16;
    let mut prev_write_count = 0u64;

    while frames < 200 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;

            // Check font tiles at every frame from 125 to 160
            if frames >= 125 && frames <= 160 {
                let write_count = emu.bus.vdc_write_range_count();
                let new_writes = write_count - prev_write_count;

                // Check tile H first word
                let h_base = 0x148u16 * 16;
                let h_word = emu.bus.vdc_vram_word(h_base);

                let mawr = emu.bus.vdc_register(0x00).unwrap_or(0);
                let cr = emu.bus.vdc_register(0x05).unwrap_or(0);
                let incr = match (cr >> 11) & 0x03 {
                    0 => 1,
                    1 => 32,
                    2 => 64,
                    _ => 128,
                };

                // Check status of all watched tiles
                let mut tile_status = String::new();
                for &(tile_id, ch) in watched_tiles {
                    let base = tile_id * 16;
                    let w0 = emu.bus.vdc_vram_word(base);
                    let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
                    let plane0_only = (0..8).all(|i| (emu.bus.vdc_vram_word(base + i) >> 8) == 0)
                        && (8..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
                    let status = if all_zero {
                        "Z"
                    } else if plane0_only {
                        "F"
                    } else {
                        "G"
                    };
                    tile_status.push_str(&format!("{}={}{:04X} ", ch, status, w0));
                }

                let changed = h_word != prev_h_word;
                println!(
                    "F{:3}: MAWR={:04X} incr={:3} writes_to_text=+{:4} (total {:5}) {} {}",
                    frames,
                    mawr,
                    incr,
                    new_writes,
                    write_count,
                    if changed { "*CHANGED*" } else { "" },
                    tile_status
                );

                prev_h_word = h_word;
                prev_write_count = write_count;
            }
        }
    }

    // At frame 200, dump the full range of VRAM to see what's there
    println!("\n=== VRAM dump at frame 200: tiles 0x130-0x15C ===");
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x131, '1'),
        (0x137, '7'),
        (0x138, '8'),
        (0x139, '9'),
        (0x13D, '©'),
        (0x140, ' '),
        (0x142, 'B'),
        (0x143, 'C'),
        (0x144, 'D'),
        (0x145, 'E'),
        (0x146, 'F'),
        (0x148, 'H'),
        (0x149, 'I'),
        (0x14E, 'N'),
        (0x14F, 'O'),
        (0x150, 'P'),
        (0x152, 'R'),
        (0x153, 'S'),
        (0x154, 'T'),
        (0x155, 'U'),
        (0x15C, '!'),
    ] {
        let base = tile_id * 16;
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        // Show plane 0 pattern
        for row in 0..8 {
            let w = emu.bus.vdc_vram_word(base + row);
            let p0 = w & 0xFF;
            for bit in (0..8).rev() {
                print!("{}", if (p0 >> bit) & 1 == 1 { "#" } else { "." });
            }
            if row < 7 {
                print!("|");
            }
        }

        // Also check all planes
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
        let p1_nonzero = (0..8).any(|i| (emu.bus.vdc_vram_word(base + i) >> 8) != 0);
        let p23_nonzero = (8..16).any(|i| emu.bus.vdc_vram_word(base + i) != 0);
        let tag = if all_zero {
            " [ZERO]"
        } else if !p1_nonzero && !p23_nonzero {
            " [p0 only]"
        } else {
            " [multi-plane]"
        };
        println!("{}", tag);
    }

    Ok(())
}
