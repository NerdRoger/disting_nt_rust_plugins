pub(crate) fn next_bits(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

pub(crate) fn signed_noise(seed: &mut u32) -> f32 {
    next_bits(seed) as f32 / 2_147_483_648.0 - 1.0
}

pub(crate) fn random_value(seed: &mut u32, min: i16, max: i16) -> i16 {
    let bits = next_bits(seed);
    let width = (i32::from(max) - i32::from(min) + 1) as u32;
    let offset = ((u64::from(bits) * u64::from(width)) >> 32) as i32;
    (i32::from(min) + offset) as i16
}
