/// Trace VDC Control Register (CR) and MAWR values around the corruption point.
/// This tells us if the increment value (CR bits 12:11) is correct.
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
            emu.bus.vdc_enable_write_log(100000);
        }

        if frame_count >= target_frame - 1 && frame_count <= target_frame {
            let mut tick_count = 0u64;
            let mut prev_log_len = 0usize;
            let mut prev_cr = emu.bus.vdc_control_register();
            let mut prev_mawr = emu.bus.vdc_mawr();

            println!("\n=== Frame {} start: CR={:04X} (incr={}) MAWR={:04X} VRAM[1400]={:04X} ===",
                frame_count,
                prev_cr,
                match (prev_cr >> 11) & 3 { 0 => 1, 1 => 32, 2 => 64, _ => 128 },
                prev_mawr,
                emu.bus.vdc_vram_word(0x1400));

            loop {
                let cr_before = emu.bus.vdc_control_register();
                let mawr_before = emu.bus.vdc_mawr();

                emu.tick();
                tick_count += 1;

                let cr_after = emu.bus.vdc_control_register();
                let mawr_after = emu.bus.vdc_mawr();
                let cur_log_len = emu.bus.vdc_write_log_len();

                // Report CR changes
                if cr_after != prev_cr {
                    let incr_before = match (prev_cr >> 11) & 3 { 0 => 1, 1 => 32, 2 => 64, _ => 128 };
                    let incr_after = match (cr_after >> 11) & 3 { 0 => 1, 1 => 32, 2 => 64, _ => 128 };
                    println!(
                        "  tick {:6}: CR changed {:04X} -> {:04X} (incr {} -> {})",
                        tick_count, prev_cr, cr_after, incr_before, incr_after
                    );
                    prev_cr = cr_after;
                }

                // Report VRAM writes with CR context
                if cur_log_len > prev_log_len {
                    let new_writes = cur_log_len - prev_log_len;
                    if new_writes > 100 || prev_log_len < 5 {
                        let incr = match (cr_before >> 11) & 3 { 0 => 1, 1 => 32, 2 => 64, _ => 128 };
                        println!(
                            "  tick {:6}: {} VRAM writes (total {}), CR={:04X} incr={}, MAWR before={:04X} after={:04X}",
                            tick_count, new_writes, cur_log_len,
                            cr_before, incr, mawr_before, mawr_after
                        );
                    }
                    prev_log_len = cur_log_len;
                }

                // Report large MAWR jumps (indicating register select change or increment change)
                if mawr_after != prev_mawr {
                    let diff = mawr_after.wrapping_sub(prev_mawr);
                    if diff != 1 && diff != 32 && diff != 64 && diff != 128 && tick_count < 100 {
                        println!(
                            "  tick {:6}: MAWR changed {:04X} -> {:04X} (delta={})",
                            tick_count, prev_mawr, mawr_after, diff as i16
                        );
                    }
                    prev_mawr = mawr_after;
                }

                if emu.take_frame().is_some() {
                    println!("  Frame complete at tick {}, VRAM[1400]={:04X}",
                        tick_count, emu.bus.vdc_vram_word(0x1400));

                    if frame_count == target_frame - 1 {
                        // Take the write log for analysis
                        let writes = emu.bus.vdc_take_write_log();
                        println!("  Write log: {} entries", writes.len());
                        // Re-enable for next frame
                        emu.bus.vdc_enable_write_log(100000);
                    }
                    break;
                }
            }
        } else {
            loop {
                emu.tick();
                if emu.take_frame().is_some() { break; }
            }
        }
        frame_count += 1;
    }

    println!("\nFinal VRAM[1400]={:04X}", emu.bus.vdc_vram_word(0x1400));
    Ok(())
}
