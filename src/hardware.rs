//! Facts of the DMG shared by every component.

/// T-cycles per second.
pub const CLOCK_HZ: u32 = 4_194_304;
/// T-cycles in one frame, ~59.73 Hz.
pub const CYCLES_PER_FRAME: u32 = 70_224;
/// T-cycles per M-cycle, the granularity of bus accesses.
pub(crate) const T_CYCLES_PER_M_CYCLE: u16 = 4;
/// M-cycles per second.
pub(crate) const M_CYCLES_PER_SECOND: u32 = CLOCK_HZ / 4;
/// Value seen on the bus when nothing drives it.
pub(crate) const OPEN_BUS: u8 = 0xFF;

const _: () = assert!(M_CYCLES_PER_SECOND * T_CYCLES_PER_M_CYCLE as u32 == CLOCK_HZ);
