use super::*;

#[test]
fn pascal_case_valid() {
    assert!(is_pascal_case("Pessoa"));
    assert!(is_pascal_case("Boolean"));
    assert!(is_pascal_case("True"));
    assert!(is_pascal_case("Int"));
    assert!(is_pascal_case("PositiveInt"));
}

#[test]
fn pascal_case_invalid() {
    assert!(!is_pascal_case("pessoa"));
    assert!(!is_pascal_case("minha_enum"));
    assert!(!is_pascal_case("snake_case"));
    assert!(!is_pascal_case(""));
}

#[test]
fn snake_case_valid() {
    assert!(is_snake_case("soma"));
    assert!(is_snake_case("minha_var"));
    assert!(is_snake_case("x"));
    assert!(is_snake_case("main"));
}

#[test]
fn snake_case_invalid() {
    assert!(!is_snake_case("Soma"));
    assert!(!is_snake_case("MinhaVar"));
    assert!(!is_snake_case(""));
}

#[test]
fn all_caps_valid() {
    assert!(is_all_caps("NUM"));
    assert!(is_all_caps("SHOW"));
    assert!(is_all_caps("ALL_CAPS"));
    assert!(is_all_caps("EQ"));
}

#[test]
fn all_caps_invalid() {
    assert!(!is_all_caps("num"));
    assert!(!is_all_caps("Num"));
    assert!(!is_all_caps("NumCaps"));
    assert!(!is_all_caps(""));
}
