use super::*;
use crate::cartridge::Cartridge;
use crate::joypad::Button;

const CODE: u16 = 0xC000;
const DATA: u16 = 0xD000;
const IF: u16 = 0xFF0F;
const IE: u16 = 0xFFFF;

/// M-cycles of every opcode with all flags clear (so NZ and NC are taken).
#[rustfmt::skip]
const M_CYCLES: [u32; 256] = [
    1,3,2,2,1,1,2,1, 5,2,2,2,1,1,2,1,
    1,3,2,2,1,1,2,1, 3,2,2,2,1,1,2,1,
    3,3,2,2,1,1,2,1, 2,2,2,2,1,1,2,1,
    3,3,2,2,3,3,3,1, 2,2,2,2,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    2,2,2,2,2,2,1,2, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    1,1,1,1,1,1,2,1, 1,1,1,1,1,1,2,1,
    5,3,4,4,6,4,2,4, 2,4,3,2,3,6,2,4,
    5,3,4,1,6,4,2,4, 2,4,3,1,3,1,2,4,
    3,3,2,1,1,4,2,4, 4,1,4,1,1,1,2,4,
    3,3,2,1,1,4,2,4, 3,2,4,1,1,1,2,4,
];

fn machine(code: &[u8]) -> (Cpu, Motherboard) {
    let cartridge = Cartridge::from_bytes(vec![0; 0x8000], None).expect("blank ROM is valid");
    let mut bus = Motherboard::new(cartridge, 48_000);
    for (addr, &byte) in (CODE..).zip(code) {
        bus.write_byte(addr, byte);
    }
    bus.write_byte(IF, 0);
    let mut cpu = Cpu::new();
    cpu.regs = Registers {
        pc: CODE,
        sp: DATA,
        h: 0xD0,
        ..Registers::default()
    };
    (cpu, bus)
}

#[test]
fn step_every_opcode_takes_documented_cycles() {
    for (op, &expected) in (0..=u8::MAX).zip(&M_CYCLES) {
        let (mut cpu, mut bus) = machine(&[op, 0x00, 0xD8]);
        assert_eq!(cpu.step(&mut bus), expected * 4, "opcode {op:#04x}");
    }
}

#[test]
fn step_every_cb_opcode_takes_documented_cycles() {
    for op in 0..=u8::MAX {
        let (mut cpu, mut bus) = machine(&[0xCB, op]);
        let expected = match (op & 7, op >> 6) {
            (6, 1) => 12,
            (6, _) => 16,
            _ => 8,
        };
        assert_eq!(cpu.step(&mut bus), expected, "CB {op:#04x}");
    }
}

#[test]
fn step_conditional_not_taken_is_shorter() {
    for (op, cycles) in [(0x20, 8), (0xC0, 8), (0xC2, 12), (0xC4, 12)] {
        let (mut cpu, mut bus) = machine(&[op, 0x00, 0xD8]);
        cpu.regs.f = FLAG_Z;
        assert_eq!(cpu.step(&mut bus), cycles, "opcode {op:#04x}");
    }
}

#[test]
fn step_ld_a16_sp_stores_both_bytes() {
    let (mut cpu, mut bus) = machine(&[0x08, 0x00, 0xD1]);
    cpu.regs.sp = 0x1234;
    cpu.step(&mut bus);
    assert_eq!((bus.read_byte(0xD100), bus.read_byte(0xD101)), (0x34, 0x12));
    assert_eq!(cpu.regs.pc, CODE + 3);
}

#[test]
fn step_call_and_ret_round_trip() {
    let (mut cpu, mut bus) = machine(&[0xCD, 0x10, 0xC0]);
    bus.write_byte(0xC010, 0xC9);
    cpu.step(&mut bus);
    assert_eq!((cpu.regs.pc, cpu.regs.sp), (0xC010, DATA - 2));
    cpu.step(&mut bus);
    assert_eq!((cpu.regs.pc, cpu.regs.sp), (CODE + 3, DATA));
}

#[test]
fn step_pop_wraps_stack_pointer() {
    let (mut cpu, mut bus) = machine(&[0xC1]);
    cpu.regs.sp = 0xFFFF;
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.sp, 1);
}

#[test]
fn step_indirect_increments_and_decrements_hl() {
    for (op, expected) in [
        (0x22, DATA + 1),
        (0x2A, DATA + 1),
        (0x32, DATA - 1),
        (0x3A, DATA - 1),
    ] {
        let (mut cpu, mut bus) = machine(&[op]);
        cpu.step(&mut bus);
        assert_eq!(cpu.regs.pair(R16::Hl), expected, "opcode {op:#04x}");
    }
}

#[test]
fn step_jr_moves_backwards() {
    let (mut cpu, mut bus) = machine(&[0x18, 0xFE]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, CODE);
}

#[test]
fn step_rst_jumps_to_vector() {
    let (mut cpu, mut bus) = machine(&[0xEF]);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, 0x28);
}

#[test]
fn step_cb_set_and_res_memory() {
    let (mut cpu, mut bus) = machine(&[0xCB, 0xFE, 0xCB, 0xBE]);
    cpu.step(&mut bus);
    assert_eq!(bus.read_byte(DATA), 0x80);
    cpu.step(&mut bus);
    assert_eq!(bus.read_byte(DATA), 0);
}

#[test]
fn step_interrupt_dispatch_acknowledges_lowest_bit() {
    let (mut cpu, mut bus) = machine(&[]);
    cpu.ime = true;
    bus.write_byte(IE, 0x1F);
    bus.write_byte(IF, 0x0C);
    assert_eq!(cpu.step(&mut bus), 20);
    assert_eq!(cpu.regs.pc, 0x50);
    assert!(!cpu.ime);
    assert_eq!(bus.read_byte(IF) & 0x1F, 0x08);
}

#[test]
fn step_interrupt_cancelled_by_ie_push_jumps_to_zero() {
    let (mut cpu, mut bus) = machine(&[]);
    cpu.ime = true;
    cpu.regs.pc = 0x00FF;
    cpu.regs.sp = 0x0000;
    bus.write_byte(IE, 0x04);
    bus.write_byte(IF, 0x04);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, 0);
    assert_eq!(bus.read_byte(IF) & 0x1F, 0x04, "nothing acknowledged");
}

#[test]
fn step_ei_enables_after_next_instruction() {
    let (mut cpu, mut bus) = machine(&[0xFB, 0x00, 0x00]);
    bus.write_byte(IE, 4);
    bus.write_byte(IF, 4);
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, CODE + 2);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, 0x50);
}

#[test]
fn step_di_cancels_pending_ei() {
    let (mut cpu, mut bus) = machine(&[0xFB, 0xF3, 0x00]);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert!(!cpu.ime);
}

#[test]
fn step_halt_wakes_on_pending_interrupt() {
    let (mut cpu, mut bus) = machine(&[0x76, 0x04]);
    cpu.step(&mut bus);
    assert_eq!(cpu.step(&mut bus), 4);
    assert_eq!(cpu.regs.b, 0);
    bus.write_byte(IE, 4);
    bus.write_byte(IF, 4);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.b, 1);
}

#[test]
fn step_halt_bug_executes_next_byte_twice() {
    let (mut cpu, mut bus) = machine(&[0x76, 0x04, 0x00]);
    bus.write_byte(IE, 4);
    bus.write_byte(IF, 4);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert_eq!((cpu.regs.b, cpu.regs.pc), (2, CODE + 2));
}

#[test]
fn step_stop_freezes_until_button() {
    let (mut cpu, mut bus) = machine(&[0x10, 0x00, 0x04]);
    cpu.step(&mut bus);
    assert_eq!(bus.read_byte(0xFF04), 0);
    for _ in 0..1000 {
        assert_eq!(cpu.step(&mut bus), 4);
    }
    assert_eq!((cpu.regs.b, bus.read_byte(0xFF04)), (0, 0));
    bus.joypad.set_button(Button::A, true);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.b, 1);
}
