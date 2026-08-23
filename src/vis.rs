pub(crate) const VIS_BIT_SECONDS: f64 = 0.030;

pub(crate) fn with_even_parity(vis: u8) -> u8 {
    let seven_bits = vis & 0x7f;
    let parity = (seven_bits.count_ones() & 1) as u8;
    seven_bits | (parity << 7)
}

pub(crate) fn has_even_parity(value: u8) -> bool {
    value.count_ones() % 2 == 0
}
