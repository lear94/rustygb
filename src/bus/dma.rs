//! OAM DMA: copies 160 bytes, one per M-cycle.
//!
//! Only OAM is blocked meanwhile. Bus conflicts on the source bus are not modelled:
//! games run the copy loop from high RAM precisely to avoid them.

/// Bytes copied by one transfer.
const TRANSFER_LEN: u8 = 160;
/// M-cycles from the register write to the first copied byte.
const START_DELAY_M_CYCLES: u8 = 2;
/// OAM and I/O cannot be a source: the copier sees echo RAM instead.
const FIRST_ECHO_PAGE: u8 = 0xE0;
const ECHO_PAGE_OFFSET: u8 = 0x20;

/// One byte to copy.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Copy {
    pub(super) source: u16,
    pub(super) oam_idx: u8,
}

#[derive(Debug)]
pub(super) struct OamDma {
    register: u8,
    start_delay: u8,
    source_page: u8,
    next_idx: u8,
    busy: bool,
}

impl OamDma {
    pub(super) fn new() -> Self {
        Self {
            register: 0xFF,
            start_delay: 0,
            source_page: 0,
            next_idx: TRANSFER_LEN,
            busy: false,
        }
    }

    /// Schedules a transfer; a running one continues until the new one starts.
    pub(super) fn start(&mut self, page: u8) {
        self.register = page;
        self.start_delay = START_DELAY_M_CYCLES;
    }

    /// Advances one M-cycle and returns the byte to copy, if any.
    pub(super) fn tick(&mut self) -> Option<Copy> {
        if !self.busy && self.start_delay == 0 {
            return None;
        }
        if self.start_delay != 0 {
            self.start_delay -= 1;
            if self.start_delay == 0 {
                let echo = self.register >= FIRST_ECHO_PAGE;
                self.source_page = self.register - if echo { ECHO_PAGE_OFFSET } else { 0 };
                self.next_idx = 0;
            }
        }
        self.busy = self.next_idx < TRANSFER_LEN;
        if !self.busy {
            return None;
        }
        let oam_idx = self.next_idx;
        self.next_idx += 1;
        Some(Copy {
            source: u16::from_be_bytes([self.source_page, oam_idx]),
            oam_idx,
        })
    }

    /// Returns whether OAM is owned by the copier during this M-cycle.
    pub(super) fn is_busy(&self) -> bool {
        self.busy
    }

    pub(super) fn register(&self) -> u8 {
        self.register
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_copies_160_bytes_after_delay() {
        let mut dma = OamDma::new();
        dma.start(0xC1);
        assert_eq!(dma.tick(), None);
        assert!(!dma.is_busy());
        for oam_idx in 0..TRANSFER_LEN {
            let source = 0xC100 + u16::from(oam_idx);
            assert_eq!(dma.tick(), Some(Copy { source, oam_idx }));
            assert!(dma.is_busy());
        }
        assert_eq!(dma.tick(), None);
        assert!(!dma.is_busy());
    }

    #[test]
    fn tick_source_above_echo_is_mirrored() {
        for (page, source) in [(0xDF, 0xDF00), (0xE0, 0xC000), (0xFF, 0xDF00)] {
            let mut dma = OamDma::new();
            dma.start(page);
            dma.tick();
            assert_eq!(dma.tick().map(|copy| copy.source), Some(source));
            assert_eq!(dma.register(), page);
        }
    }

    #[test]
    fn start_while_running_restarts() {
        let mut dma = OamDma::new();
        dma.start(0xC0);
        dma.tick();
        dma.tick();
        dma.start(0xC1);
        assert_eq!(dma.tick().map(|copy| copy.source), Some(0xC001));
        assert_eq!(dma.tick().map(|copy| copy.source), Some(0xC100));
    }
}
