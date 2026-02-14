#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct QuadReg(u8);

impl QuadReg {
    #[allow(dead_code)]
    pub const N: Self = Self(0);
    #[allow(dead_code)]
    pub const F: Self = Self(1);
    #[allow(dead_code)]
    pub const T: Self = Self(2);
    #[allow(dead_code)]
    pub const S: Self = Self(3);

    #[allow(dead_code)]
    pub fn from_bits(bits: u8) -> Self {
        Self(bits & 0b11)
    }

    #[allow(dead_code)]
    pub fn bits(self) -> u8 {
        self.0 & 0b11
    }

    #[allow(dead_code)]
    pub fn merge(self, other: Self) -> Self {
        if self.bits() == other.bits() {
            self
        } else {
            Self::S
        }
    }
}
