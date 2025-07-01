mod lut;
//pub mod matrix;
pub mod matrix;
pub mod simd;
use std::ops::{Add, AddAssign, Mul, MulAssign, Sub, SubAssign};

pub const DEFAULT_POLYNOMIAL: u16 = 0x11D;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GF256<const PRIMITIVE_POLYNOMIAL: u16 = DEFAULT_POLYNOMIAL>(pub u8);

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("division by zero")]
    DivideByZero,
}

pub trait Additive {
    fn identity() -> Self;
    fn inverse(&self) -> Self;
}

pub trait Multiplicative {
    fn identity() -> Self;
    fn inverse(&self) -> Result<Self, Error>
    where
        Self: Sized;
}

impl<const PRIMITIVE_POLYNOMIAL: u16> GF256<PRIMITIVE_POLYNOMIAL> {
    pub(crate) const LOG_TABLE: [u8; 256] = lut::generate_log_table(PRIMITIVE_POLYNOMIAL);
    pub(crate) const EXP_TABLE: [u8; 256] = lut::generate_exp_table(PRIMITIVE_POLYNOMIAL);
    pub(crate) const MUL_TABLE: [[u8; 256]; 256] = lut::generate_mul_table(PRIMITIVE_POLYNOMIAL);
}

impl<const PRIMITIVE_POLYNOMIAL: u16> Additive for GF256<PRIMITIVE_POLYNOMIAL> {
    fn identity() -> Self {
        GF256(0)
    }

    #[inline]
    fn inverse(&self) -> Self {
        *self
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> Multiplicative for GF256<PRIMITIVE_POLYNOMIAL> {
    fn identity() -> Self {
        GF256(1)
    }

    #[inline]
    fn inverse(&self) -> Result<Self, Error>
    where
        Self: Sized,
    {
        if self.0 == 0 {
            return Err(Error::DivideByZero);
        }

        // Use the property that a^254 = a^(-1) in GF(256)
        let log_val = Self::LOG_TABLE[self.0 as usize];
        let inv_log = 255u8.wrapping_sub(log_val);
        Ok(GF256(Self::EXP_TABLE[inv_log as usize]))
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> Add for GF256<PRIMITIVE_POLYNOMIAL> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        GF256(self.0 ^ rhs.0)
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> AddAssign for GF256<PRIMITIVE_POLYNOMIAL> {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> Sub for GF256<PRIMITIVE_POLYNOMIAL> {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        // Subtraction in GF(256) is the same as addition (XOR)
        GF256(self.0 ^ rhs.0)
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> SubAssign for GF256<PRIMITIVE_POLYNOMIAL> {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        // Subtraction in GF(256) is the same as addition (XOR)
        self.0 ^= rhs.0;
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> Mul for GF256<PRIMITIVE_POLYNOMIAL> {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        GF256(Self::MUL_TABLE[self.0 as usize][rhs.0 as usize])
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> MulAssign for GF256<PRIMITIVE_POLYNOMIAL> {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl<const PRIMITIVE_POLYNOMIAL: u16> std::iter::Sum for GF256<PRIMITIVE_POLYNOMIAL> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(<Self as Additive>::identity(), |acc, x| acc + x)
    }
}

#[test]
fn test_add() {
    let zero = <GF256<{ DEFAULT_POLYNOMIAL }> as Additive>::identity();
    let one = <GF256<{ DEFAULT_POLYNOMIAL }> as Multiplicative>::identity();
    assert_eq!(zero, zero + zero);
    assert_eq!(one, one + zero);
    assert_eq!(zero, one + one);
}

#[test]
fn test_mul_inv() {
    let zero = <GF256<{ DEFAULT_POLYNOMIAL }> as Additive>::identity();
    let one = <GF256<{ DEFAULT_POLYNOMIAL }> as Multiplicative>::identity();
    assert_eq!(zero, zero * zero);
    assert_eq!(zero, one * zero);
    assert_eq!(one, one * one);

    for i in 1..=255 {
        let i = GF256::<DEFAULT_POLYNOMIAL>(i);
        let inv = Multiplicative::inverse(&i).unwrap();
        assert_eq!(one, i * inv);
        assert_eq!(i, (i * i) * inv);
    }
}
