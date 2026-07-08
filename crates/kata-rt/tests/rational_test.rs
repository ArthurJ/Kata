use kata_rt::rat_from_text;

#[test]
fn rat_from_integer_text() {
    let r = rat_from_text("42");
    assert_eq!(r.numer().to_string(), "42");
    assert_eq!(r.denom().to_string(), "1");
}

#[test]
fn rat_from_decimal_text() {
    let r = rat_from_text("3.14");
    // 3.14 = 314/100 = 157/50
    assert_eq!(r.numer().to_string(), "157");
    assert_eq!(r.denom().to_string(), "50");
}

#[test]
fn rat_from_half() {
    let r = rat_from_text("0.5");
    assert_eq!(r.numer().to_string(), "1");
    assert_eq!(r.denom().to_string(), "2");
}

#[test]
fn rat_from_zero_decimal() {
    let r = rat_from_text("5.0");
    assert_eq!(r.numer().to_string(), "5");
    assert_eq!(r.denom().to_string(), "1");
}

#[test]
fn rat_from_negative_decimal() {
    let r = rat_from_text("-1.5");
    assert_eq!(r.numer().to_string(), "-3");
    assert_eq!(r.denom().to_string(), "2");
}
