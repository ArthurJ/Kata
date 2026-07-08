use kata_rt::float_to_string;

#[test]
fn float_to_string_integer() {
    assert_eq!(float_to_string(5.0), "5.0");
}

#[test]
fn float_to_string_decimal() {
    // 3.14 pode ter imprecisão de f64, mas vamos testar valores exatos
    assert_eq!(float_to_string(0.5), "0.5");
}

#[test]
fn float_to_string_large() {
    let s = float_to_string(1000.0);
    assert_eq!(s, "1000.0");
}
