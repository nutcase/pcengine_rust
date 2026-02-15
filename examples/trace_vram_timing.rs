/// Trace VDC timing state during VRAM writes in the corruption frame.
/// Checks whether writes happen during VBlank or active display.
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

    // Hook into the frame before corruption
    for _ in 0..target_frame + 1 {
        let joypad = if frame_count % 120 < 5 {
            0xFF ^ 0x08
        } else if frame_count % 60 < 3 {
            0xFF ^ 0x01
        } else {
            0xFF
        };
        emu.bus.set_joypad_input(joypad);

        if frame_count == target_frame - 1 {
            // Enable write logging for the target frame
            emu.bus.vdc_enable_write_log(100000);
        }

        // Before running frame, snapshot VRAM[0x1400]
        let pre_vram_1400 = emu.bus.vdc_vram_word(0x1400);

        if frame_count == target_frame {
            // Run this frame tick-by-tick to see timing of each write
            println!("\n=== Frame {} tick-by-tick ===", frame_count);
            println!("Pre-frame VRAM[1400]={:04X}", pre_vram_1400);
            println!("Pre-frame in_vblank={}, scanline={}, busy_cycles={}",
                emu.bus.vdc_in_vblank(),
                emu.bus.vdc_current_scanline(),
                emu.bus.vdc_busy_cycles());

            let mut tick_count = 0u64;
            let mut last_vram_1400 = pre_vram_1400;
            let mut prev_log_len = 0usize;
            let mut prev_scanline = emu.bus.vdc_current_scanline();
            let mut prev_vblank = emu.bus.vdc_in_vblank();

            loop {
                let scanline_before = emu.bus.vdc_current_scanline();
                let vblank_before = emu.bus.vdc_in_vblank();
                let busy_before = emu.bus.vdc_busy_cycles();

                emu.tick();
                tick_count += 1;

                let scanline_after = emu.bus.vdc_current_scanline();
                let vblank_after = emu.bus.vdc_in_vblank();
                let cur_log_len = emu.bus.vdc_write_log_len();

                // Check if new writes happened this tick
                if cur_log_len > prev_log_len {
                    let new_writes = cur_log_len - prev_log_len;
                    let cur_vram_1400 = emu.bus.vdc_vram_word(0x1400);

                    if new_writes <= 4 || prev_log_len < 5 || cur_vram_1400 != last_vram_1400 {
                        println!(
                            "  tick {:6}: {} new write(s) (total {}), scanline {} -> {}, vblank {} -> {}, busy {} -> {}, VRAM[1400]={:04X}",
                            tick_count, new_writes, cur_log_len,
                            scanline_before, scanline_after,
                            vblank_before, vblank_after,
                            busy_before, emu.bus.vdc_busy_cycles(),
                            cur_vram_1400
                        );
                    }
                    last_vram_1400 = cur_vram_1400;
                    prev_log_len = cur_log_len;
                }

                // Track scanline changes
                if scanline_after != prev_scanline || vblank_after != prev_vblank {
                    if tick_count < 200 || vblank_after != prev_vblank {
                        println!(
                            "  tick {:6}: scanline {} -> {}, vblank {} -> {}",
                            tick_count, prev_scanline, scanline_after, prev_vblank, vblank_after
                        );
                    }
                    prev_scanline = scanline_after;
                    prev_vblank = vblank_after;
                }

                if emu.take_frame().is_some() {
                    println!("  Frame complete at tick {}", tick_count);
                    break;
                }
            }

            let post_vram_1400 = emu.bus.vdc_vram_word(0x1400);
            println!("Post-frame VRAM[1400]={:04X}", post_vram_1400);

            let all_writes = emu.bus.vdc_take_write_log();
            println!("Total VRAM writes in frame: {}", all_writes.len());

            // Show the first 10 writes with timing context
            println!("\nFirst 10 writes:");
            for (i, (addr, val)) in all_writes.iter().take(10).enumerate() {
                println!("  [{:3}] VRAM[{:04X}] = {:04X}", i, addr, val);
            }
        } else {
            // Normal frame
            loop {
                emu.tick();
                if emu.take_frame().is_some() {
                    break;
                }
            }
        }
        frame_count += 1;
    }

    Ok(())
}
