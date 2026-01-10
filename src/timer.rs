pub struct Timer {
    pub div: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    last_bit: bool,
    overflow: bool, // Buffer for interrupt delay (Hardware quirk)
}

impl Timer {
    pub fn new() -> Self {
        Self {
            div: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            last_bit: false,
            overflow: false,
        }
    }

    pub fn tick(&mut self, cycles: u8, if_reg: &mut u8) {
        // Do not multiply by 4. 'cycles' are already T-Cycles.
        let inc = cycles as u16;

        // Delayed Overflow Handling (1 M-Clock cycle)
        if self.overflow {
            self.tima = self.tma;
            *if_reg |= 4; // Request Timer Interrupt
            self.overflow = false;
        }

        // DIV is a 16-bit counter that always increments
        self.div = self.div.wrapping_add(inc);

        // Frequency Multiplexer
        // 00: bit 9, 01: bit 3, 10: bit 5, 11: bit 7
        let bit_idx = match self.tac & 3 {
            0 => 9,
            1 => 3,
            2 => 5,
            _ => 7,
        };

        let timer_enable = (self.tac & 4) != 0;
        let bit_value = (self.div >> bit_idx) & 1;
        let new_bit = timer_enable && (bit_value != 0);

        // Falling Edge Detection
        if self.last_bit && !new_bit {
            // Increment TIMA
            let (result, ov) = self.tima.overflowing_add(1);
            self.tima = result;

            if ov {
                // Real hardware takes a moment to reload and request the INT
                self.overflow = true;
            }
        }
        self.last_bit = new_bit;
    }

    pub fn read(&self, a: u16) -> u8 {
        match a {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, a: u16, v: u8) {
        match a {
            0xFF04 => self.div = 0, // Reset DIV
            0xFF05 => self.tima = v,
            0xFF06 => self.tma = v,
            0xFF07 => self.tac = v,
            _ => {}
        }
    }
}
