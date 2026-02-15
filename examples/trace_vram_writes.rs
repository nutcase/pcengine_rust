/// Dump ALL VRAM writes during the corruption frame (2142).
use pce::emulator::Emulator;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let rom_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "roms/Kato-chan & Ken-chan (Japan).pce".to_string());

    let rom = std::fs::read(&rom_path)?;
    let mut emu = Emulator::new();
    emu.load_hucard(&rom)?;
    emu.reset();

    let target_frame = 2142u64;
    let mut frame_count = 0u64;

    println!("Running to frame {}...", target_frame);

    // Enable write logging just before the target frame
    for _ in 0..target_frame + 2 {
        let joypad = if frame_count % 120 < 5 {
            0xFF ^ 0x08
        } else if frame_count % 60 < 3 {
            0xFF ^ 0x01
        } else {
            0xFF
        };
        emu.bus.set_joypad_input(joypad);

        if frame_count == target_frame - 1 {
            emu.bus.vdc_enable_write_log(100000);
        }

        loop {
            emu.tick();
            if emu.take_frame().is_some() {
                break;
            }
        }
        frame_count += 1;

        if frame_count == target_frame {
            let all_writes = emu.bus.vdc_take_write_log();
            println!("\n=== Frame {} VRAM writes: {} total ===", frame_count, all_writes.len());

            // Filter writes to addresses in the tile data range (0x1000-0x1600)
            // Show ALL writes in order to see the MAWR sequence
            println!("\n=== Full VRAM write sequence ({} writes) ===", all_writes.len());
            for (i, (addr, val)) in all_writes.iter().enumerate() {
                let tile_id = *addr as usize / 16;
                let word_in_tile = *addr as usize % 16;
                let marker = if *val == 0 && *addr >= 0x1000 && *addr % 16 == 0 { " *** ROW0=0 ***" } else { "" };
                let in_range = *addr >= 0x1200 && *addr < 0x1600;
                if in_range || marker.len() > 0 || i < 20 || all_writes.len() - i < 20 {
                    println!(
                        "  [{:3}] VRAM[{:04X}] = {:04X}  (tile 0x{:03X} +{:X}){}",
                        i, addr, val, tile_id, word_in_tile, marker
                    );
                }
            }

            // Re-enable for next frame
            emu.bus.vdc_enable_write_log(100000);
        }

        if frame_count == target_frame + 1 {
            let v1400 = emu.bus.vdc_vram_word(0x1400);
            println!("\nFrame {}: VRAM[1400]={:04X}", frame_count, v1400);
        }
    }

    Ok(())
}
