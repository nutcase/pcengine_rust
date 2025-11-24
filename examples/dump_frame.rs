use pce::emulator::Emulator;
use std::{env, error::Error, fs::File, io::Write, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let rom_path = args
        .next()
        .ok_or("usage: dump_frame <rom.[bin|pce]> [frames] [output.ppm]")?;
    let frame_target: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    let output_path = args.next().unwrap_or_else(|| "frame.ppm".to_string());

    let rom = std::fs::read(&rom_path)?;

    let mut emulator = Emulator::new();
    let is_pce = Path::new(&rom_path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);

    if is_pce {
        emulator.load_hucard(&rom)?;
    } else {
        emulator.load_program(0xC000, &rom);
    }
    emulator.reset();

    let mut frames_collected = 0;
    let mut safety_cycles = (frame_target as u64).saturating_mul(250_000).max(5_000_000);
    while frames_collected < frame_target && safety_cycles > 0 {
        let cycles = emulator.tick() as u64;
        if cycles == 0 {
            safety_cycles = safety_cycles.saturating_sub(1);
        } else {
            safety_cycles = safety_cycles.saturating_sub(cycles);
        }
        if let Some(frame) = emulator.take_frame() {
            frames_collected += 1;
            if frames_collected == frame_target {
                write_ppm(&frame, &output_path)?;
                println!("wrote frame {frame_target} to {output_path}");
                println!(
                    "VDC control register: {:#06X} map size: {:?}",
                    emulator.bus.vdc_register(0x05).unwrap_or(0),
                    emulator.bus.vdc_map_dimensions()
                );
                println!(
                    "VDC control writes: {} (last {:#06X})",
                    emulator.bus.vdc_control_write_count(),
                    emulator.bus.vdc_last_control()
                );
                println!("VDC status: {:#04X}", emulator.bus.vdc_status_bits());
                println!(
                    "SATB pending: {} source: {:#06X}",
                    emulator.bus.vdc_satb_pending(),
                    emulator.bus.vdc_satb_source()
                );
                println!(
                    "CRAM DMA last src: {:#06X} len: {:#06X}",
                    emulator.bus.vdc_cram_last_source(),
                    emulator.bus.vdc_cram_last_length()
                );
                println!(
                    "VRAM DMA count: {} last src: {:#06X} dst: {:#06X} len: {:#06X}",
                    emulator.bus.vdc_vram_dma_count(),
                    emulator.bus.vdc_vram_last_source(),
                    emulator.bus.vdc_vram_last_destination(),
                    emulator.bus.vdc_vram_last_length()
                );
                break;
            }
        }
    }

    if frames_collected < frame_target {
        eprintln!("warning: reached cycle budget without collecting frame {frame_target}");
    }

    Ok(())
}

fn write_ppm(frame: &[u32], path: &str) -> Result<(), Box<dyn Error>> {
    const WIDTH: usize = 256;
    const HEIGHT: usize = 240;
    if frame.len() != WIDTH * HEIGHT {
        return Err("unexpected frame dimensions".into());
    }

    let mut file = File::create(path)?;
    writeln!(file, "P6\n{} {}\n255", WIDTH, HEIGHT)?;
    for pixel in frame {
        let r = ((pixel >> 16) & 0xFF) as u8;
        let g = ((pixel >> 8) & 0xFF) as u8;
        let b = (pixel & 0xFF) as u8;
        file.write_all(&[r, g, b])?;
    }
    Ok(())
}
