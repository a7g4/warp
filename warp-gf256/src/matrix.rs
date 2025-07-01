use super::{Additive, GF256, Multiplicative};
use std::ops::{Add, AddAssign, Index, IndexMut, Mul, Sub, SubAssign};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matrix<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16 = { super::DEFAULT_POLYNOMIAL }>(
    [[super::GF256<PRIMITIVE_POLYNOMIAL>; COLS]; ROWS],
);

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL> {
    pub fn new(data: [[u8; COLS]; ROWS]) -> Self {
        Self(data.map(|row| row.map(super::GF256::<PRIMITIVE_POLYNOMIAL>)))
    }

    pub fn transpose(&self) -> Matrix<COLS, ROWS, PRIMITIVE_POLYNOMIAL> {
        let mut data = [[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); ROWS]; COLS];

        for i in 0..ROWS {
            for j in 0..COLS {
                data[j][i] = self.0[i][j];
            }
        }
        Matrix::<COLS, ROWS, PRIMITIVE_POLYNOMIAL>(data)
    }
}

pub fn scalar_product<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: super::GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    #[cfg(target_feature = "neon")]
    return scalar_product_neon(scalar, vector);
    #[allow(unreachable_code)]
    scalar_product_fallback(scalar, vector)
}

pub fn scalar_product_fallback<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: super::GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    let mul_lookup_table = GF256::<PRIMITIVE_POLYNOMIAL>::MUL_TABLE[scalar.0 as usize];
    vector.map(|x| super::GF256(mul_lookup_table[x.0 as usize]))
}

#[cfg(target_feature = "neon")]
pub fn scalar_product_neon<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: super::GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    let mut product = [<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); SIZE];
    let mul_lookup_table = GF256::<PRIMITIVE_POLYNOMIAL>::MUL_TABLE[scalar.0 as usize];
    unsafe {
        // Stolen from: https://users.rust-lang.org/t/ensure-that-struct-t-has-size-n-at-compile-time/61108/4
        // Compile time check that GF256 is the same size as u8 so that ptr cast below is valid
        const _: () = [(); 1][(core::mem::size_of::<super::GF256<0>>() == core::mem::size_of::<u8>()) as usize ^ 1];

        let mut i = 0;
        while i + 16 < SIZE {
            let simd_slice_chunk = std::arch::aarch64::vld1q_u8(vector[i..].as_ptr() as *mut u8);
            let simd_slice_chunk_low = std::arch::aarch64::vget_low_u8(simd_slice_chunk);
            let simd_slice_chunk_high = std::arch::aarch64::vget_high_u8(simd_slice_chunk);
            let low_result = std::arch::aarch64::vqtbl1_u8(
                std::arch::aarch64::vld1q_u8(mul_lookup_table.as_ptr()),
                simd_slice_chunk_low,
            );
            let high_result = std::arch::aarch64::vqtbl1_u8(
                std::arch::aarch64::vld1q_u8(mul_lookup_table.as_ptr()),
                simd_slice_chunk_high,
            );
            let result = std::arch::aarch64::vcombine_u8(low_result, high_result);
            std::arch::aarch64::vst1q_u8(product[i..].as_mut_ptr() as *mut u8, result);
            i += 16;
        }

        for j in i..SIZE {
            product[j] = super::GF256(mul_lookup_table[vector[j].0 as usize]);
        }
    }
    product
}

#[test]
fn test_scalar_product() {
    use super::GF256;
    const SCALAR: GF256 = GF256(7);
    let a: [super::GF256; 20] = [
        GF256(0),
        GF256(1),
        GF256(2),
        GF256(3),
        GF256(4),
        GF256(5),
        GF256(6),
        GF256(7),
        GF256(8),
        GF256(9),
        GF256(10),
        GF256(11),
        GF256(12),
        GF256(13),
        GF256(14),
        GF256(15),
        GF256(16),
        GF256(17),
        GF256(18),
        GF256(19),
    ];

    let b = scalar_product(SCALAR, &a);

    for (i, val) in a.iter().enumerate() {
        assert_eq!(*val * SCALAR, b[i]);
    }
}

fn inner_product<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    a: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
    b: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> super::GF256<PRIMITIVE_POLYNOMIAL> {
    #[allow(unreachable_code)]
    inner_product_fallback(a, b)
}

fn inner_product_fallback<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    a: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
    b: &[super::GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> super::GF256<PRIMITIVE_POLYNOMIAL> {
    a.iter().zip(b.iter()).map(|(x, y)| (*x) * (*y)).sum()
}

#[test]
fn test_inner_product() {
    use super::GF256;
    const SCALAR: GF256 = GF256(7);
    let a: [super::GF256; 4] = [GF256(0), GF256(1), GF256(2), GF256(3)];

    let b = inner_product(&a, &a);
    assert_eq!(
        b,
        GF256(0) * GF256(0) + GF256(1) * GF256(1) + GF256(2) * GF256(2) + GF256(3) * GF256(3)
    );
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> Index<(usize, usize)>
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    type Output = GF256<PRIMITIVE_POLYNOMIAL>;

    #[inline]
    fn index(&self, index: (usize, usize)) -> &Self::Output {
        &self.0[index.0][index.1]
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> IndexMut<(usize, usize)>
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    #[inline]
    fn index_mut(&mut self, index: (usize, usize)) -> &mut GF256<PRIMITIVE_POLYNOMIAL> {
        &mut self.0[index.0][index.1]
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> Additive
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    fn identity() -> Self {
        Self([[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); COLS]; ROWS])
    }

    fn inverse(&self) -> Self {
        self.clone()
    }
}

impl<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16> Multiplicative for Matrix<SIZE, SIZE, PRIMITIVE_POLYNOMIAL> {
    fn identity() -> Self {
        let mut data = [[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); SIZE]; SIZE];
        for i in 0..SIZE {
            data[i][i] = <super::GF256<PRIMITIVE_POLYNOMIAL> as Multiplicative>::identity();
        }
        Self(data)
    }

    fn inverse(&self) -> Result<Self, crate::Error>
    where
        Self: Sized,
    {
        todo!()
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> Add
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    type Output = Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>;

    fn add(self, rhs: Self) -> Self::Output {
        let mut data = [[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); COLS]; ROWS];

        for row in 0..ROWS {
            for col in 0..COLS {
                data[row][col] = self.0[row][col] + rhs.0[row][col];
            }
        }
        Matrix(data)
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> AddAssign
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    fn add_assign(&mut self, rhs: Self) {
        for row in 0..ROWS {
            for col in 0..COLS {
                self.0[row][col] = self.0[row][col] + rhs.0[row][col];
            }
        }
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> Sub
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    type Output = [[super::GF256<PRIMITIVE_POLYNOMIAL>; COLS]; ROWS];

    fn sub(self, rhs: Self) -> Self::Output {
        let mut result = [[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); COLS]; ROWS];

        for row in 0..ROWS {
            for col in 0..COLS {
                result[row][col] = self.0[row][col] - rhs.0[row][col];
            }
        }
        result
    }
}

impl<const ROWS: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16> SubAssign
    for Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>
{
    fn sub_assign(&mut self, rhs: Self) {
        for row in 0..ROWS {
            for col in 0..COLS {
                self.0[row][col] = self.0[row][col] - rhs.0[row][col];
            }
        }
    }
}

impl<const ROWS: usize, const INNER: usize, const COLS: usize, const PRIMITIVE_POLYNOMIAL: u16>
    Mul<Matrix<INNER, COLS, PRIMITIVE_POLYNOMIAL>> for Matrix<ROWS, INNER, PRIMITIVE_POLYNOMIAL>
{
    type Output = Matrix<ROWS, COLS, PRIMITIVE_POLYNOMIAL>;

    fn mul(self, rhs: Matrix<INNER, COLS, PRIMITIVE_POLYNOMIAL>) -> Self::Output {
        let mut data = [[<super::GF256<PRIMITIVE_POLYNOMIAL> as Additive>::identity(); COLS]; ROWS];

        let rhs_t = rhs.transpose();

        for i in 0..ROWS {
            for j in 0..COLS {
                data[i][j] = inner_product(&self.0[i], &rhs_t.0[j]);
            }
        }

        Matrix::<ROWS, COLS, PRIMITIVE_POLYNOMIAL>(data)
    }
}

#[test]
fn test_add() {
    let a = <Matrix<5, 5> as Multiplicative>::identity();
    let a_x2 = a.clone() + a.clone();
    assert_eq!(a_x2[(0, 0)], GF256(0));
}

#[test]
fn test_mul() {
    let a = Matrix::<2, 3>::new([[1, 2, 3], [4, 5, 6]]);
    let a_x2 = a.clone() + a.clone();
    assert_eq!(a_x2[(0, 0)], GF256(0));
}
