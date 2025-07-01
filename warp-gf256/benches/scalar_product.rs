use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

pub fn scalar_product(c: &mut Criterion) {
    use warp_gf256::GF256;
    const SCALAR: GF256 = GF256(7);
    let a: [warp_gf256::GF256; 20] = [
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

    let mut group = c.benchmark_group("scalar_product");

    let input: [u8; 8] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 8] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 8), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 8), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 16] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 16] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 16), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 16), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 32] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 32] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 32), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 32), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 64] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 64] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 64), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 64), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 128] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 128] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 128), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 128), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 256] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 256] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 256), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 256), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 512] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 512] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 512), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 512), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 1024] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 1024] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 1024), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 1024), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 2048] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 2048] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 2048), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 2048), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });

    let input: [u8; 4096] = std::array::from_fn(|i| i as u8);
    let input: [GF256; 4096] = input.map(GF256);
    group.bench_with_input(BenchmarkId::new("scalar_product_fallback", 4096), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_fallback(SCALAR, i))
    });
    #[cfg(target_feature = "neon")]
    group.bench_with_input(BenchmarkId::new("scalar_product_neon", 4096), &input, |b, i| {
        b.iter(|| warp_gf256::simd::scalar_product_neon(SCALAR, i))
    });
}

criterion_group!(benches, scalar_product);
criterion_main!(benches);
