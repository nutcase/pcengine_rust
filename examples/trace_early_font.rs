use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom = std::fs::read("roms/Kato-chan & Ken-chan (Japan).pce")?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;

    emu.reset();
    // Zero out font tile area to see if game writes its own font
    for addr in 0x1200u16..0x1800 {
        emu.bus.vdc_write_vram_direct(addr, 0);
    }

    // Track VRAM writes to font tile area (tiles 0x130-0x15C = VRAM 0x1300-0x15CF)
    emu.bus.vdc_set_write_range(0x1300, 0x15D0);

    let mut frames = 0;
    let mut prev_write_count = 0u64;

    while frames < 200 {
        emu.tick();
        if let Some(_f) = emu.take_frame() {
            frames += 1;

            let write_count = emu.bus.vdc_write_range_count();
            let new_writes = write_count - prev_write_count;

            if new_writes > 0 || frames <= 5 || (frames >= 130 && frames <= 145) {
                // Check tile 'H' (0x148) first word
                let h_base = 0x148u16 * 16;
                let h_w0 = emu.bus.vdc_vram_word(h_base);

                // Check tile '0' (0x130) first word
                let zero_base = 0x130u16 * 16;
                let zero_w0 = emu.bus.vdc_vram_word(zero_base);

                println!(
                    "F{:3}: +{:5} writes (total {:6}) H={:04X} 0={:04X}",
                    frames, new_writes, write_count, h_w0, zero_w0
                );
            }

            prev_write_count = write_count;
        }
    }

    // Show what's in font tiles at frame 200
    println!("\n=== Font tiles at frame 200 (no BIOS font) ===");
    for &(tile_id, ch) in &[
        (0x130u16, '0'),
        (0x148, 'H'),
        (0x150, 'P'),
        (0x142, 'B'),
        (0x15C, '!'),
    ] {
        let base = tile_id * 16;
        print!("  Tile 0x{:03X} '{}': ", tile_id, ch);
        let all_zero = (0..16).all(|i| emu.bus.vdc_vram_word(base + i) == 0);
        if all_zero {
            println!("[ALL ZERO]");
        } else {
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
            println!();
        }
    }

    Ok(())
}
