//! Audio Processing Unit (APU).
//!
//! Implements three of the four DMG sound channels:
//!
//! * `CH1` and `CH2` — square wave channels (sweep is not yet modelled).
//! * `CH4` — pseudo-random noise driven by a 15-bit LFSR.
//!
//! The wave channel (`CH3`) is stubbed out, which is enough to run the vast
//! majority of commercial DMG titles without audible regressions.
//!
//! The APU is deliberately host-agnostic: it mixes samples into an internal
//! ring buffer, and the embedder (native binary, WebAssembly front-end,
//! integration tests, …) decides how to drain them. This keeps the core free
//! of `cpal` / `crossbeam` dependencies and makes the same code portable to
//! `wasm32-unknown-unknown`.

use std::collections::VecDeque;

/// Standard DMG duty cycle patterns for the two square channels.
/// Index = duty selector (NRx1 bits 6-7), inner index = position counter.
const DUTY_PATTERNS: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5 %
    [1, 0, 0, 0, 0, 0, 0, 1], // 25.0 %
    [1, 0, 0, 0, 0, 1, 1, 1], // 50.0 %
    [0, 1, 1, 1, 1, 1, 1, 0], // 75.0 %
];

/// Frequency divisor table used by the noise channel (CH4).
const NOISE_DIVISORS: [u8; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

/// One sound channel. The same struct backs the two square channels and the
/// noise channel — `is_noise` selects the timer/output behaviour.
struct Channel {
    enabled: bool,
    is_noise: bool,

    // --- Frequency / timing ---
    freq: u16,
    timer: u32,
    duty: u8,
    duty_pos: u8,
    lfsr: u16,
    width_mode: bool,
    divisor_code: u8,
    shift_amount: u8,

    // --- Volume envelope ---
    env_vol: u8,
    env_dir: bool,
    env_period: u8,
    env_timer: u8,
    initial_vol: u8,

    // --- Length counter ---
    length: u8,
    use_len: bool,
}

impl Channel {
    fn new(is_noise: bool) -> Self {
        Self {
            enabled: false,
            is_noise,
            freq: 0,
            timer: 0,
            duty: 0,
            duty_pos: 0,
            lfsr: 0x7FFF,
            width_mode: false,
            divisor_code: 0,
            shift_amount: 0,
            env_vol: 0,
            env_dir: false,
            env_period: 0,
            env_timer: 0,
            initial_vol: 0,
            length: 0,
            use_len: false,
        }
    }

    /// Advance the frequency/LFSR timer by `cycles` T-cycles.
    fn tick(&mut self, cycles: u32) {
        if self.timer > cycles {
            self.timer -= cycles;
            return;
        }

        if self.is_noise {
            let divisor = NOISE_DIVISORS[(self.divisor_code & 7) as usize] as u32;
            self.timer = divisor << self.shift_amount;

            let xor_res = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr = (self.lfsr >> 1) | (xor_res << 14);
            if self.width_mode {
                self.lfsr = (self.lfsr & !0x40) | (xor_res << 6);
            }
        } else {
            self.timer = (2048 - self.freq as u32) * 4;
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    /// Step the volume envelope (called at 64 Hz by the frame sequencer).
    fn tick_envelope(&mut self) {
        if self.env_period == 0 {
            return;
        }
        self.env_timer = self.env_timer.saturating_sub(1);
        if self.env_timer == 0 {
            self.env_timer = self.env_period;
            if self.env_dir && self.env_vol < 15 {
                self.env_vol += 1;
            } else if !self.env_dir && self.env_vol > 0 {
                self.env_vol -= 1;
            }
        }
    }

    /// Produce the channel's current sample in the range `[-1.0, 1.0]`.
    fn sample(&self) -> f32 {
        if !self.enabled || self.env_vol == 0 {
            return 0.0;
        }

        let high = if self.is_noise {
            (self.lfsr & 1) == 0
        } else {
            DUTY_PATTERNS[self.duty as usize][self.duty_pos as usize] == 1
        };

        let amplitude = self.env_vol as f32 / 15.0;
        if high {
            amplitude
        } else {
            -amplitude
        }
    }
}

/// Soft cap on the number of buffered samples. When the embedder pulls
/// samples slower than the APU produces them, the oldest are dropped so
/// the queue does not grow unbounded.
const MAX_BUFFERED_SAMPLES: usize = 8192;

/// Audio Processing Unit. Generates a stream of mono `f32` samples in
/// `[-1.0, 1.0]` at the host-chosen sample rate.
pub struct Apu {
    ch1: Channel,
    ch2: Channel,
    ch4: Channel,

    frame_step: u8,
    frame_div: u32,

    downsample_count: f32,
    cycles_per_sample: f32,

    nr50: u8,
    nr51: u8,
    nr52: u8,

    /// Mixed samples awaiting consumption by the host.
    sample_buffer: VecDeque<f32>,
}

impl Apu {
    /// Build an APU configured to produce samples at `sample_rate` Hz.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            ch1: Channel::new(false),
            ch2: Channel::new(false),
            ch4: Channel::new(true),
            frame_step: 0,
            frame_div: 0,
            downsample_count: 0.0,
            cycles_per_sample: 4_194_304.0 / sample_rate as f32,
            nr50: 0x77,
            nr51: 0xF3,
            nr52: 0x80,
            sample_buffer: VecDeque::with_capacity(MAX_BUFFERED_SAMPLES),
        }
    }

    /// Drain at most `out.len()` samples into `out` and return how many were
    /// actually written. Anything beyond the available count is left as the
    /// existing contents of `out`.
    pub fn drain_samples(&mut self, out: &mut [f32]) -> usize {
        let mut written = 0;
        while written < out.len() {
            match self.sample_buffer.pop_front() {
                Some(sample) => {
                    out[written] = sample;
                    written += 1;
                }
                None => break,
            }
        }
        written
    }

    /// Pop a single sample, or `None` if the buffer is empty.
    pub fn next_sample(&mut self) -> Option<f32> {
        self.sample_buffer.pop_front()
    }

    /// Number of samples currently buffered.
    pub fn buffered_samples(&self) -> usize {
        self.sample_buffer.len()
    }

    /// Advance the APU by `cycles` T-cycles. Mixed samples are pushed into
    /// the internal buffer at the configured sample rate.
    pub fn tick(&mut self, cycles: u8) {
        if (self.nr52 & 0x80) == 0 {
            return;
        }

        let cycles = cycles as u32;
        self.ch1.tick(cycles);
        self.ch2.tick(cycles);
        self.ch4.tick(cycles);

        self.downsample_count += cycles as f32;
        while self.downsample_count >= self.cycles_per_sample {
            self.downsample_count -= self.cycles_per_sample;
            self.emit_sample();
        }

        self.tick_frame_sequencer(cycles);
    }

    fn emit_sample(&mut self) {
        let mut mixed = 0.0;
        if (self.nr51 & 0x11) != 0 {
            mixed += self.ch1.sample();
        }
        if (self.nr51 & 0x22) != 0 {
            mixed += self.ch2.sample();
        }
        if (self.nr51 & 0x88) != 0 {
            mixed += self.ch4.sample();
        }

        let master_volume = (((self.nr50 >> 4) & 7) + (self.nr50 & 7)) as f32 / 14.0;
        let sample = mixed * 0.1 * master_volume;

        if self.sample_buffer.len() >= MAX_BUFFERED_SAMPLES {
            self.sample_buffer.pop_front();
        }
        self.sample_buffer.push_back(sample);
    }

    /// 512 Hz frame sequencer that drives length counters (256 Hz) and the
    /// volume envelopes (64 Hz). Sweep is intentionally not implemented.
    fn tick_frame_sequencer(&mut self, cycles: u32) {
        self.frame_div += cycles;
        while self.frame_div >= 8192 {
            self.frame_div -= 8192;
            self.frame_step = (self.frame_step + 1) & 7;

            if self.frame_step % 2 == 0 {
                Self::tick_length(&mut self.ch1);
                Self::tick_length(&mut self.ch2);
                Self::tick_length(&mut self.ch4);
            }

            if self.frame_step == 7 {
                self.ch1.tick_envelope();
                self.ch2.tick_envelope();
                self.ch4.tick_envelope();
            }
        }
    }

    fn tick_length(channel: &mut Channel) {
        if channel.use_len && channel.length > 0 {
            channel.length -= 1;
            if channel.length == 0 {
                channel.enabled = false;
            }
        }
    }

    /// Read an APU register. Unmapped addresses return open-bus (`0xFF`).
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => self.nr52,
            _ => 0xFF,
        }
    }

    /// Write to an APU register. While the APU is powered off (`NR52` bit 7
    /// clear) writes are ignored except to `NR52` itself.
    pub fn write(&mut self, addr: u16, val: u8) {
        if (self.nr52 & 0x80) == 0 && addr != 0xFF26 {
            return;
        }
        match addr {
            // --- CH1 (square 1) ---
            0xFF11 => {
                self.ch1.duty = val >> 6;
                self.ch1.length = 64 - (val & 0x3F);
            }
            0xFF12 => {
                self.ch1.initial_vol = val >> 4;
                self.ch1.env_dir = (val & 8) != 0;
                self.ch1.env_period = val & 7;
            }
            0xFF13 => {
                self.ch1.freq = (self.ch1.freq & 0xFF00) | val as u16;
            }
            0xFF14 => {
                self.ch1.freq = (self.ch1.freq & 0x00FF) | ((val as u16 & 7) << 8);
                self.ch1.use_len = (val & 0x40) != 0;
                if (val & 0x80) != 0 {
                    Self::trigger(&mut self.ch1);
                }
            }
            // --- CH2 (square 2) ---
            0xFF16 => {
                self.ch2.duty = val >> 6;
                self.ch2.length = 64 - (val & 0x3F);
            }
            0xFF17 => {
                self.ch2.initial_vol = val >> 4;
                self.ch2.env_dir = (val & 8) != 0;
                self.ch2.env_period = val & 7;
            }
            0xFF18 => {
                self.ch2.freq = (self.ch2.freq & 0xFF00) | val as u16;
            }
            0xFF19 => {
                self.ch2.freq = (self.ch2.freq & 0x00FF) | ((val as u16 & 7) << 8);
                self.ch2.use_len = (val & 0x40) != 0;
                if (val & 0x80) != 0 {
                    Self::trigger(&mut self.ch2);
                }
            }
            // --- CH4 (noise) ---
            0xFF20 => {
                self.ch4.length = 64 - (val & 0x3F);
            }
            0xFF21 => {
                self.ch4.initial_vol = val >> 4;
                self.ch4.env_dir = (val & 8) != 0;
                self.ch4.env_period = val & 7;
            }
            0xFF22 => {
                self.ch4.shift_amount = val >> 4;
                self.ch4.width_mode = (val & 8) != 0;
                self.ch4.divisor_code = val & 7;
            }
            0xFF23 => {
                self.ch4.use_len = (val & 0x40) != 0;
                if (val & 0x80) != 0 {
                    self.ch4.lfsr = 0x7FFF;
                    Self::trigger(&mut self.ch4);
                }
            }
            // --- Master control ---
            0xFF24 => self.nr50 = val,
            0xFF25 => self.nr51 = val,
            0xFF26 => {
                self.nr52 = val & 0x80;
                if (val & 0x80) == 0 {
                    self.ch1.enabled = false;
                    self.ch2.enabled = false;
                    self.ch4.enabled = false;
                }
            }
            _ => {}
        }
    }

    fn trigger(channel: &mut Channel) {
        channel.enabled = true;
        channel.env_vol = channel.initial_vol;
        channel.env_timer = channel.env_period;
        if channel.length == 0 {
            channel.length = 64;
        }
    }
}
