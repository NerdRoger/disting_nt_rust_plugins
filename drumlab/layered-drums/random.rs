pub(crate) fn random_value(seed: &mut u32, min: i16, max: i16) -> i16 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    let width = (i32::from(max) - i32::from(min) + 1) as u32;
    let offset = ((u64::from(*seed) * u64::from(width)) >> 32) as i32;
    (i32::from(min) + offset) as i16
}
