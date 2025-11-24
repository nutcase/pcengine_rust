use pce::emulator::Emulator;
use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioStatus};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const AUDIO_BATCH: usize = 1024;

struct PcmStream {
    buffer: Arc<Mutex<Vec<i16>>>,
}

impl AudioCallback for PcmStream {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        let mut guard = self.buffer.lock().unwrap();
        for sample in out.iter_mut() {
            *sample = guard.pop().unwrap_or(0);
        }
    }
}

fn main() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let audio = sdl.audio()?;
    let shared = Arc::new(Mutex::new(Vec::with_capacity(AUDIO_BATCH * 2)));

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
    thread::spawn(move || {
        let mut emu = Emulator::new();
        emu.reset();
        loop {
            emu.tick();
            if let Some(samples) = emu.take_audio_samples() {
                let mut guard = shared_thread.lock().unwrap();
                guard.extend(samples);
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
