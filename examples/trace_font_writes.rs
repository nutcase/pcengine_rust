use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font and zero VRAM font area
    emu.bus.vdc_clear_bios_font_store();
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Enable detailed write logging for font area
    emu.bus.vdc_enable_write_log(100000);
    emu.bus.vdc_set_write_range(0x1200, 0x1800);

    let mut frames = 0;
    let mut last_count = 0u64;

    while frames < 150 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let count = emu.bus.vdc_write_range_count();
            if count > last_count {
                println!(
                    "Frame {}: {} new writes to font area",
                    frames,
                    count - last_count
                );
                last_count = count;
            }
        }
    }

    let log = emu.bus.vdc_take_write_log();
    println!("\nTotal write log entries: {}", log.len());

    // Filter for font area writes (0x1200-0x17FF)
    let font_writes: Vec<_> = log
        .iter()
        .filter(|&&(addr, _)| addr >= 0x1200 && addr < 0x1800)
        .collect();

    println!("Writes to font area (0x1200-0x17FF): {}", font_writes.len());
    for (i, &&(addr, val)) in font_writes.iter().enumerate().take(50) {
        let tile_id = addr / 16;
        let word_in_tile = addr % 16;
        println!(
            "  #{}: VRAM[0x{:04X}] = 0x{:04X} (tile 0x{:03X} word {})",
            i, addr, val, tile_id, word_in_tile
        );
    }

    // Check font tile contents
    println!("\n=== Font tile contents at frame 150 ===");
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x141, 'A'),
        (0x148, 'H'),
        (0x150, 'P'),
        (0x153, 'S'),
        (0x155, 'U'),
        (0x140, '@'),
        (0x15C, '!'),
    ] {
        let base = tile_id * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
        if all_zero {
            println!("  Tile 0x{:03X} '{}': [ALL ZERO]", tile_id, ch);
        } else {
            print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
            // Show combined pixel pattern (OR all planes)
            for row in 0..8 {
                let w01 = emu.bus.vdc_vram_word(base + row);
                let w23 = emu.bus.vdc_vram_word(base + 8 + row);
                let combined =
                    (w01 & 0xFF) | ((w01 >> 8) & 0xFF) | (w23 & 0xFF) | ((w23 >> 8) & 0xFF);
                for bit in (0..8).rev() {
                    print!("{}", if (combined >> bit) & 1 == 1 { "#" } else { "." });
                }
                if row < 7 {
                    print!("|");
                }
            }
            println!();
        }
    }

    Ok(())
}
