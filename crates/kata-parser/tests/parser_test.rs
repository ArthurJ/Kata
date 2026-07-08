//! Integration tests for kata-parser.
//!
//! These tests exercise the parser through the public `parse` API,
//! lexing source strings and verifying the resulting AST structure.

use kata_ast::{DirectiveArg, Expr, Item, TypeExpr};
use kata_lexer::lex;
use kata_parser::parse;

fn parse_src(src: &str) -> kata_ast::Module {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap()
}

fn first_item(m: &kata_ast::Module) -> &Item {
    &m.items.first().expect("at least one item").node
}

// ── Basic expression parsing ──────────────────────────────────────

#[test]
fn apply_plus_1_2() {
    let m = parse_src("+ 1 2");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(args[1].node, Expr::IntLit { text: "2".into() });
            }
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn let_binding() {
    let m = parse_src("let x := 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Let { name, value } => {
                assert_eq!(name, "x");
                assert_eq!(value.node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Let, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn type_ascription_rational() {
    let m = parse_src("3.14::Rational");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::TypeAscription { expr, ty } => {
                assert_eq!(
                    expr.node,
                    Expr::FloatLit {
                        text: "3.14".into()
                    }
                );
                assert_eq!(ty.node, TypeExpr::Named("Rational".into()));
            }
            other => panic!("expected TypeAscription, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn tuple_three_elements() {
    let m = parse_src("(1, 2, 3)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Tuple { elements } => {
                assert_eq!(elements.len(), 3);
                assert_eq!(elements[0].node, Expr::IntLit { text: "1".into() });
                assert_eq!(elements[1].node, Expr::IntLit { text: "2".into() });
                assert_eq!(elements[2].node, Expr::IntLit { text: "3".into() });
            }
            other => panic!("expected Tuple, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn tuple_trailing_comma() {
    let m = parse_src("(42,)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Tuple { elements } => {
                assert_eq!(elements.len(), 1);
                assert_eq!(elements[0].node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Tuple, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn grouping_single() {
    let m = parse_src("(42)");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Grouping { inner } => {
                assert_eq!(inner.node, Expr::IntLit { text: "42".into() });
            }
            other => panic!("expected Grouping, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn unit_literal() {
    let m = parse_src("()");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(e.node, Expr::Unit),
        other => panic!("expected EntryExpr(Unit), got {other:?}"),
    }
}

#[test]
fn variant_qual() {
    let m = parse_src("Boolean::True");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::VariantQual { enum_name, variant } => {
                assert_eq!(enum_name, "Boolean");
                assert_eq!(variant, "True");
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Declarations ──────────────────────────────────────────────────

#[test]
fn data_decl_empty() {
    let m = parse_src("data Int ()");
    let item = first_item(&m);
    match item {
        Item::DataDecl {
            name,
            fields,
            directives,
        } => {
            assert_eq!(name, "Int");
            assert!(fields.is_empty());
            assert!(directives.is_empty());
        }
        other => panic!("expected DataDecl, got {other:?}"),
    }
}

#[test]
fn enum_decl_variants() {
    let m = parse_src("enum Boolean\n    True\n    False");
    let item = first_item(&m);
    match item {
        Item::EnumDecl {
            name,
            variants,
            directives,
        } => {
            assert_eq!(name, "Boolean");
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].name, "True");
            assert_eq!(variants[1].name, "False");
            assert!(directives.is_empty());
        }
        other => panic!("expected EnumDecl, got {other:?}"),
    }
}

#[test]
fn sig_simple() {
    let m = parse_src("+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name,
            params,
            ret,
            directives,
            body,
        } => {
            assert_eq!(name, "+");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].node, TypeExpr::Named("Int".into()));
            assert_eq!(params[1].node, TypeExpr::Named("Int".into()));
            assert_eq!(ret.node, TypeExpr::Named("Int".into()));
            assert!(directives.is_empty());
            assert!(body.is_none());
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── Directives ────────────────────────────────────────────────────

#[test]
fn directive_ffi_with_sig() {
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name, directives, ..
        } => {
            assert_eq!(name, "+");
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(
                directives[0].args,
                vec![DirectiveArg::Str("kata_rt_bi_add".into())]
            );
        }
        other => panic!("expected Sig with directive, got {other:?}"),
    }
}

#[test]
fn directive_associative_int() {
    let m = parse_src("@associative(0)\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { directives, .. } => {
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "associative");
            assert_eq!(directives[0].args, vec![DirectiveArg::Int(0)]);
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

#[test]
fn multiple_directives_stacked() {
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n@associative(0)\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig { directives, .. } => {
            assert_eq!(directives.len(), 2);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(directives[1].name, "associative");
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── Multi-item module ────────────────────────────────────────────

#[test]
fn multiple_items() {
    let src = "data Int ()\nenum Boolean\n    True\n    False\n+ 1 2";
    let m = parse_src(src);
    assert_eq!(m.items.len(), 3);
    assert!(matches!(m.items[0].node, Item::DataDecl { .. }));
    assert!(matches!(m.items[1].node, Item::EnumDecl { .. }));
    assert!(matches!(m.items[2].node, Item::EntryExpr(_)));
}

// ── Greedy application ────────────────────────────────────────────

#[test]
fn greedy_application_three_args() {
    let m = parse_src("f a b c");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "f".into() });
                assert_eq!(args.len(), 3);
                assert_eq!(args[0].node, Expr::Ident { name: "a".into() });
                assert_eq!(args[1].node, Expr::Ident { name: "b".into() });
                assert_eq!(args[2].node, Expr::Ident { name: "c".into() });
            }
            other => panic!("expected Apply, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

// ── Text literals ────────────────────────────────────────────────

#[test]
fn text_literal() {
    let m = parse_src("\"hello\"");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(
            e.node,
            Expr::TextLit {
                text: "hello".into()
            }
        ),
        other => panic!("expected EntryExpr(TextLit), got {other:?}"),
    }
}

// ── Identifier as expression ─────────────────────────────────────

#[test]
fn ident_alone() {
    let m = parse_src("x");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => assert_eq!(e.node, Expr::Ident { name: "x".into() }),
        other => panic!("expected EntryExpr(Ident), got {other:?}"),
    }
}

// ── Fio 2: Lambda anônimo ────────────────────────────────────────

#[test]
fn lambda_anon_simple() {
    let m = parse_src("lambda x: + x 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda {
                patterns,
                body,
                guards,
                with_bindings,
            } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("x".into()));
                // body é `+ x 1` (Apply)
                match &body.node {
                    Expr::Apply { callee, args } => {
                        assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                        assert_eq!(args.len(), 2);
                    }
                    other => panic!("expected Apply body, got {other:?}"),
                }
                assert!(guards.is_empty());
                assert!(with_bindings.is_empty());
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_multi_pattern() {
    let m = parse_src("lambda x y: + x y");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 2);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("x".into()));
                assert_eq!(patterns[1].node, kata_ast::Pattern::Ident("y".into()));
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_unicode() {
    let m = parse_src("λ n: + n 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Ident("n".into()));
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_wildcard_pattern() {
    let m = parse_src("lambda _: 42");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                assert_eq!(patterns[0].node, kata_ast::Pattern::Wildcard);
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_literal_pattern() {
    let m = parse_src("lambda 0: 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Literal(expr) => {
                        assert_eq!(expr.node, Expr::IntLit { text: "0".into() });
                    }
                    other => panic!("expected Literal pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_variant_pattern() {
    let m = parse_src("lambda Boolean::True: 1");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Variant {
                        enum_name,
                        variant,
                    } => {
                        assert_eq!(enum_name, "Boolean");
                        assert_eq!(variant, "True");
                    }
                    other => panic!("expected Variant pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

#[test]
fn lambda_anon_tuple_pattern() {
    let m = parse_src("lambda (a, b): a");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Lambda { patterns, .. } => {
                assert_eq!(patterns.len(), 1);
                match &patterns[0].node {
                    kata_ast::Pattern::Tuple(elements) => {
                        assert_eq!(elements.len(), 2);
                        assert_eq!(elements[0].node, kata_ast::Pattern::Ident("a".into()));
                        assert_eq!(elements[1].node, kata_ast::Pattern::Ident("b".into()));
                    }
                    other => panic!("expected Tuple pattern, got {other:?}"),
                }
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Lambda), got {other:?}"),
    }
}

// ── Fio 2: Hole (`_`) em posição de argumento ────────────────────

#[test]
fn hole_in_apply_arg() {
    let m = parse_src("+ 10 _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(callee.node, Expr::Ident { name: "+".into() });
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::IntLit { text: "10".into() });
                assert_eq!(args[1].node, Expr::Hole);
            }
            other => panic!("expected Apply with Hole, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Apply), got {other:?}"),
    }
}

#[test]
fn hole_multiple() {
    let m = parse_src("+ _ _");
    let item = first_item(&m);
    match item {
        Item::EntryExpr(e) => match &e.node {
            Expr::Apply { callee, args } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].node, Expr::Hole);
                assert_eq!(args[1].node, Expr::Hole);
            }
            other => panic!("expected Apply with 2 Holes, got {other:?}"),
        },
        other => panic!("expected EntryExpr(Apply), got {other:?}"),
    }
}
