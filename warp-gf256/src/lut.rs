pub const fn generate_exp_table(primitive_polynomial: u16) -> [u8; 256] {
    let mut exp = [0u8; 256];
    let mut x: u16 = 1;

    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= primitive_polynomial;
        }
        i += 1;
    }

    // Make exp[255] = exp[0] for easier modular arithmetic
    exp[255] = exp[0];
    exp
}

pub const fn generate_log_table(primitive_polynomial: u16) -> [u8; 256] {
    let exp = generate_exp_table(primitive_polynomial);
    let mut log = [0u8; 256];

    // log[0] is undefined, leave as 0

    let mut i = 0;
    while i < 255 {
        log[exp[i] as usize] = i as u8;
        i += 1;
    }

    log
}

pub const fn generate_mul_table(primitive_polynomial: u16) -> [[u8; 256]; 256] {
    let exp = generate_exp_table(primitive_polynomial);
    let log = generate_log_table(primitive_polynomial);
    let mut mul = [[0u8; 256]; 256];

    let mut a = 0;
    while a < 256 {
        let mut b = 0;
        while b < 256 {
            if a == 0 || b == 0 {
                mul[a][b] = 0;
            } else {
                let log_a = log[a] as u16;
                let log_b = log[b] as u16;
                let log_result = (log_a + log_b) % 255;
                mul[a][b] = exp[log_result as usize];
            }
            b += 1;
        }
        a += 1;
    }

    mul
}
