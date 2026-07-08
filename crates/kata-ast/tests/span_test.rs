use kata_ast::{Span, Spanned};

#[test]
fn span_new_sets_all_fields() {
    let s = Span::new(10, 2, 5, 3);
    assert_eq!(s.offset, 10);
    assert_eq!(s.line, 2);
    assert_eq!(s.col, 5);
    assert_eq!(s.len, 3);
}

#[test]
fn span_zero_has_line1_col1() {
    let s = Span::zero();
    assert_eq!(s.offset, 0);
    assert_eq!(s.line, 1);
    assert_eq!(s.col, 1);
    assert_eq!(s.len, 0);
}

#[test]
fn span_synthetic_has_line0() {
    let s = Span::synthetic();
    assert_eq!(s.line, 0);
    assert!(s.is_synthetic());
}

#[test]
fn span_non_synthetic_has_nonzero_line() {
    let s = Span::new(5, 1, 1, 2);
    assert!(!s.is_synthetic());
}

#[test]
fn span_cover_overlapping() {
    let a = Span::new(0, 1, 1, 10); // 0..10
    let b = Span::new(5, 1, 6, 20); // 5..25
    let c = a.cover(b);
    assert_eq!(c.offset, 0);
    assert_eq!(c.len, 25); // 25 - 0
}

#[test]
fn span_cover_disjoint() {
    let a = Span::new(0, 1, 1, 5); // 0..5
    let b = Span::new(20, 2, 1, 5); // 20..25
    let c = a.cover(b);
    assert_eq!(c.offset, 0);
    assert_eq!(c.len, 25); // 25 - 0
}

#[test]
fn span_cover_same_start() {
    let a = Span::new(10, 1, 1, 5);
    let b = Span::new(10, 1, 1, 20);
    let c = a.cover(b);
    assert_eq!(c.offset, 10);
    assert_eq!(c.len, 20);
}

#[test]
fn span_display_format() {
    let s = Span::new(42, 3, 7, 5);
    assert_eq!(format!("{s}"), "3:7@42+5");
}

#[test]
fn spanned_new_wraps_node_and_span() {
    let s = Span::new(0, 1, 1, 3);
    let sp = Spanned::new(42i32, s);
    assert_eq!(sp.node, 42);
    assert_eq!(sp.span, s);
}

#[test]
fn spanned_map_transforms_node_preserves_span() {
    let s = Span::new(0, 1, 1, 3);
    let sp = Spanned::new(5i32, s);
    let mapped = sp.map(|x| x * 2);
    assert_eq!(mapped.node, 10);
    assert_eq!(mapped.span, s);
}

#[test]
fn spanned_as_ref_borrows() {
    let s = Span::new(0, 1, 1, 3);
    let sp = Spanned::new(String::from("hello"), s);
    let r = sp.as_ref();
    assert_eq!(r.node, "hello");
    assert_eq!(r.span, s);
}
