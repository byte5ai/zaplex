use super::*;

#[test]
fn darker_lighter_test() {
    assert_eq!(
        darken(ColorU::new(255, 128, 0, OPAQUE)),
        ColorU::new(123, 62, 0, OPAQUE)
    );
    assert_eq!(
        lighten(ColorU::new(255, 128, 0, OPAQUE)),
        ColorU::new(255, 192, 128, OPAQUE)
    );
}
