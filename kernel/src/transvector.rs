use crate::quad::QuadReg;
use spin::Mutex;

#[derive(Copy, Clone)]
pub struct Transvector {
    pub id: u64,
    pub state: QuadReg,
    pub phys_addr: u64,
}

#[derive(Copy, Clone)]
pub struct Rule {
    pub a: u64,
    pub b: u64,
    pub out: u64,
}

pub const RULES: [Rule; 2] = [Rule { a: 1, b: 2, out: 1 }, Rule { a: 2, b: 1, out: 2 }];

static REGISTRY: Mutex<[Option<Transvector>; 16]> = Mutex::new([None; 16]);

impl Transvector {
    pub fn write(&mut self, new_state: QuadReg) {
        self.state = new_state;
    }
}

pub fn inject(id: u64, raw: u64) -> Transvector {
    let state = match raw {
        0 => QuadReg::N,
        1 => QuadReg::T,
        2 => QuadReg::F,
        3 => QuadReg::S,
        _ => QuadReg::N,
    };
    let phys_addr = 0x1000 + id * 0x100;
    Transvector {
        id,
        state,
        phys_addr,
    }
}

pub fn register(tv: Transvector) -> bool {
    let mut registry = REGISTRY.lock();
    let mut i = 0usize;
    while i < 16 {
        if registry[i].is_none() {
            registry[i] = Some(tv);
            return true;
        }
        i += 1;
    }
    false
}

pub fn write_by_id(id: u64, new_state: QuadReg) -> bool {
    let mut registry = REGISTRY.lock();
    let mut i = 0usize;
    while i < 16 {
        if let Some(tv) = registry[i].as_mut() {
            if tv.id == id {
                tv.write(new_state);
                return true;
            }
        }
        i += 1;
    }
    false
}

pub fn read_by_id(id: u64) -> Option<Transvector> {
    let registry = REGISTRY.lock();
    let mut i = 0usize;
    while i < 16 {
        if let Some(tv) = registry[i] {
            if tv.id == id {
                return Some(tv);
            }
        }
        i += 1;
    }
    None
}

pub fn merge_by_id(a: u64, b: u64) -> Option<QuadReg> {
    let registry = REGISTRY.lock();
    let mut left: Option<QuadReg> = None;
    let mut right: Option<QuadReg> = None;
    let mut i = 0usize;
    while i < 16 {
        if let Some(tv) = registry[i] {
            if tv.id == a {
                left = Some(tv.state);
            }
            if tv.id == b {
                right = Some(tv.state);
            }
        }
        i += 1;
    }

    match (left, right) {
        (Some(x), Some(y)) => Some(x.merge(y)),
        _ => None,
    }
}

pub fn fabric_step() -> u32 {
    let mut applied = 0u32;
    let mut i = 0usize;
    while i < RULES.len() {
        let r = RULES[i];
        if let Some(merged) = merge_by_id(r.a, r.b) {
            if write_by_id(r.out, merged) {
                applied += 1;
            }
        }
        i += 1;
    }
    applied
}
