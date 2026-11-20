use super::edid_dimensions;

#[test]
fn edid_requires_valid_checksum_and_both_dimensions() {
    let mut edid = [0u8; 128];
    edid[1..7].fill(255);
    edid[21] = 60;
    edid[22] = 34;
    assert_eq!(edid_dimensions(&edid), None);
    edid[127] = 0u8.wrapping_sub(edid[..127].iter().copied().fold(0, u8::wrapping_add));
    assert_eq!(edid_dimensions(&edid), Some([600, 340]));
    edid[127] = edid[127].wrapping_add(edid[22]);
    edid[22] = 0;
    assert_eq!(edid_dimensions(&edid), None);
}
