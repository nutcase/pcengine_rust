use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear the stored BIOS font so restore does nothing
    emu.bus.vdc_clear_bios_font_store();

    // Zero out font tile area
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Track both regular writes AND DMA writes
    emu.bus.vdc_set_write_range(0x1200, 0x1800);

    let mut frames = 0;
    let mut prev_write_count = 0u64;
    let mut prev_dma_count = 0u64;

    while frames < 200 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;

            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;

            let dma_count = emu.bus.vdc_vram_dma_count();
            let new_dmas = dma_count - prev_dma_count;

            if new_writes > 0 || new_dmas > 0 || frames <= 5 || frames == 200 {
                println!(
                    "F{:3}: +{:5} writes (total {:6}), +{} DMA ops (total {})",
                    frames, new_writes, write_count, new_dmas, dma_count
                );
                if new_dmas > 0 {
                    println!(
                        "      DMA src=0x{:04X} dst=0x{:04X} len=0x{:04X}",
                        emu.bus.vdc_vram_last_source(),
                        emu.bus.vdc_vram_last_destination(),
                        emu.bus.vdc_vram_last_length(),
                    );
                }
            }

            prev_write_count = write_count;
            prev_dma_count = dma_count;
        }
    }

    // Check if any non-zero data in font tiles
    println!("\n=== Font tile check (no restore, no BIOS font) ===");
    let mut nonzero = 0;
    for tile_id in 0x120u16..0x180 {
        let base = tile_id as usize * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        if !all_zero {
            nonzero += 1;
        }
    }
    println!("Non-zero tiles: {}/96", nonzero);

    // Show specific font tiles
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x141, 'A'),
        (0x148, 'H'),
        (0x150, 'P'),
        (0x15C, '!'),
    ] {
        let base = tile_id as usize * 16;
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
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

    Ok(())
}
