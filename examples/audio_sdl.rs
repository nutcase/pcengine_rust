use pce::emulator::Emulator;
use sdl2::audio::{AudioCallback, AudioSpecDesired, AudioStatus};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Internal emulator sample rate (must match AUDIO_SAMPLE_RATE in bus.rs).
const EMU_SAMPLE_RATE: u32 = 44_100;

/// Target buffer size in samples – enough for ~50 ms of audio.
const TARGET_BUFFER: usize = 2205;
/// Maximum buffer before throttling – ~200 ms.
const MAX_BUFFER: usize = 8820;

struct PcmStream {
    buffer: Arc<Mutex<VecDeque<i16>>>,
    /// Resampling state: converts from EMU_SAMPLE_RATE to the actual device rate.
    resample_ratio: f64, // device_rate / EMU_SAMPLE_RATE
    resample_phase: f64,
    prev_sample: i16,
    underrun_count: Arc<AtomicU64>,
}

impl AudioCallback for PcmStream {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        let mut guard = self.buffer.lock().unwrap();

        if self.resample_ratio == 1.0 {
            // No resampling needed – fast path.
            for sample in out.iter_mut() {
                *sample = guard.pop_front().unwrap_or_else(|| {
                    self.underrun_count.fetch_add(1, Ordering::Relaxed);
                    self.prev_sample // repeat last sample instead of zero (less click)
                });
                self.prev_sample = *sample;
            }
        } else {
            // Linear interpolation resampling.
            let step = 1.0 / self.resample_ratio; // how much to advance in source per output sample
            for sample in out.iter_mut() {
                // Consume whole source samples that we've moved past.
                let skip = self.resample_phase as usize;
                for _ in 0..skip {
                    if guard.len() > 1 {
                        self.prev_sample = guard.pop_front().unwrap();
                    }
                }
                self.resample_phase -= skip as f64;

                let frac = self.resample_phase;
                let s0 = guard.front().copied().unwrap_or(self.prev_sample);
                let s1 = guard.get(1).copied().unwrap_or(s0);
                let interp = s0 as f64 * (1.0 - frac) + s1 as f64 * frac;
                *sample = interp as i16;
                self.resample_phase += step;
            }
        }
    }
}

fn main() -> Result<(), String> {
    let rom_path = std::env::args().nth(1).ok_or_else(|| {
        "usage: cargo run --release --example audio_sdl --features audio-sdl -- <rom.pce>"
            .to_string()
    })?;
    let rom = std::fs::read(&rom_path).map_err(|e| format!("failed to read ROM: {e}"))?;
    let is_pce = Path::new(&rom_path)
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("pce"))
        .unwrap_or(false);

    let sdl = sdl2::init()?;
    let audio = sdl.audio()?;
    let shared = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_BUFFER)));
    let underrun_count = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));

    let desired = AudioSpecDesired {
        freq: Some(EMU_SAMPLE_RATE as i32),
        channels: Some(1),
        samples: Some(1024),
    };

    let underrun_cb = underrun_count.clone();
    let device = audio.open_playback(None, &desired, |spec| {
        let actual_rate = spec.freq as u32;
        let ratio = actual_rate as f64 / EMU_SAMPLE_RATE as f64;
        eprintln!(
            "Audio device: {} Hz, {} ch, {} samples/cb (ratio={:.4})",
            spec.freq, spec.channels, spec.samples, ratio
        );
        if (ratio - 1.0).abs() > 0.001 {
            eprintln!(
                "WARNING: Device rate ({}) != emulator rate ({}), resampling active",
                actual_rate, EMU_SAMPLE_RATE
            );
        }
        PcmStream {
            buffer: shared.clone(),
            resample_ratio: ratio,
            resample_phase: 0.0,
            prev_sample: 0,
            underrun_count: underrun_cb,
        }
    })?;

    // Pre-buffer audio before starting playback.
    let shared_thread = shared.clone();
    let running_thread = running.clone();
    let emu_handle = thread::spawn(move || {
        let mut emu = Emulator::new();
        if is_pce {
            if let Err(err) = emu.load_hucard(&rom) {
                eprintln!("failed to load HuCard: {err}");
                return;
            }
        } else {
            emu.load_program(0xC000, &rom);
        }
        emu.reset();
        emu.set_audio_batch_size(128); // ~3ms chunks, balances latency and overhead

        while running_thread.load(Ordering::Relaxed) {
            emu.tick();

            if let Some(samples) = emu.take_audio_samples() {
                let mut guard = shared_thread.lock().unwrap();
                for sample in samples {
                    guard.push_back(sample);
                }
                let len = guard.len();
                drop(guard);

                // Throttle if buffer is too full (don't run too far ahead).
                if len > MAX_BUFFER {
                    thread::sleep(Duration::from_millis(10));
                }
            }

            if emu.cpu.halted {
                break;
            }
        }
    });

    // Wait for pre-buffer to fill before starting playback.
    eprintln!("Pre-buffering...");
    loop {
        let len = shared.lock().unwrap().len();
        if len >= TARGET_BUFFER {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    eprintln!("Starting playback");
    device.resume();

    // Monitor loop.
    let start = Instant::now();
    let mut last_underrun = 0u64;
    while device.status() == AudioStatus::Playing && running.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(500));
        let buf_len = shared.lock().unwrap().len();
        let underruns = underrun_count.load(Ordering::Relaxed);
        let elapsed = start.elapsed().as_secs_f64();
        if underruns > last_underrun {
            eprintln!(
                "[{:.1}s] buffer={} underruns={} (try: cargo run --release)",
                elapsed, buf_len, underruns
            );
            last_underrun = underruns;
        }
    }

    running.store(false, Ordering::Relaxed);
    let _ = emu_handle.join();

    Ok(())
}
