use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let mut frames = 0;

    // Key tile IDs for text characters (0x100 + ASCII)
    let text_tiles: &[(u16, char)] = &[
        (0x148, 'H'), // 'H'
        (0x149, 'I'), // 'I'
        (0x153, 'S'), // 'S'
        (0x143, 'C'), // 'C'
        (0x14F, 'O'), // 'O'
        (0x152, 'R'), // 'R'
        (0x145, 'E'), // 'E'
        (0x150, 'P'), // 'P'
        (0x155, 'U'), // 'U'
    ];

    // Check BAT and tile data at key frames
    let check_frames = [
        1, 10, 50, 100, 110, 120, 125, 130, 135, 140, 145, 150, 200, 250, 300,
    ];
    let mut next_check = 0;

    // Map dimensions for BAT lookup
    let map_width = 64usize; // 64x64 map (MWR=0x0050)

    // Text row locations in the title screen (tile row, description)
    let text_rows: &[(usize, &str)] = &[
        (21, "HISCORE"),   // Y ~= 168-175
        (25, "PUSH RUN"),  // Y ~= 200-207
        (27, "COPYRIGHT"), // Y ~= 216-223
    ];

    while frames < 301 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;

            if next_check < check_frames.len() && frames == check_frames[next_check] {
                next_check += 1;

                println!("\n=== Frame {} ===", frames);

                // Check VDC registers
                let cr = emu.bus.vdc_register(0x05).unwrap_or(0);
                let mwr = emu.bus.vdc_register(0x09).unwrap_or(0);
                let mawr = emu.bus.vdc_register(0x00).unwrap_or(0);
                let bxr = emu.bus.vdc_register(0x07).unwrap_or(0);
                let byr = emu.bus.vdc_register(0x08).unwrap_or(0);
                let incr = match (cr >> 11) & 0x03 {
                    0 => 1,
                    1 => 32,
                    2 => 64,
                    _ => 128,
                };
                println!(
                    "  CR={:04X} BG={} SPR={} INCR={}",
                    cr,
                    (cr >> 7) & 1,
                    (cr >> 6) & 1,
                    incr
                );
                println!(
                    "  MWR={:04X} MAWR={:04X} BXR={:04X} BYR={:04X}",
                    mwr, mawr, bxr, byr
                );

                // Check BAT entries at text rows
                for &(tile_row, desc) in text_rows {
                    print!("  BAT row {} ({}): ", tile_row, desc);
                    let mut tiles = Vec::new();
                    for tile_col in 6..28 {
                        // columns 6-27 (text area)
                        let bat_addr = (tile_row * map_width + tile_col) as u16;
                        let bat_entry = emu.bus.vdc_vram_word(bat_addr);
                        let tile_id = bat_entry & 0x07FF;
                        let palette = (bat_entry >> 12) & 0x0F;
                        tiles.push((tile_id, palette));
                    }
                    // Show unique tile IDs
                    let mut unique: Vec<_> = tiles.iter().map(|&(t, p)| (t, p)).collect();
                    unique.sort();
                    unique.dedup();
                    if unique.len() <= 5 {
                        for &(t, p) in &unique {
                            print!("0x{:03X}/pal{} ", t, p);
                        }
                    } else {
                        let min_t = tiles.iter().map(|x| x.0).min().unwrap();
                        let max_t = tiles.iter().map(|x| x.0).max().unwrap();
                        print!(
                            "range 0x{:03X}-0x{:03X} ({} unique)",
                            min_t,
                            max_t,
                            unique.len()
                        );
                    }
                    println!();
                }

                // Check font tile patterns
                print!("  Font tiles: ");
                for &(tile_id, ch) in text_tiles {
                    let base = tile_id as u16 * 16;
                    let w0 = emu.bus.vdc_vram_word(base);
                    let w1 = emu.bus.vdc_vram_word(base + 1);
                    let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
                    let plane0_only = (0..8).all(|i| {
                        let word = emu.bus.vdc_vram_word(base + i);
                        (word >> 8) == 0 // plane 1 = 0
                    }) && (8..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
                    let status = if all_zero {
                        "ZERO"
                    } else if plane0_only {
                        "font"
                    } else {
                        "gfx"
                    };
                    print!("{}={} ", ch, status);
                }
                println!();

                // Show tile H detail at key frames
                if frames == 130 || frames == 140 || frames == 150 || frames == 300 {
                    let tile_h = 0x148u16;
                    let base = tile_h * 16;
                    println!("  Tile 'H' (0x148) raw VRAM:");
                    for row in 0..8 {
                        let w = emu.bus.vdc_vram_word(base + row);
                        let p0 = w & 0xFF;
                        let p1 = (w >> 8) & 0xFF;
                        print!("    row{}: p0={:02X} p1={:02X}  ", row, p0, p1);
                        for bit in (0..8).rev() {
                            let b0 = (p0 >> bit) & 1;
                            let b1 = (p1 >> bit) & 1;
                            let val = b0 | (b1 << 1);
                            print!(
                                "{}",
                                match val {
                                    0 => ".",
                                    1 => "#",
                                    2 => "o",
                                    _ => "X",
                                }
                            );
                        }
                        println!();
                    }
                    // Check planes 2-3
                    let w8 = emu.bus.vdc_vram_word(base + 8);
                    let w9 = emu.bus.vdc_vram_word(base + 9);
                    println!("    planes2-3: w8={:04X} w9={:04X}", w8, w9);
                }

                // Check VRAM DMA count
                let dma_count = emu.bus.vdc_vram_dma_count();
                if dma_count > 0 {
                    let dma_src = emu.bus.vdc_vram_last_source();
                    let dma_dst = emu.bus.vdc_vram_last_destination();
                    let dma_len = emu.bus.vdc_vram_last_length();
                    println!(
                        "  VRAM DMA: {} ops, last src={:04X} dst={:04X} len={:04X}",
                        dma_count, dma_src, dma_dst, dma_len
                    );
                }
            }
        }
    }

    Ok(())
}
