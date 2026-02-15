use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    // Clear BIOS font so restore does nothing
    emu.bus.vdc_clear_bios_font_store();
    // DON'T zero font area - let's see what the game puts there naturally

    // Track writes to font tile area from the very start
    emu.bus.vdc_set_write_range(0x1300, 0x15D0); // tiles 0x130-0x15C exactly

    let mut frames = 0;
    let mut prev_write_count = 0u64;

    // First, check what's in VRAM right now (after reset, before first tick)
    println!("=== Before first tick ===");
    for &(tile_id, ch) in &[(0x130u16, '0'), (0x148, 'H'), (0x150, 'P'), (0x15C, '!')] {
        let base = tile_id as usize * 16;
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base as u16 + i as u16) == 0);
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
            let w0 = emu.bus.vdc_vram_word(base as u16);
            println!("[HAS DATA] w0=0x{:04X}", w0);
        }
    }

    // Run and track
    while frames < 200 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;
            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;
            if new_writes > 0 || frames <= 5 || frames == 136 || frames == 200 {
                println!(
                    "F{:3}: +{:5} writes (total {:6})",
                    frames, new_writes, write_count
                );
            }
            prev_write_count = write_count;
        }
    }

    // Show final state
    println!("\n=== Font tiles at frame 200 (no BIOS font, no zero-fill) ===");
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x141, 'A'),
        (0x148, 'H'),
        (0x14F, 'O'),
        (0x150, 'P'),
        (0x155, 'U'),
        (0x15C, '!'),
        (0x140, ' '),
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
                let p1 = (w >> 8) & 0xFF;
                for bit in (0..8).rev() {
                    let b0 = (p0 >> bit) & 1;
                    let b1 = (p1 >> bit) & 1;
                    let v = b0 | (b1 << 1);
                    print!("{}", if v > 0 { "#" } else { "." });
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
