//! DMG timer block: DIV, TIMA, TMA and TAC.

/// T-cycles per call to [`Timer::tick`].
const T_CYCLES_PER_TICK: u16 = 4;
/// Divider when the first opcode fetch begins after the DMG boot ROM.
const POST_BOOT_DIVIDER: u16 = 0xABC8;
const TAC_ENABLE: u8 = 0x04;
const TAC_MASK: u8 = 0x07;
/// Bits that do not exist in TAC read as 1.
const TAC_UNUSED: u8 = 0xF8;

/// Phase of TIMA, whose reload lags its overflow by one M-cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Counting,
    Overflowed,
    Reloading,
}

/// Divider and programmable timer.
#[derive(Debug)]
pub struct Timer {
    /// DIV is the high byte; the low byte drives the fast TIMA rates.
    divider: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    phase: Phase,
}

impl Timer {
    /// Creates a timer in its post-boot state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            divider: POST_BOOT_DIVIDER,
            tima: 0,
            tma: 0,
            tac: 0,
            phase: Phase::Counting,
        }
    }

    /// Advances one M-cycle.
    ///
    /// The fastest rate has a 16 T-cycle period, so one M-cycle holds at most one edge.
    ///
    /// # Returns
    /// Whether the timer interrupt is requested.
    pub fn tick(&mut self) -> bool {
        let reload = self.phase == Phase::Overflowed;
        if reload {
            self.tima = self.tma;
            self.phase = Phase::Reloading;
        } else {
            self.phase = Phase::Counting;
        }
        self.change_input(self.divider.wrapping_add(T_CYCLES_PER_TICK), self.tac);
        reload
    }

    /// Clears the divider, as a DIV write or STOP does.
    pub fn reset_divider(&mut self) {
        self.change_input(0, self.tac);
    }

    /// Reads a timer register; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => self.divider.to_be_bytes()[0],
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | TAC_UNUSED,
            _ => 0xFF,
        }
    }

    /// Writes a timer register; other addresses are ignored.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0xFF04 => self.reset_divider(),
            // The reload wins over a write landing on the same M-cycle.
            0xFF05 if self.phase != Phase::Reloading => {
                self.tima = val;
                self.phase = Phase::Counting;
            }
            0xFF06 => {
                self.tma = val;
                if self.phase == Phase::Reloading {
                    self.tima = val;
                }
            }
            0xFF07 => self.change_input(self.divider, val & TAC_MASK),
            _ => {}
        }
    }

    /// TIMA counts falling edges of the selected divider bit, whatever causes them.
    fn change_input(&mut self, divider: u16, tac: u8) {
        let before = input_level(self.divider, self.tac);
        self.divider = divider;
        self.tac = tac;
        if before && !input_level(divider, tac) {
            let (tima, overflowed) = self.tima.overflowing_add(1);
            self.tima = tima;
            if overflowed {
                self.phase = Phase::Overflowed;
            }
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

fn input_level(divider: u16, tac: u8) -> bool {
    let bit = match tac & 3 {
        0 => 9,
        1 => 3,
        2 => 5,
        _ => 7,
    };
    tac & TAC_ENABLE != 0 && divider & (1 << bit) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIV: u16 = 0xFF04;
    const TIMA: u16 = 0xFF05;
    const TMA: u16 = 0xFF06;
    const TAC: u16 = 0xFF07;

    fn timer(tac: u8) -> Timer {
        let mut timer = Timer::new();
        timer.reset_divider();
        timer.write(TAC, tac);
        timer
    }

    fn run(timer: &mut Timer, m_cycles: u32) -> u32 {
        (0..m_cycles).map(|_| u32::from(timer.tick())).sum()
    }

    #[test]
    fn tick_counts_at_every_rate() {
        for (tac, period) in [(4, 256), (5, 4), (6, 16), (7, 64)] {
            let mut timer = timer(tac);
            run(&mut timer, period * 5 - 1);
            assert_eq!(timer.read(TIMA), 4, "TAC {tac}");
            timer.tick();
            assert_eq!(timer.read(TIMA), 5, "TAC {tac}");
        }
    }

    #[test]
    fn tick_disabled_does_not_count() {
        let mut timer = timer(1);
        run(&mut timer, 1024);
        assert_eq!(timer.read(TIMA), 0);
    }

    #[test]
    fn tick_overflow_reloads_one_cycle_later() {
        let mut timer = timer(5);
        timer.write(TMA, 0x42);
        timer.write(TIMA, 0xFF);
        assert_eq!(run(&mut timer, 4), 0);
        assert_eq!(timer.read(TIMA), 0);
        assert!(timer.tick());
        assert_eq!(timer.read(TIMA), 0x42);
        assert!(!timer.tick());
    }

    #[test]
    fn write_tima_after_overflow_cancels_reload() {
        let mut timer = timer(5);
        timer.write(TIMA, 0xFF);
        run(&mut timer, 4);
        timer.write(TIMA, 0x10);
        assert!(!timer.tick());
        assert_eq!(timer.read(TIMA), 0x10);
    }

    #[test]
    fn write_during_reload_follows_tma() {
        let mut timer = timer(5);
        timer.write(TIMA, 0xFF);
        run(&mut timer, 5);
        timer.write(TIMA, 0x10);
        assert_eq!(timer.read(TIMA), 0);
        timer.write(TMA, 0x20);
        assert_eq!(timer.read(TIMA), 0x20);
    }

    #[test]
    fn write_div_on_high_input_counts() {
        let mut timer = timer(5);
        run(&mut timer, 2);
        timer.write(DIV, 0xFF);
        assert_eq!((timer.read(DIV), timer.read(TIMA)), (0, 1));
    }

    #[test]
    fn write_tac_disabling_on_high_input_counts() {
        let mut timer = timer(5);
        run(&mut timer, 2);
        timer.write(TAC, 0);
        assert_eq!((timer.read(TAC), timer.read(TIMA)), (0xF8, 1));
    }

    #[test]
    fn read_div_is_high_byte() {
        let mut timer = timer(0);
        run(&mut timer, 64);
        assert_eq!(timer.read(DIV), 1);
        assert_eq!(timer.read(0), 0xFF);
    }
}
