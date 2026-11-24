//! Real-time clock of MBC3 cartridges.
//!
//! Time advances with emulated cycles; [`Rtc::restore`] and [`Rtc::sync`] add the time the
//! cartridge spent unplugged. The save footer is the 48-byte layout shared by other
//! emulators: five live and five latched registers as little-endian words, then a Unix
//! timestamp.

/// Footer with a 64-bit timestamp, the one written.
pub(super) const FOOTER_LEN: usize = 48;
/// Footer with a 32-bit timestamp, accepted on load.
pub(super) const SHORT_FOOTER_LEN: usize = 44;
const REGISTER_COUNT: usize = 5;
const WORD_LEN: usize = 4;
const TIMESTAMP_OFFSET: usize = 2 * REGISTER_COUNT * WORD_LEN;

const M_CYCLES_PER_SECOND: u32 = 1_048_576;
const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;
const SECONDS_PER_DAY: u64 = SECONDS_PER_MINUTE * MINUTES_PER_HOUR * HOURS_PER_DAY;
const DAY_COUNTER_LEN: u64 = 512;

const LAST_SECOND: u8 = 59;
const LAST_MINUTE: u8 = 59;
const LAST_HOUR: u8 = 23;
const DAY_HIGH: u8 = 0x01;
const HALT: u8 = 0x40;
const DAY_CARRY: u8 = 0x80;
/// Bits that exist in each register: seconds, minutes, hours, day low, day high.
const REGISTER_MASKS: [u8; REGISTER_COUNT] = [0x3F, 0x3F, 0x1F, 0xFF, HALT | DAY_CARRY | DAY_HIGH];
/// Bank-select value of the first register.
const FIRST_SELECT: u8 = 0x08;

/// Clock registers: seconds, minutes, hours, day low, day high and flags.
type Registers = [u8; REGISTER_COUNT];

/// Real-time clock.
#[derive(Debug, Default)]
pub(super) struct Rtc {
    live: Registers,
    latched: Registers,
    sub_second_m_cycles: u32,
    /// Moment the restored registers were current, until the host syncs.
    saved_at: Option<u64>,
    latch_armed: bool,
}

impl Rtc {
    /// Advances one M-cycle.
    pub(super) fn tick(&mut self) {
        if self.is_halted() {
            return;
        }
        self.sub_second_m_cycles += 1;
        if self.sub_second_m_cycles == M_CYCLES_PER_SECOND {
            self.sub_second_m_cycles = 0;
            self.advance_second();
        }
    }

    /// Reads the latched register at bank-select value `select`.
    pub(super) fn read(&self, select: u8) -> u8 {
        self.latched
            .get(register_idx(select))
            .copied()
            .unwrap_or(0xFF)
    }

    /// Writes the live register at bank-select value `select`.
    pub(super) fn write(&mut self, select: u8, val: u8) {
        let idx = register_idx(select);
        let (Some(register), Some(mask)) = (self.live.get_mut(idx), REGISTER_MASKS.get(idx)) else {
            return;
        };
        *register = val & mask;
        if idx == 0 {
            self.sub_second_m_cycles = 0;
        }
        // Hardware exposes the write at once, without a new latch.
        if let Some(latched) = self.latched.get_mut(idx) {
            *latched = val & mask;
        }
    }

    /// Writes the latch register: 0 then 1 captures the live registers.
    pub(super) fn write_latch(&mut self, val: u8) {
        if self.latch_armed && val == 1 {
            self.latched = self.live;
        }
        self.latch_armed = val == 0;
    }

    /// Adds the time elapsed since the save was written.
    pub(super) fn sync(&mut self, unix_now_s: u64) {
        let Some(saved_at) = self.saved_at.take() else {
            return;
        };
        if !self.is_halted() {
            self.advance_seconds(unix_now_s.saturating_sub(saved_at));
        }
    }

    /// Serializes the clock as a save footer.
    pub(super) fn footer(&self, unix_now_s: u64) -> Vec<u8> {
        let registers = self.live.iter().chain(&self.latched);
        let mut footer: Vec<u8> = registers
            .flat_map(|&register| u32::from(register).to_le_bytes())
            .collect();
        footer.extend(self.saved_at.unwrap_or(unix_now_s).to_le_bytes());
        footer
    }

    /// Restores a footer; one of unknown length leaves the clock unchanged.
    pub(super) fn restore(&mut self, footer: &[u8]) {
        if footer.len() != FOOTER_LEN && footer.len() != SHORT_FOOTER_LEN {
            return;
        }
        let mut words = footer.chunks(WORD_LEN).map(|word| word.first().copied());
        let registers = self.live.iter_mut().chain(&mut self.latched);
        for (register, (byte, mask)) in
            registers.zip(words.by_ref().zip(REGISTER_MASKS.iter().cycle()))
        {
            *register = byte.unwrap_or(0) & mask;
        }
        let timestamp = footer.get(TIMESTAMP_OFFSET..).unwrap_or_default();
        self.saved_at = Some(
            timestamp
                .iter()
                .rev()
                .fold(0, |value, &byte| (value << u8::BITS) | u64::from(byte)),
        );
    }

    fn is_halted(&self) -> bool {
        let [.., flags] = self.live;
        flags & HALT != 0
    }

    fn advance_second(&mut self) {
        let [second, minute, hour, _, _] = &mut self.live;
        // Out-of-range values wrap at the register width without carrying.
        let carried = step(second, LAST_SECOND, REGISTER_MASKS[0])
            && step(minute, LAST_MINUTE, REGISTER_MASKS[1])
            && step(hour, LAST_HOUR, REGISTER_MASKS[2]);
        if carried {
            self.advance_days(1);
        }
    }

    /// Advances whole days arithmetically and the rest in range-checked steps.
    ///
    /// # Complexity
    /// O(1): at most one day of seconds is stepped. Runs once per load.
    fn advance_seconds(&mut self, seconds: u64) {
        self.advance_days(seconds / SECONDS_PER_DAY);
        for _ in 0..seconds % SECONDS_PER_DAY {
            self.advance_second();
        }
    }

    fn advance_days(&mut self, days: u64) {
        let [.., day_low, flags] = &mut self.live;
        let day = u64::from(u16::from_be_bytes([*flags & DAY_HIGH, *day_low]));
        let total = day.saturating_add(days);
        let [low, high, ..] = (total % DAY_COUNTER_LEN).to_le_bytes();
        *day_low = low;
        *flags = (*flags & !DAY_HIGH) | high;
        if total >= DAY_COUNTER_LEN {
            *flags |= DAY_CARRY;
        }
    }
}

/// Increments a register and returns whether it carried past `last`.
fn step(register: &mut u8, last: u8, mask: u8) -> bool {
    let carried = *register == last;
    *register = if carried {
        0
    } else {
        register.wrapping_add(1) & mask
    };
    carried
}

/// Index of a bank-select value; below the first register, wraps beyond any index.
fn register_idx(select: u8) -> usize {
    usize::from(select.wrapping_sub(FIRST_SELECT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtc(live: Registers) -> Rtc {
        Rtc {
            live,
            ..Rtc::default()
        }
    }

    #[test]
    fn advance_second_carries() {
        for (before, after) in [
            ([0, 0, 0, 0, 0], [1, 0, 0, 0, 0]),
            ([59, 0, 0, 0, 0], [0, 1, 0, 0, 0]),
            ([59, 59, 22, 0, 0], [0, 0, 23, 0, 0]),
            ([59, 59, 23, 255, 0], [0, 0, 0, 0, 1]),
            ([59, 59, 23, 255, 1], [0, 0, 0, 0, DAY_CARRY]),
            ([63, 59, 0, 0, 0], [0, 59, 0, 0, 0]),
            ([59, 63, 0, 0, 0], [0, 0, 0, 0, 0]),
            ([59, 59, 31, 0, 0], [0, 0, 0, 0, 0]),
        ] {
            let mut rtc = rtc(before);
            rtc.advance_second();
            assert_eq!(rtc.live, after, "{before:?}");
        }
    }

    #[test]
    fn tick_counts_emulated_seconds_unless_halted() {
        for (flags, expected) in [(0, 1), (HALT, 0)] {
            let mut rtc = rtc([0, 0, 0, 0, flags]);
            (0..M_CYCLES_PER_SECOND).for_each(|_| rtc.tick());
            assert_eq!(rtc.live[0], expected);
        }
    }

    #[test]
    fn write_latch_captures_on_zero_then_one() {
        let mut rtc = rtc([7, 0, 0, 0, 0]);
        rtc.write_latch(1);
        assert_eq!(rtc.read(0x08), 0);
        rtc.write_latch(0);
        rtc.write_latch(1);
        assert_eq!(rtc.read(0x08), 7);
    }

    #[test]
    fn write_masks_registers_and_resets_sub_second() {
        let mut rtc = Rtc::default();
        rtc.tick();
        for (select, expected) in [(8, 0x3F), (9, 0x3F), (10, 0x1F), (11, 0xFF), (12, 0xC1)] {
            rtc.write(select, 0xFF);
            assert_eq!(rtc.read(select), expected);
        }
        assert_eq!(rtc.sub_second_m_cycles, 0);
        rtc.write(7, 1);
        rtc.write(13, 1);
        assert_eq!((rtc.read(7), rtc.read(13)), (0xFF, 0xFF));
    }

    #[test]
    fn sync_adds_unplugged_time_once() {
        let mut source = rtc([58, 59, 23, 0xFF, 1]);
        source.latched = [1, 2, 3, 4, 1];
        let footer = source.footer(1_000);
        assert_eq!(footer.len(), FOOTER_LEN);
        for len in [FOOTER_LEN, SHORT_FOOTER_LEN] {
            let mut restored = Rtc::default();
            restored.restore(&footer[..len]);
            assert_eq!(restored.latched, source.latched);
            restored.sync(1_002 + SECONDS_PER_DAY);
            assert_eq!(restored.live, [0, 0, 0, 1, DAY_CARRY]);
            restored.sync(u64::MAX);
            assert_eq!(restored.live, [0, 0, 0, 1, DAY_CARRY]);
        }
    }

    #[test]
    fn sync_extreme_clocks_do_not_overflow() {
        for (saved_at, now) in [(0, u64::MAX), (u64::MAX, 0)] {
            let mut clock = Rtc::default();
            clock.restore(&rtc([0; 5]).footer(saved_at));
            clock.sync(now);
            assert!(clock.live[0] < 60);
        }
    }

    #[test]
    fn sync_halted_clock_stays() {
        let mut clock = Rtc::default();
        clock.restore(&rtc([1, 0, 0, 0, HALT]).footer(0));
        clock.sync(1_000);
        assert_eq!(clock.live, [1, 0, 0, 0, HALT]);
    }

    #[test]
    fn footer_before_sync_keeps_original_moment() {
        let mut clock = Rtc::default();
        clock.restore(&rtc([0; 5]).footer(5));
        assert_eq!(clock.footer(99), rtc([0; 5]).footer(5));
    }

    #[test]
    fn restore_unknown_length_is_ignored() {
        let mut clock = rtc([9, 0, 0, 0, 0]);
        clock.restore(&[0; 47]);
        assert_eq!(clock.live[0], 9);
    }
}
