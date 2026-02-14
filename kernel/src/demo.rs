use crate::framebuffer::FramebufferWriter;
use crate::log;
use crate::quad::QuadReg;
use crate::transjector::{
    bus_pending, bus_push, bus_step_gated, trace_last, transject, StepResult, TransjectorEvent,
};
use crate::transvector::{fabric_step, inject, merge_by_id, read_by_id, register, write_by_id};

pub fn run(fb: &mut FramebufferWriter) {
    let tv = inject(1, 1);
    let _ = register(tv);

    let mut line3 = [0u8; 32];
    let mut n3 = 0usize;
    n3 += append_bytes(&mut line3[n3..], b"Transvector #");
    n3 += append_u64(&mut line3[n3..], tv.id);
    if let Ok(s) = core::str::from_utf8(&line3[..n3]) {
        fb.write_line(s);
    }

    let state_text = match tv.state.bits() {
        0 => "N",
        1 => "F",
        2 => "T",
        3 => "S",
        _ => "N",
    };

    let mut line4 = [0u8; 32];
    let mut n4 = 0usize;
    n4 += append_bytes(&mut line4[n4..], b"State: ");
    n4 += append_bytes(&mut line4[n4..], state_text.as_bytes());
    if let Ok(s) = core::str::from_utf8(&line4[..n4]) {
        fb.write_line(s);
    }

    let mut line5 = [0u8; 32];
    let mut n5 = 0usize;
    n5 += append_bytes(&mut line5[n5..], b"Phys: 0x");
    n5 += append_hex_u64(&mut line5[n5..], tv.phys_addr);
    if let Ok(s) = core::str::from_utf8(&line5[..n5]) {
        fb.write_line(s);
    }

    let updated = write_by_id(1, QuadReg::S);
    if updated {
        fb.write_line("Updated state for ID 1:");
        fb.write_line("State: S");
    }

    let tv2 = inject(2, 2);
    let _ = register(tv2);

    if let Some(tv2_read) = read_by_id(2) {
        let mut line6 = [0u8; 32];
        let mut n6 = 0usize;
        n6 += append_bytes(&mut line6[n6..], b"Transvector #");
        n6 += append_u64(&mut line6[n6..], tv2_read.id);
        if let Ok(s) = core::str::from_utf8(&line6[..n6]) {
            fb.write_line(s);
        }

        let mut line7 = [0u8; 32];
        let mut n7 = 0usize;
        n7 += append_bytes(&mut line7[n7..], b"State: ");
        n7 += append_bytes(&mut line7[n7..], state_name(tv2_read.state).as_bytes());
        if let Ok(s) = core::str::from_utf8(&line7[..n7]) {
            fb.write_line(s);
        }
    }

    let _ = write_by_id(1, QuadReg::T);

    let t9 = transject(9, 0, 3);
    let t10 = transject(10, 0, 0);
    let t11 = transject(11, 0, 1);
    let _ = t9.id;
    let _ = t9.lane;
    let _ = t10.id;
    let _ = t10.lane;
    let _ = t11.id;
    let _ = t11.lane;

    let ev1 = TransjectorEvent {
        out_id: 1,
        t: t9,
    };
    let ev2 = TransjectorEvent {
        out_id: 1,
        t: t10,
    };
    let ev3 = TransjectorEvent {
        out_id: 1,
        t: t11,
    };
    let _ = bus_push(0, ev1);
    let _ = bus_push(0, ev2);
    let _ = bus_push(0, ev3);
    log::serial_only("K05 bus push done");

    let mut line_bus = [0u8; 32];
    let mut nb = 0usize;
    nb += append_bytes(&mut line_bus[nb..], b"Bus pending: ");
    nb += append_u64(&mut line_bus[nb..], bus_pending() as u64);
    if let Ok(s) = core::str::from_utf8(&line_bus[..nb]) {
        fb.write_line(s);
    }

    let mut i = 0usize;
    while i < 3 {
        match bus_step_gated() {
            StepResult::Applied => fb.write_line("Bus step: applied"),
            StepResult::Blocked => fb.write_line("Bus step: blocked"),
            StepResult::Empty => fb.write_line("Bus step: empty"),
        }
        let tr = trace_last();
        let mut lt = [0u8; 96];
        let mut nt = 0usize;
        nt += append_bytes(&mut lt[nt..], b"Trace t=");
        nt += append_u64(&mut lt[nt..], tr.tick as u64);
        nt += append_bytes(&mut lt[nt..], b" l=");
        nt += append_u64(&mut lt[nt..], tr.lane as u64);
        nt += append_bytes(&mut lt[nt..], b" out=");
        nt += append_u64(&mut lt[nt..], tr.out_id);
        nt += append_bytes(&mut lt[nt..], b" in=");
        nt += append_u64(&mut lt[nt..], tr.in_id);
        nt += append_bytes(&mut lt[nt..], b" ");
        nt += append_bytes(&mut lt[nt..], state_name_from_bits(tr.cur).as_bytes());
        nt += append_bytes(&mut lt[nt..], state_name_from_bits(tr.incoming).as_bytes());
        nt += append_bytes(&mut lt[nt..], b" -> ");
        nt += append_bytes(
            &mut lt[nt..],
            match tr.result {
                0 => b"APPLY",
                1 => b"BLOCK",
                _ => b"EMPTY",
            },
        );
        if let Ok(s) = core::str::from_utf8(&lt[..nt]) {
            fb.write_line(s);
        }
        if let Some(tv1_now) = read_by_id(1) {
            let mut line_t2 = [0u8; 32];
            let mut nt2 = 0usize;
            nt2 += append_bytes(&mut line_t2[nt2..], b"State #1: ");
            nt2 += append_bytes(&mut line_t2[nt2..], state_name(tv1_now.state).as_bytes());
            if let Ok(s) = core::str::from_utf8(&line_t2[..nt2]) {
                fb.write_line(s);
            }
        }
        i += 1;
    }
    log::serial_only("K06 bus steps done");

    if let Some(m) = merge_by_id(1, 2) {
        fb.write_line("Merge(1,2):");
        let mut line8 = [0u8; 32];
        let mut n8 = 0usize;
        n8 += append_bytes(&mut line8[n8..], b"State: ");
        n8 += append_bytes(&mut line8[n8..], state_name(m).as_bytes());
        if let Ok(s) = core::str::from_utf8(&line8[..n8]) {
            fb.write_line(s);
        }
    }

    let n = fabric_step();
    let mut line9 = [0u8; 32];
    let mut n9 = 0usize;
    n9 += append_bytes(&mut line9[n9..], b"Fabric applied: ");
    n9 += append_u64(&mut line9[n9..], n as u64);
    if let Ok(s) = core::str::from_utf8(&line9[..n9]) {
        fb.write_line(s);
    }

    if let Some(tv1_now) = read_by_id(1) {
        let mut line10 = [0u8; 32];
        let mut n10 = 0usize;
        n10 += append_bytes(&mut line10[n10..], b"State #1: ");
        n10 += append_bytes(&mut line10[n10..], state_name(tv1_now.state).as_bytes());
        if let Ok(s) = core::str::from_utf8(&line10[..n10]) {
            fb.write_line(s);
        }
    }

    if let Some(tv2_now) = read_by_id(2) {
        let mut line11 = [0u8; 32];
        let mut n11 = 0usize;
        n11 += append_bytes(&mut line11[n11..], b"State #2: ");
        n11 += append_bytes(&mut line11[n11..], state_name(tv2_now.state).as_bytes());
        if let Ok(s) = core::str::from_utf8(&line11[..n11]) {
            fb.write_line(s);
        }
    }
    log::serial_only("K07 fabric done");
}

fn state_name(s: QuadReg) -> &'static str {
    match s.bits() {
        0 => "N",
        1 => "F",
        2 => "T",
        3 => "S",
        _ => "N",
    }
}

fn state_name_from_bits(bits: u8) -> &'static str {
    match bits & 0b11 {
        0 => "N",
        1 => "F",
        2 => "T",
        _ => "S",
    }
}

fn append_bytes(dst: &mut [u8], src: &[u8]) -> usize {
    let count = core::cmp::min(dst.len(), src.len());
    dst[..count].copy_from_slice(&src[..count]);
    count
}

fn append_u64(dst: &mut [u8], mut value: u64) -> usize {
    if dst.is_empty() {
        return 0;
    }

    if value == 0 {
        dst[0] = b'0';
        return 1;
    }

    let mut rev = [0u8; 20];
    let mut n = 0usize;
    while value > 0 && n < rev.len() {
        rev[n] = b'0' + (value % 10) as u8;
        value /= 10;
        n += 1;
    }

    let out = core::cmp::min(n, dst.len());
    for i in 0..out {
        dst[i] = rev[n - 1 - i];
    }
    out
}

fn append_hex_u64(dst: &mut [u8], mut value: u64) -> usize {
    if dst.is_empty() {
        return 0;
    }

    if value == 0 {
        dst[0] = b'0';
        return 1;
    }

    let mut rev = [0u8; 16];
    let mut n = 0usize;
    while value > 0 && n < rev.len() {
        let d = (value & 0xF) as u8;
        rev[n] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        value >>= 4;
        n += 1;
    }

    let out = core::cmp::min(n, dst.len());
    for i in 0..out {
        dst[i] = rev[n - 1 - i];
    }
    out
}
