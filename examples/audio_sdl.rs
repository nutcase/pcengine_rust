use pce::emulator::Emulator;
use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioStatus};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const AUDIO_BATCH: usize = 1024;

struct PcmStream {
    buffer: Arc<Mutex<VecDeque<i16>>>,
}

impl AudioCallback for PcmStream {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        let mut guard = self.buffer.lock().unwrap();
        for sample in out.iter_mut() {
            *sample = guard.pop_front().unwrap_or(0);
        }
    }
}

fn main() -> Result<(), String> {
    let rom_path = std::env::args().nth(1).ok_or_else(|| {
        "usage: cargo run --example audio_sdl --features audio-sdl -- <rom.[bin|pce]>".to_string()
    })?;
    let rom = std::fs::read(&rom_path).map_err(|e| format!("failed to read ROM: {e}"))?;
    let is_pce = Path::new(&rom_path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);

    let sdl = sdl2::init()?;
    let audio = sdl.audio()?;
    let shared = Arc::new(Mutex::new(VecDeque::with_capacity(AUDIO_BATCH * 4)));

    let desired = AudioSpecDesired {
        freq: Some(44_100),
        channels: Some(1),
        samples: Some(AUDIO_BATCH as u16),
    };

    let device = audio.open_playback(None, &desired, |spec| {
        println!("Opened audio: {:?}", spec);
        PcmStream {
            buffer: shared.clone(),
        }
    })?;

    device.resume();

    let shared_thread = shared.clone();
    let rom_thread = rom;
    thread::spawn(move || {
        let mut emu = Emulator::new();
        if is_pce {
            if let Err(err) = emu.load_hucard(&rom_thread) {
                eprintln!("failed to load HuCard: {err}");
                return;
            }
        } else {
            emu.load_program(0xC000, &rom_thread);
        }
        emu.reset();
        loop {
            emu.tick();
            if let Some(samples) = emu.take_audio_samples() {
                let mut guard = shared_thread.lock().unwrap();
                for sample in samples {
                    guard.push_back(sample);
                }
            }
            if emu.cpu.halted {
                break;
            }
        }
    });

    while device.status() == AudioStatus::Playing {
        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
