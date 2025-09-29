use aead::KeyInit;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use warp_protocol::codec::Message;
use warp_protocol::*;
use warp_protocol_derive::AeadMessage;

// const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0xF1]
pub struct TunnelPayloadEncrypted {
    #[Aead(encrypted)]
    pub tunnel_id: [u8; 8],
    #[Aead(encrypted)]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, AeadMessage)]
#[message_id = 0xF1]
pub struct TunnelPayloadUnencrypted {
    #[Aead(encrypted)]
    pub tunnel_id: [u8; 8],
    #[Aead(associated_data)]
    pub data: Vec<u8>,
}

pub fn encrypted_vs_unencrypted(c: &mut Criterion) {
    let mut data = [0_u8; 1500];
    rand::fill(&mut data);

    let key = crate::Cipher::generate_key().unwrap();
    let cipher = crate::Cipher::new(&key);

    let mut group = c.benchmark_group("Comparison");

    group.bench_function("Encrypted", |b| {
        b.iter(|| {
            let message = TunnelPayloadEncrypted {
                tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
                data: data.into(),
            };
            message.encode().unwrap().encrypt(&cipher).unwrap()
        })
    });

    group.bench_function("Unencrypted", |b| {
        b.iter(|| {
            let message = TunnelPayloadUnencrypted {
                tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
                data: data.into(),
            };
            criterion::black_box(message.encode().unwrap().encrypt(&cipher).unwrap())
        })
    });

    group.finish();
}

pub fn encryption_time(c: &mut Criterion) {
    const MAX_SIZE: u8 = 16;
    let mut data = [0u8; 2 << MAX_SIZE];
    rand::fill(&mut data);

    let key = crate::Cipher::generate_key().unwrap();
    let cipher = crate::Cipher::new(&key);

    let mut group = c.benchmark_group("Encryption Time");

    group.measurement_time(core::time::Duration::from_secs(1));
    group.warm_up_time(core::time::Duration::from_millis(500));

    for size in 0..MAX_SIZE {
        group.bench_with_input(BenchmarkId::new("bytes", 2 << size), &size, |b, &size| {
            b.iter(|| {
                let message = TunnelPayloadEncrypted {
                    tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
                    data: data[0..(2 << size)].to_vec(),
                };
                criterion::black_box(message.encode().unwrap().encrypt(&cipher).unwrap())
            })
        });
    }

    group.finish();
}

pub fn time_to_discover_incorrect_key(c: &mut Criterion) {
    const MAX_SIZE: u8 = 16;
    let mut data = [0u8; 2 << MAX_SIZE];
    rand::fill(&mut data);

    let key_encryption = crate::Cipher::generate_key().unwrap();
    let cipher_encryption = crate::Cipher::new(&key_encryption);

    let key_decryption = crate::Cipher::generate_key().unwrap();
    let cipher_decryption = crate::Cipher::new(&key_decryption);

    let mut group = c.benchmark_group("Time to discover incorrect key");

    group.measurement_time(core::time::Duration::from_secs(1));
    group.warm_up_time(core::time::Duration::from_millis(500));

    for size in 0..MAX_SIZE {
        let message = TunnelPayloadEncrypted {
            tunnel_id: [1, 2, 3, 4, 5, 6, 7, 8],
            data: data[0..(2 << size)].to_vec(),
        };
        let encrypted_message = message.encode().unwrap().encrypt(&cipher_encryption).unwrap();
        group.bench_with_input(BenchmarkId::new("bytes", 2 << size), &size, |b, &size| {
            b.iter(|| match encrypted_message.clone().decrypt(&cipher_decryption) {
                Ok(_) => panic!("The message shouldn't be decipherable with the wrong key!"),
                Err(e) => criterion::black_box(e),
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    encrypted_vs_unencrypted,
    encryption_time,
    time_to_discover_incorrect_key
);
criterion_main!(benches);
