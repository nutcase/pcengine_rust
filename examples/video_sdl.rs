use pce::emulator::Emulator;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

const FRAME_WIDTH: usize = 256;
const FRAME_HEIGHT: usize = 240;
const SCALE: u32 = 3;
const DEFAULT_FRAME_LIMIT: Option<usize> = Some(120);
const MAX_CYCLES_PER_FRAME: u64 = 5_000_000;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let rom_path = args
        .next()
        .ok_or_else(|| "usage: video_sdl <rom.[bin|pce]>".to_string())?;
    let rom = std::fs::read(&rom_path)
        .map_err(|err| format!("failed to read ROM {}: {err}", rom_path))?;

    let mut emulator = Emulator::new();
    let is_pce = Path::new(&rom_path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);
    if is_pce {
        emulator
            .load_hucard(&rom)
            .map_err(|err| format!("failed to load HuCard: {err}"))?;
    } else {
        emulator.load_program(0xC000, &rom);
    }
    emulator.reset();

    let sdl = sdl2::init().map_err(|e| e.to_string())?;
    let video = sdl.video().map_err(|e| e.to_string())?;
    let window = video
        .window(
            "PC Engine (preview)",
            (FRAME_WIDTH as u32) * SCALE,
            (FRAME_HEIGHT as u32) * SCALE,
        )
        .position_centered()
        .opengl()
        .build()
        .map_err(|e| e.to_string())?;

    let mut canvas = window
        .into_canvas()
        .present_vsync()
        .build()
        .map_err(|e| e.to_string())?;
    canvas
        .set_scale(SCALE as f32, SCALE as f32)
        .map_err(|e| e.to_string())?;
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(
            PixelFormatEnum::ABGR8888,
            FRAME_WIDTH as u32,
            FRAME_HEIGHT as u32,
        )
        .map_err(|e| e.to_string())?;

    let mut event_pump = sdl.event_pump().map_err(|e| e.to_string())?;
    let mut quit = false;
    let mut pressed: HashSet<Keycode> = HashSet::new();
    let mut last_frame = Instant::now();

    while !quit {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => quit = true,
                Event::KeyDown {
                    keycode: Some(code),
                    repeat: false,
                    ..
                } => {
                    if code == Keycode::Escape {
                        quit = true;
                    } else {
                        pressed.insert(code);
                    }
                }
                Event::KeyUp {
                    keycode: Some(code),
                    repeat: false,
                    ..
                } => {
                    pressed.remove(&code);
                }
                _ => {}
            }
        }

        let pad_state = build_pad_state(&pressed);
        emulator.bus.set_joypad_input(pad_state);

        let mut cycles_budget = MAX_CYCLES_PER_FRAME;
        let mut frame_rendered = false;
        let mut frames_seen = 0usize;
        while cycles_budget > 0 && DEFAULT_FRAME_LIMIT.map_or(true, |limit| frames_seen < limit) {
            let cycles = emulator.tick() as u64;
            if cycles == 0 {
                cycles_budget = cycles_budget.saturating_sub(1);
            } else {
                cycles_budget = cycles_budget.saturating_sub(cycles);
            }
            if let Some(frame) = emulator.take_frame() {
                update_texture(&mut texture, &frame)?;
                canvas.clear();
                canvas.copy(
                    &texture,
                    None,
                    Some(Rect::new(0, 0, FRAME_WIDTH as u32, FRAME_HEIGHT as u32)),
                )?;
                canvas.present();
                frame_rendered = true;
                frames_seen += 1;
                break;
            }
        }

        if !frame_rendered {
            // throttle a little to avoid pegging the CPU when no frame is ready
            std::thread::sleep(Duration::from_millis(2));
        } else {
            // crude frame pacing based on vsync timing
            let elapsed = last_frame.elapsed();
            last_frame = Instant::now();
            if elapsed < Duration::from_millis(16) {
                std::thread::sleep(Duration::from_millis(16) - elapsed);
            }
        }
    }

    Ok(())
}

fn update_texture(texture: &mut sdl2::render::Texture, frame: &[u32]) -> Result<(), String> {
    texture.with_lock(None, |buffer, pitch| {
        for (y, row) in frame.chunks(FRAME_WIDTH).enumerate() {
            let dest = &mut buffer[y * pitch..y * pitch + FRAME_WIDTH * 4];
            for (pixel, chunk) in row.iter().zip(dest.chunks_mut(4)) {
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (*pixel & 0xFF) as u8;
                chunk[0] = b;
                chunk[1] = g;
                chunk[2] = r;
                chunk[3] = 0xFF;
            }
        }
    })
}

fn build_pad_state(pressed: &HashSet<Keycode>) -> u8 {
    let mut state: u8 = 0xFF;
    // Active low buttons.
    let mut clear = |bit: u8| state &= !(1 << bit);
    if pressed.contains(&Keycode::Right) {
        clear(0);
    }
    if pressed.contains(&Keycode::Left) {
        clear(1);
    }
    if pressed.contains(&Keycode::Down) {
        clear(2);
    }
    if pressed.contains(&Keycode::Up) {
        clear(3);
    }
    if pressed.contains(&Keycode::Z) {
        clear(4); // Button I
    }
    if pressed.contains(&Keycode::X) {
        clear(5); // Button II
    }
    if pressed.contains(&Keycode::Return) {
        clear(6); // Select
    }
    if pressed.contains(&Keycode::Space) {
        clear(7); // Run
    }
    state
}
