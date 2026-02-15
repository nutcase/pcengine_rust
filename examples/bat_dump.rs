use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let mut frames = 0;
    while frames < 150 {
        emu.tick();
        if let Some(_) = emu.take_frame() {
            frames += 1;
        }
    }

    let (map_w, map_h) = emu.bus.vdc_map_dimensions();
    let byr = emu.bus.vdc_register(0x08).unwrap_or(0) as usize;
    println!("Map: {}x{}, BYR={}", map_w, map_h, byr);

    // With BYR=51 and 224 active lines, visible sample_y range: 51 to 274
    // Title rows 0-34 (sample_y 0-279) cover the visible range
    println!("\nBAT rows 0-35 (flat addressing, first 32 cols):");
    for bat_row in 0..36 {
        let sample_y_start = bat_row * 8;
        let sample_y_end = sample_y_start + 7;
        let active_row_start = if sample_y_start >= byr {
            sample_y_start - byr
        } else {
            999
        };
        let visible = sample_y_start >= byr && sample_y_start < byr + 224;

        // Count non-zero and non-0x0200 tiles
        let mut nonzero = 0;
        let mut special = 0;
        let mut first_nonzero_tile = 0u16;
        let mut first_pal = 0u16;
        for col in 0..32 {
            let addr = bat_row * map_w + col;
            let entry = emu.bus.vdc_vram_word(addr as u16);
            if entry != 0 && entry != 0x0200 {
                if nonzero == 0 {
                    first_nonzero_tile = entry & 0x7FF;
                    first_pal = (entry >> 12) & 0xF;
                }
                nonzero += 1;
            }
            if entry == 0x0200 {
                special += 1;
            }
        }
        if nonzero > 0 || visible {
            print!(
                "  row {:2} (sy {:3}-{:3})",
                bat_row, sample_y_start, sample_y_end
            );
            if visible {
                print!(" [VIS ar{:3}]", active_row_start);
            } else {
                print!(" [      ]");
            }
            print!(" non200={:2} bg200={:2}", nonzero, special);
            if nonzero > 0 {
                print!(" first=tile{:03X}p{}", first_nonzero_tile, first_pal);
            }

            // Show actual tiles for key rows
            if nonzero > 0 && nonzero <= 20 {
                print!(" |");
                for col in 0..32 {
                    let addr = bat_row * map_w + col;
                    let entry = emu.bus.vdc_vram_word(addr as u16);
                    let tile = entry & 0x7FF;
                    let pal = (entry >> 12) & 0xF;
                    if entry != 0 && entry != 0x0200 {
                        print!(" {:03X}p{}", tile, pal);
                    }
                }
            }
            println!();
        }
    }

    // Also check what visible lines map to
    println!("\nActive line summary with BYR={}:", byr);
    println!("  active_row   0: sample_y={}, tile_row={}", byr, byr / 8);
    println!(
        "  active_row  28: sample_y={}, tile_row={}",
        byr + 28,
        (byr + 28) / 8
    );
    println!(
        "  active_row  29: sample_y={}, tile_row={}",
        byr + 29,
        (byr + 29) / 8
    );
    println!(
        "  active_row 100: sample_y={}, tile_row={}",
        byr + 100,
        (byr + 100) / 8
    );
    println!(
        "  active_row 223: sample_y={}, tile_row={}",
        byr + 223,
        (byr + 223) / 8
    );

    Ok(())
}
