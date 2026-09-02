use super::*;
use kata_ast::{DirectiveArg, Expr, ImportItem, Item, TypeExpr};
use kata_lexer::lex;

fn parse_src(src: &str) -> Module {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap()
}

fn first_item(m: &Module) -> &Item {
    &m.items.first().expect("at least one item").node
}

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
    let m = parse_src("constant x := 42");
    let item = first_item(&m);
    match item {
        Item::ConstantDecl { name, value } => {
            assert_eq!(name, "x");
            assert_eq!(value.node, Expr::IntLit { text: "42".into() });
        }
        other => panic!("expected ConstantDecl, got {other:?}"),
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
fn unit_lit() {
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
            Expr::VariantQual {
                enum_name, variant, ..
            } => {
                assert_eq!(enum_name, "Boolean");
                assert_eq!(variant, "True");
            }
            other => panic!("expected VariantQual, got {other:?}"),
        },
        other => panic!("expected EntryExpr, got {other:?}"),
    }
}

#[test]
fn data_decl_empty() {
    let m = parse_src("data Int ()");
    let item = first_item(&m);
    match item {
        Item::DataDecl {
            name,
            fields,
            directives,
            ..
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
            ..
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

#[test]
fn directive_ffi() {
    let m = parse_src("@ffi(\"kata_rt_bi_add\")\n+ :: Int Int => Int");
    let item = first_item(&m);
    match item {
        Item::Sig {
            name, directives, ..
        } => {
            assert_eq!(name, "+");
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "ffi");
            assert_eq!(directives[0].args.len(), 1);
            match &directives[0].args[0] {
                DirectiveArg::Expr(e) => {
                    assert_eq!(
                        e.node,
                        Expr::TextLit {
                            text: "kata_rt_bi_add".into()
                        }
                    );
                }
                other => panic!("expected Expr arg, got {other:?}"),
            }
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
            assert_eq!(directives[0].args.len(), 1);
            match &directives[0].args[0] {
                DirectiveArg::Expr(e) => {
                    assert_eq!(e.node, Expr::IntLit { text: "0".into() });
                }
                other => panic!("expected Expr arg, got {other:?}"),
            }
        }
        other => panic!("expected Sig, got {other:?}"),
    }
}

// ── Import paths: super e stdlib ──────────────────────

fn first_import(m: &Module) -> (&[String], &Option<String>, &Option<Vec<ImportItem>>) {
    match first_item(m) {
        Item::ImportDecl { path, alias, items } => (path, alias, items),
        other => panic!("expected ImportDecl, got {other:?}"),
    }
}

#[test]
fn import_super_single() {
    let m = parse_src("import super.calculus");
    let (path, _, _) = first_import(&m);
    assert_eq!(path, &vec!["super", "calculus"]);
}

#[test]
fn import_super_double() {
    let m = parse_src("import super.super.utils");
    let (path, _, _) = first_import(&m);
    assert_eq!(path, &vec!["super", "super", "utils"]);
}

#[test]
fn import_super_nested() {
    let m = parse_src("import super.vectors.vec2");
    let (path, _, _) = first_import(&m);
    assert_eq!(path, &vec!["super", "vectors", "vec2"]);
}

#[test]
fn import_super_selective() {
    let m = parse_src("import super.calculus.(dobrar fatorial)");
    let (path, _, items) = first_import(&m);
    assert_eq!(path, &vec!["super", "calculus"]);
    let items = items.as_ref().expect("selective import");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "dobrar");
    assert_eq!(items[1].name, "fatorial");
}

#[test]
fn import_super_alias() {
    let m = parse_src("import super.calculus as calc");
    let (path, alias, _) = first_import(&m);
    // super.calculus as calc — alias do módulo inteiro
    // (normal_components = 1, não 2, então não desugara)
    assert_eq!(path, &vec!["super", "calculus"]);
    assert_eq!(alias, &Some("calc".to_string()));
}

#[test]
fn import_stdlib_basic() {
    let m = parse_src("import stdlib.math");
    let (path, _, _) = first_import(&m);
    assert_eq!(path, &vec!["stdlib", "math"]);
}

#[test]
fn import_stdlib_selective() {
    let m = parse_src("import stdlib.math.(sqrt)");
    let (path, _, items) = first_import(&m);
    assert_eq!(path, &vec!["stdlib", "math"]);
    let items = items.as_ref().expect("selective import");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "sqrt");
}

#[test]
fn import_stdlib_alias() {
    let m = parse_src("import stdlib.complex as cm");
    let (path, alias, _) = first_import(&m);
    assert_eq!(path, &vec!["stdlib", "complex"]);
    assert_eq!(alias, &Some("cm".to_string()));
}

#[test]
fn import_super_after_normal_is_error() {
    // `import math.super` — super não pode aparecer após componente normal
    let tokens = lex("import math.super").unwrap();
    assert!(parse(tokens).is_err());
}

#[test]
fn import_super_alone_is_error() {
    // `import super` sozinho não carrega nada
    let tokens = lex("import super").unwrap();
    assert!(parse(tokens).is_err());
}

#[test]
fn import_stdlib_alone_is_error() {
    // `import stdlib` sozinho não carrega nada
    let tokens = lex("import stdlib").unwrap();
    assert!(parse(tokens).is_err());
}

#[test]
fn import_existing_still_works() {
    // Retrocompatibilidade: import sem prefixo especial
    let m = parse_src("import modulo.submodulo");
    let (path, _, _) = first_import(&m);
    assert_eq!(path, &vec!["modulo", "submodulo"]);
}

#[test]
fn import_existing_selective_still_works() {
    let m = parse_src("import modulo.(item1 item2 as alias2)");
    let (path, _, items) = first_import(&m);
    assert_eq!(path, &vec!["modulo"]);
    let items = items.as_ref().expect("selective import");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "item1");
    assert_eq!(items[1].name, "item2");
    assert_eq!(items[1].alias, Some("alias2".to_string()));
}

#[test]
fn import_existing_alias_desugar_still_works() {
    // `import mod.item as al` desugara para `import mod.(item as al)`
    let m = parse_src("import modulo.item as al");
    let (path, _, items) = first_import(&m);
    assert_eq!(path, &vec!["modulo"]);
    let items = items.as_ref().expect("selective import");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "item");
    assert_eq!(items[0].alias, Some("al".to_string()));
}
