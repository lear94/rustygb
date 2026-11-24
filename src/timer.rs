//! DMG timer block: DIV, TIMA, TMA and TAC.

use crate::hardware::{OPEN_BUS, T_CYCLES_PER_M_CYCLE};
use crate::memory_map::{DIV, TAC, TIMA, TMA};

/// Divider when the first opcode fetch begins after the DMG boot ROM.
const POST_BOOT_DIVIDER: u16 = 0xABC8;
/// Divider bit falling at 512 Hz.
const AUDIO_CLOCK_BIT: u16 = 1 << 12;
/// Divider bit falling at 8192 Hz.
const SERIAL_CLOCK_BIT: u16 = 1 << 8;
const TAC_ENABLE: u8 = 0x04;
const TAC_RATE_MASK: u8 = 0x03;
const TIMA_4096_HZ_BIT: u16 = 1 << 9;
const TIMA_262144_HZ_BIT: u16 = 1 << 3;
const TIMA_65536_HZ_BIT: u16 = 1 << 5;
const TIMA_16384_HZ_BIT: u16 = 1 << 7;
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

/// What happened during one [`Timer::tick`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimerTick {
    /// The timer interrupt is requested.
    pub interrupt: bool,
    /// The audio sequencer is clocked.
    pub audio_clock: bool,
    /// The serial port is clocked.
    pub serial_clock: bool,
}

/// Divider and programmable timer.
#[derive(Debug)]
pub struct Timer {
    /// DIV is the high byte; the low byte drives the fast TIMA rates.
    divider: u16,
    tima: u8,
    tma: u8,
    tac: u8,
    /// Divider bits whose falling edge has an effect.
    watched_bits: u16,
    phase: Phase,
    /// Edges not reported yet, e.g. caused by a DIV write.
    pending: TimerTick,
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
            watched_bits: watched_bits(0),
            phase: Phase::Counting,
            pending: TimerTick::default(),
        }
    }

    /// Advances one M-cycle.
    ///
    /// The fastest rate has a 16 T-cycle period, so one M-cycle holds at most one edge.
    pub fn tick(&mut self) -> TimerTick {
        let divider = self.divider.wrapping_add(T_CYCLES_PER_M_CYCLE);
        let fallen = self.divider & !divider;
        // Nearly every tick is idle; this costs one branch instead of the full update.
        if fallen & self.watched_bits == 0
            && self.phase == Phase::Counting
            && self.pending == TimerTick::default()
        {
            self.divider = divider;
            return TimerTick::default();
        }
        let reload = self.phase == Phase::Overflowed;
        if reload {
            self.tima = self.tma;
            self.phase = Phase::Reloading;
        } else {
            self.phase = Phase::Counting;
        }
        self.change_input(divider, self.tac);
        self.pending.interrupt = reload;
        std::mem::take(&mut self.pending)
    }

    /// Clears the divider, as a DIV write or STOP does.
    pub fn reset_divider(&mut self) {
        self.change_input(0, self.tac);
    }

    /// Level of the divider bit whose fall clocks the audio sequencer.
    pub(crate) fn audio_clock_level(&self) -> bool {
        self.divider & AUDIO_CLOCK_BIT != 0
    }

    /// Reads a timer register; other addresses are open bus.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            DIV => self.divider.to_be_bytes()[0],
            TIMA => self.tima,
            TMA => self.tma,
            TAC => self.tac | TAC_UNUSED,
            _ => OPEN_BUS,
        }
    }

    /// Writes a timer register; other addresses are ignored.
    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            DIV => self.reset_divider(),
            // The reload wins over a write landing on the same M-cycle.
            TIMA if self.phase != Phase::Reloading => {
                self.tima = val;
                self.phase = Phase::Counting;
            }
            TMA => {
                self.tma = val;
                if self.phase == Phase::Reloading {
                    self.tima = val;
                }
            }
            TAC => self.change_input(self.divider, val & TAC_MASK),
            _ => {}
        }
    }

    /// TIMA counts falling edges of the selected divider bit, whatever causes them.
    fn change_input(&mut self, divider: u16, tac: u8) {
        let before = input_level(self.divider, self.tac);
        let fallen = self.divider & !divider;
        self.pending.audio_clock |= fallen & AUDIO_CLOCK_BIT != 0;
        self.pending.serial_clock |= fallen & SERIAL_CLOCK_BIT != 0;
        self.divider = divider;
        self.tac = tac;
        self.watched_bits = watched_bits(tac);
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

fn watched_bits(tac: u8) -> u16 {
    AUDIO_CLOCK_BIT | SERIAL_CLOCK_BIT | rate_bit(tac)
}

fn input_level(divider: u16, tac: u8) -> bool {
    tac & TAC_ENABLE != 0 && divider & rate_bit(tac) != 0
}

/// Divider bit counted by TIMA.
fn rate_bit(tac: u8) -> u16 {
    match tac & TAC_RATE_MASK {
        0 => TIMA_4096_HZ_BIT,
        1 => TIMA_262144_HZ_BIT,
        2 => TIMA_65536_HZ_BIT,
        _ => TIMA_16384_HZ_BIT,
    }
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
        timer.pending = TimerTick::default();
        timer
    }

    fn run(timer: &mut Timer, m_cycles: u32) -> u32 {
        (0..m_cycles)
            .map(|_| u32::from(timer.tick().interrupt))
            .sum()
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
        assert!(timer.tick().interrupt);
        assert_eq!(timer.read(TIMA), 0x42);
        assert!(!timer.tick().interrupt);
    }

    #[test]
    fn write_tima_after_overflow_cancels_reload() {
        let mut timer = timer(5);
        timer.write(TIMA, 0xFF);
        run(&mut timer, 4);
        timer.write(TIMA, 0x10);
        assert!(!timer.tick().interrupt);
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

    #[test]
    fn tick_reports_divider_clocks() {
        let mut timer = timer(0);
        let ticks: Vec<TimerTick> = (0..2048).map(|_| timer.tick()).collect();
        assert_eq!(ticks.iter().filter(|tick| tick.serial_clock).count(), 16);
        assert_eq!(ticks.iter().filter(|tick| tick.audio_clock).count(), 1);
        assert!(ticks[2047].audio_clock);
    }

    #[test]
    fn write_div_on_high_bits_clocks() {
        let mut timer = timer(0);
        run(&mut timer, 1024 + 64);
        timer.write(DIV, 0);
        let tick = timer.tick();
        assert!(tick.audio_clock && tick.serial_clock);
        assert!(!timer.tick().audio_clock);
    }
}
