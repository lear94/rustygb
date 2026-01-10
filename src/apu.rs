use crossbeam_channel::Sender;

const DUTY_PATTERNS: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

struct Channel {
    enabled: bool,
    is_noise: bool,

    // Oscillator
    freq: u16,
    timer: u32,
    duty: u8,
    duty_pos: u8,
    lfsr: u16,
    width_mode: bool,
    divisor_code: u8,
    shift_amount: u8, // Noise specific

    // Envelope
    env_vol: u8,
    env_dir: bool,
    env_period: u8,
    env_timer: u8,
    initial_vol: u8,

    // Length
    length: u8,
    use_len: bool,
}

impl Channel {
    fn new(noise: bool) -> Self {
        Self {
            enabled: false,
            is_noise: noise,
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

    fn tick(&mut self, cycles: u32) {
        if self.timer > cycles {
            self.timer -= cycles;
        } else {
            if self.is_noise {
                let divisors = [8, 16, 32, 48, 64, 80, 96, 112];
                let div = divisors[(self.divisor_code & 7) as usize];
                self.timer = (div as u32) << self.shift_amount;

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
    }

    fn tick_envelope(&mut self) {
        if self.env_period > 0 {
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
    }

    fn sample(&self) -> f32 {
        if !self.enabled || self.env_vol == 0 {
            return 0.0;
        }

        let output = if self.is_noise {
            if (self.lfsr & 1) == 0 {
                1
            } else {
                0
            }
        } else {
            DUTY_PATTERNS[self.duty as usize][self.duty_pos as usize]
        };

        if output == 1 {
            (self.env_vol as f32) / 15.0
        } else {
            -(self.env_vol as f32) / 15.0
        }
    }
}

pub struct Apu {
    pub sender: Option<Sender<f32>>,
    ch1: Channel,
    ch2: Channel,
    ch4: Channel, // Simplified: 3 main channels

    frame_step: u8,
    downsample_count: f32,
    cycles_per_sample: f32,
    nr50: u8,
    nr51: u8,
    nr52: u8,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sender: None,
            ch1: Channel::new(false),
            ch2: Channel::new(false),
            ch4: Channel::new(true),
            frame_step: 0,
            downsample_count: 0.0,
            cycles_per_sample: 4194304.0 / sample_rate as f32,
            nr50: 0x77,
            nr51: 0xF3,
            nr52: 0x80,
        }
    }

    pub fn tick(&mut self, cycles: u8) {
        if (self.nr52 & 0x80) == 0 {
            return;
        }
        let c = cycles as u32;
        self.ch1.tick(c);
        self.ch2.tick(c);
        self.ch4.tick(c);

        // Audio Output
        self.downsample_count += cycles as f32;
        while self.downsample_count >= self.cycles_per_sample {
            self.downsample_count -= self.cycles_per_sample;

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

            let vol = (((self.nr50 >> 4) & 7) + (self.nr50 & 7)) as f32 / 14.0;
            if let Some(tx) = &self.sender {
                let _ = tx.try_send(mixed * 0.1 * vol);
            }
        }

        // Frame Sequencer (512Hz)
        static mut FRAME_DIV: u32 = 0;
        unsafe {
            FRAME_DIV += c;
            if FRAME_DIV >= 8192 {
                FRAME_DIV -= 8192;
                self.frame_step = (self.frame_step + 1) & 7;

                // Length Control (256Hz)
                if self.frame_step % 2 == 0 {
                    if self.ch1.use_len && self.ch1.length > 0 {
                        self.ch1.length -= 1;
                        if self.ch1.length == 0 {
                            self.ch1.enabled = false;
                        }
                    }
                    if self.ch2.use_len && self.ch2.length > 0 {
                        self.ch2.length -= 1;
                        if self.ch2.length == 0 {
                            self.ch2.enabled = false;
                        }
                    }
                    if self.ch4.use_len && self.ch4.length > 0 {
                        self.ch4.length -= 1;
                        if self.ch4.length == 0 {
                            self.ch4.enabled = false;
                        }
                    }
                }

                // Volume Envelope (64Hz)
                if self.frame_step == 7 {
                    self.ch1.tick_envelope();
                    self.ch2.tick_envelope();
                    self.ch4.tick_envelope();
                }
            }
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF24 => self.nr50,
            0xFF25 => self.nr51,
            0xFF26 => self.nr52,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        if (self.nr52 & 0x80) == 0 && addr != 0xFF26 {
            return;
        }
        match addr {
            // CH1
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
                    self.ch1.enabled = true;
                    self.ch1.env_vol = self.ch1.initial_vol;
                    self.ch1.env_timer = self.ch1.env_period;
                    if self.ch1.length == 0 {
                        self.ch1.length = 64;
                    }
                }
            }
            // CH2
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
                    self.ch2.enabled = true;
                    self.ch2.env_vol = self.ch2.initial_vol;
                    self.ch2.env_timer = self.ch2.env_period;
                    if self.ch2.length == 0 {
                        self.ch2.length = 64;
                    }
                }
            }
            // CH4 (Noise)
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
                    self.ch4.enabled = true;
                    self.ch4.env_vol = self.ch4.initial_vol;
                    self.ch4.env_timer = self.ch4.env_period;
                    self.ch4.lfsr = 0x7FFF;
                    if self.ch4.length == 0 {
                        self.ch4.length = 64;
                    }
                }
            }
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
}
