use super::GF256;

pub fn scalar_product<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    // TODO: Benchmarks show this to be slower than the fallback! Make SIMD faster?
    // #[cfg(target_feature = "neon")]
    // return scalar_product_neon(scalar, vector);
    #[allow(unreachable_code)]
    scalar_product_fallback(scalar, vector)
}

pub fn scalar_product_fallback<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    let mul_lookup_table = GF256::<PRIMITIVE_POLYNOMIAL>::MUL_TABLE[scalar.0 as usize];
    vector.map(|x| GF256(mul_lookup_table[x.0 as usize]))
}

#[cfg(target_feature = "neon")]
pub fn scalar_product_neon<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    scalar: GF256<PRIMITIVE_POLYNOMIAL>,
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> [GF256<PRIMITIVE_POLYNOMIAL>; SIZE] {
    let mul_table_row = &GF256::<PRIMITIVE_POLYNOMIAL>::MUL_TABLE[scalar.0 as usize];
    let mut result = [GF256(0); SIZE];

    let mut i = 0;
    unsafe {
        let mul_table_row_ptr = mul_table_row.as_ptr();

        while i + 16 <= SIZE {
            use std::arch::aarch64::*;

            // Load input vector (16 bytes)
            let input = vld1q_u8(vector.as_ptr().add(i).cast::<u8>());

            // Split into low/high nibbles (4-bit halves)
            let lo_nibble = vandq_u8(input, vdupq_n_u8(0x0F)); // Lower 4 bits (0-15)
            let hi_nibble = vshrq_n_u8(input, 4); // Upper 4 bits (0-15)

            // Load 16x 16-byte chunks of the multiplication table
            let tables = [
                vld1q_u8(mul_table_row_ptr.add(0)),
                vld1q_u8(mul_table_row_ptr.add(16)),
                vld1q_u8(mul_table_row_ptr.add(32)),
                vld1q_u8(mul_table_row_ptr.add(48)),
                vld1q_u8(mul_table_row_ptr.add(64)),
                vld1q_u8(mul_table_row_ptr.add(80)),
                vld1q_u8(mul_table_row_ptr.add(96)),
                vld1q_u8(mul_table_row_ptr.add(112)),
                vld1q_u8(mul_table_row_ptr.add(128)),
                vld1q_u8(mul_table_row_ptr.add(144)),
                vld1q_u8(mul_table_row_ptr.add(160)),
                vld1q_u8(mul_table_row_ptr.add(176)),
                vld1q_u8(mul_table_row_ptr.add(192)),
                vld1q_u8(mul_table_row_ptr.add(208)),
                vld1q_u8(mul_table_row_ptr.add(224)),
                vld1q_u8(mul_table_row_ptr.add(240)),
            ];

            // Lookup results for each nibble in each table segment
            let mut res = vdupq_n_u8(0);
            for (table_idx, &table) in tables.iter().enumerate() {
                let mask = vceqq_u8(hi_nibble, vdupq_n_u8(table_idx as u8));
                let lookup = vqtbl1q_u8(table, lo_nibble);
                res = vbslq_u8(mask, lookup, res);
            }

            vst1q_u8(result.as_mut_ptr().add(i).cast::<u8>(), res);
            i += 16;
        }
    }

    // Handle remaining elements
    for j in i..SIZE {
        result[j] = GF256(mul_table_row[vector[j].0 as usize]);
    }

    result
}

#[cfg(target_feature = "neon")]
#[test]
fn test_scalar_product_neon() {
    let scalar = GF256(77);
    let input: [u8; 300] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 300] = input.map(GF256);
    assert_eq!(
        scalar_product_neon(scalar, &input),
        scalar_product_fallback(scalar, &input)
    )
}

pub fn sum<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> GF256<PRIMITIVE_POLYNOMIAL> {
    #[cfg(target_feature = "neon")]
    return sum_neon(vector);
    #[allow(unreachable_code)]
    sum_fallback(vector)
}

#[cfg(target_feature = "neon")]
pub fn sum_neon<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> GF256<PRIMITIVE_POLYNOMIAL> {
    GF256::<PRIMITIVE_POLYNOMIAL>(unsafe {
        // Stolen from: https://users.rust-lang.org/t/ensure-that-struct-t-has-size-n-at-compile-time/61108/4
        // Compile time check that GF256 is the same size as u8 so that ptr cast below is valid
        const _: () = [(); 1][(core::mem::size_of::<GF256<0>>() == core::mem::size_of::<u8>()) as usize ^ 1];

        // Initialize result vector with zeros
        let mut result = std::arch::aarch64::vdupq_n_u8(0);

        let mut i = 0;
        // Process 16 bytes at a time
        while i + 16 <= SIZE {
            // Load 16 bytes from the array
            let chunk = std::arch::aarch64::vld1q_u8(vector[i..].as_ptr() as *mut u8);
            // XOR with the result
            result = std::arch::aarch64::veorq_u8(result, chunk);
            i += 16;
        }

        // Horizontal XOR of the 16 bytes in the result vector
        let temp = std::arch::aarch64::veor_u8(
            std::arch::aarch64::vget_low_u8(result),
            std::arch::aarch64::vget_high_u8(result),
        );

        let temp2 = std::arch::aarch64::vreinterpret_u32_u8(temp);
        let temp3 = std::arch::aarch64::vdup_lane_u32(temp2, 0);
        let temp4 = std::arch::aarch64::vdup_lane_u32(temp2, 1);
        let temp5 = std::arch::aarch64::veor_u32(temp3, temp4);
        let temp6 = std::arch::aarch64::vreinterpret_u8_u32(temp5);

        // Further reduce from 4 bytes to 2 bytes
        let temp7 = std::arch::aarch64::vget_lane_u32(std::arch::aarch64::vreinterpret_u32_u8(temp6), 0);
        let xor_value = (temp7 & 0xFF) ^ ((temp7 >> 8) & 0xFF) ^ ((temp7 >> 16) & 0xFF) ^ ((temp7 >> 24) & 0xFF);
        let mut result_u8 = xor_value as u8;

        // Process remaining bytes
        while i < SIZE {
            result_u8 ^= vector[i].0;
            i += 1;
        }
        result_u8
    })
}

#[cfg(target_feature = "neon")]
#[test]
fn test_sum_neon() {
    let input: [u8; 200] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 200] = input.map(GF256);
    assert_eq!(sum_neon(&input), sum_fallback(&input))
}

pub fn sum_fallback<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    vector: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> GF256<PRIMITIVE_POLYNOMIAL> {
    GF256::<PRIMITIVE_POLYNOMIAL>(vector.iter().fold(0, |acc, &x| acc ^ x.0))
}

fn inner_product<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    a: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
    b: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> GF256<PRIMITIVE_POLYNOMIAL> {
    #[allow(unreachable_code)]
    inner_product_fallback(a, b)
}

fn inner_product_fallback<const SIZE: usize, const PRIMITIVE_POLYNOMIAL: u16>(
    a: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
    b: &[GF256<PRIMITIVE_POLYNOMIAL>; SIZE],
) -> GF256<PRIMITIVE_POLYNOMIAL> {
    a.iter().zip(b.iter()).map(|(x, y)| (*x) * (*y)).sum()
}
