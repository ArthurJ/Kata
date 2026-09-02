use super::*;

#[test]
fn byte_and_scalar() {
    assert_eq!(
        untag_smi(kata_rt_byte_and(tag_smi(0xF0), tag_smi(0x3C))),
        0x30
    );
}

#[test]
fn byte_or_scalar() {
    assert_eq!(
        untag_smi(kata_rt_byte_or(tag_smi(0xF0), tag_smi(0x0F))),
        0xFF
    );
}

#[test]
fn byte_xor_scalar() {
    assert_eq!(
        untag_smi(kata_rt_byte_xor(tag_smi(0xFF), tag_smi(0x0F))),
        0xF0
    );
}

#[test]
fn byte_not_scalar() {
    assert_eq!(untag_smi(kata_rt_byte_not(tag_smi(0xF0))), 0x0F);
}

#[test]
fn byte_shr() {
    assert_eq!(untag_smi(kata_rt_byte_shr(tag_smi(0xF0), tag_smi(4))), 0x0F);
}

#[test]
fn byte_shl() {
    assert_eq!(untag_smi(kata_rt_byte_shl(tag_smi(0x0F), tag_smi(4))), 0xF0);
}

#[test]
fn byte_to_int() {
    assert_eq!(untag_smi(kata_rt_byte_to_int(tag_smi(0x48))), 0x48);
}

#[test]
fn int_to_byte() {
    assert_eq!(untag_smi(kata_rt_int_to_byte(tag_smi(300))), 44); // 300 mod 256 = 44
}
