//! Testes de parser de interface, implements, import, export, Self.
//!
//! Valida que o parser reconhece as novas declarações e produz a AST correta.

use kata_ast::{Item, TypeExpr};
use kata_lexer::lex;
use kata_parser::parse;

fn parse_src(src: &str) -> kata_ast::Module {
    let tokens = lex(src).unwrap();
    parse(tokens).unwrap()
}

fn first_item(m: &kata_ast::Module) -> &Item {
    &m.items.first().expect("at least one item").node
}

// ── Interface ──────────────────────────────────────────────────

#[test]
fn interface_simple() {
    let src = "interface NUM\n    + :: NUM NUM => NUM\n    abs :: NUM => NUM";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl {
            name,
            supertraits,
            type_params,
            signatures,
        } => {
            assert_eq!(name, "NUM");
            assert!(supertraits.is_empty());
            assert!(type_params.is_empty());
            assert_eq!(signatures.len(), 2);
            assert_eq!(signatures[0].name, "+");
            assert_eq!(signatures[0].params.len(), 2);
            assert_eq!(signatures[0].ret.node, TypeExpr::Named("NUM".into()));
            assert_eq!(signatures[1].name, "abs");
            assert_eq!(signatures[1].params.len(), 1);
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

#[test]
fn interface_with_supertraits() {
    let src = "interface NUM implements ORD EQ\n    + :: NUM NUM => NUM";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl {
            name, supertraits, ..
        } => {
            assert_eq!(name, "NUM");
            assert_eq!(supertraits, &["ORD".to_string(), "EQ".to_string()]);
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

#[test]
fn interface_with_type_params() {
    let src = "interface ITERABLE::(A)\n    next :: Self => Optional::(A)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl {
            name,
            type_params,
            signatures,
            ..
        } => {
            assert_eq!(name, "ITERABLE");
            assert_eq!(type_params, &["A".to_string()]);
            assert_eq!(signatures.len(), 1);
            assert_eq!(signatures[0].name, "next");
            assert_eq!(signatures[0].params.len(), 1);
            assert_eq!(signatures[0].params[0].node, TypeExpr::SelfRef);
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

// ── Implements ─────────────────────────────────────────────────

#[test]
fn implements_with_ffi() {
    let src = "Int implements NUM\n    @ffi(\"kata_rt_bi_add\")\n    + :: Int Int => Int";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImplementsDecl {
            type_name,
            type_params,
            interface_name,
            iface_params,
            methods,
        } => {
            assert_eq!(type_name, "Int");
            assert!(type_params.is_empty());
            assert_eq!(interface_name, "NUM");
            assert!(iface_params.is_empty());
            assert_eq!(methods.len(), 1);
            assert_eq!(methods[0].name, "+");
            assert_eq!(methods[0].params.len(), 2);
            assert_eq!(methods[0].ret.node, TypeExpr::Named("Int".into()));
            assert!(methods[0].body.is_none()); // FFI — sem corpo
            assert_eq!(methods[0].directives.len(), 1);
            assert_eq!(methods[0].directives[0].name, "ffi");
        }
        other => panic!("expected ImplementsDecl, got {other:?}"),
    }
}

#[test]
fn implements_with_lambda_body() {
    let src = "Complex implements NUM\n    + :: Complex Complex => Complex\n    lambda a b: a";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImplementsDecl {
            methods,
            type_name,
            interface_name,
            ..
        } => {
            assert_eq!(type_name, "Complex");
            assert_eq!(interface_name, "NUM");
            assert_eq!(methods.len(), 1);
            assert!(methods[0].body.is_some()); // tem corpo lambda
        }
        other => panic!("expected ImplementsDecl, got {other:?}"),
    }
}

#[test]
fn implements_with_type_params() {
    let src = "List::(A) implements ITERABLE::(A)\n    next :: List::(A) => Optional::(A)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImplementsDecl {
            type_name,
            type_params,
            interface_name,
            iface_params,
            ..
        } => {
            assert_eq!(type_name, "List");
            assert_eq!(type_params, &["A".to_string()]);
            assert_eq!(interface_name, "ITERABLE");
            assert_eq!(iface_params, &["A".to_string()]);
        }
        other => panic!("expected ImplementsDecl, got {other:?}"),
    }
}

// ── Import ─────────────────────────────────────────────────────

#[test]
fn import_simple() {
    let src = "import utilidades.matematica";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImportDecl { path, alias, items } => {
            assert_eq!(path, &["utilidades".to_string(), "matematica".to_string()]);
            assert!(alias.is_none());
            assert!(items.is_none());
        }
        other => panic!("expected ImportDecl, got {other:?}"),
    }
}

#[test]
fn import_with_alias() {
    // `import utilidades.matematica as mat` — açúcar para
    // `import utilidades.(matematica as mat)` (import seletivo com alias).
    let src = "import utilidades.matematica as mat";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImportDecl { path, alias, items } => {
            // O açúcar transforma em path=["utilidades"], items=[ImportItem{matematica, Some(mat)}]
            assert_eq!(path, &["utilidades".to_string()]);
            assert!(alias.is_none());
            let items = items.clone().expect("deve ter items (seletivo)");
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].name, "matematica");
            assert_eq!(items[0].alias.as_deref(), Some("mat"));
        }
        other => panic!("expected ImportDecl, got {other:?}"),
    }
}

#[test]
fn import_module_with_alias() {
    // `import utilidades as util` — alias do módulo inteiro (não açúcar).
    let src = "import utilidades as util";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImportDecl { path, alias, items } => {
            assert_eq!(path, &["utilidades".to_string()]);
            assert_eq!(alias.as_deref(), Some("util"));
            assert!(items.is_none());
        }
        other => panic!("expected ImportDecl, got {other:?}"),
    }
}

#[test]
fn import_selective() {
    let src = "import utilidades.(matematica TipoX)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImportDecl { path, items, .. } => {
            assert_eq!(path, &["utilidades".to_string()]);
            let items = items.as_ref().expect("selective import");
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].name, "matematica");
            assert!(items[0].alias.is_none());
            assert_eq!(items[1].name, "TipoX");
            assert!(items[1].alias.is_none());
        }
        other => panic!("expected ImportDecl, got {other:?}"),
    }
}

// ── Export ─────────────────────────────────────────────────────

#[test]
fn export_simple() {
    let src = "export + - TipoX";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ExportDecl { items } => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].name, "+");
            assert!(items[0].reexport_from.is_none());
            assert_eq!(items[1].name, "-");
            assert_eq!(items[2].name, "TipoX");
        }
        other => panic!("expected ExportDecl, got {other:?}"),
    }
}

#[test]
fn export_reexport() {
    let src = "export tipos.(Int Float)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ExportDecl { items } => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].reexport_from.as_deref(), Some("tipos"));
            let reexport = items[0].reexport_items.as_ref().expect("reexport items");
            assert_eq!(reexport, &["Int".to_string(), "Float".to_string()]);
        }
        other => panic!("expected ExportDecl, got {other:?}"),
    }
}

// ── Self em posição de tipo ────────────────────────────────────

#[test]
fn self_in_interface_signature() {
    let src = "interface EQ\n    = :: Self Self => Boolean";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl { signatures, .. } => {
            assert_eq!(signatures[0].name, "=");
            assert_eq!(signatures[0].params.len(), 2);
            assert_eq!(signatures[0].params[0].node, TypeExpr::SelfRef);
            assert_eq!(signatures[0].params[1].node, TypeExpr::SelfRef);
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

// ── Sintaxe :: sem parênteses para single-param ────────

#[test]
fn interface_type_params_single_param_no_parens() {
    let src = "interface ITERABLE::A\n    next :: Self => Optional::A";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl {
            name,
            type_params,
            signatures,
            ..
        } => {
            assert_eq!(name, "ITERABLE");
            assert_eq!(type_params, &["A".to_string()]);
            assert_eq!(signatures[0].name, "next");
            // Optional::A → ParamApp { name: "Optional", params: [Named("A")] }
            match &signatures[0].ret.node {
                TypeExpr::ParamApp { name, params } => {
                    assert_eq!(name, "Optional");
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].node, TypeExpr::Named("A".into()));
                }
                other => panic!("expected ParamApp for ret, got {other:?}"),
            }
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

#[test]
fn implements_type_params_single_param_no_parens() {
    let src = "List::A implements ITERABLE::A\n    next :: List::A => Optional::A";
    let m = parse_src(src);
    match first_item(&m) {
        Item::ImplementsDecl {
            type_name,
            type_params,
            interface_name,
            iface_params,
            ..
        } => {
            assert_eq!(type_name, "List");
            assert_eq!(type_params, &["A".to_string()]);
            assert_eq!(interface_name, "ITERABLE");
            assert_eq!(iface_params, &["A".to_string()]);
        }
        other => panic!("expected ImplementsDecl, got {other:?}"),
    }
}

#[test]
fn type_expr_single_param_no_parens() {
    // `List::A` como type expression em assinatura.
    let src = "interface INDEXABLE::A\n    at :: Self Int => Result::(A, Err)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl {
            name, type_params, ..
        } => {
            assert_eq!(name, "INDEXABLE");
            assert_eq!(type_params, &["A".to_string()]);
        }
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}

#[test]
fn type_expr_tuple_still_requires_parens() {
    // `Result::(A, Err)` — tupla de tipos continua exigindo parênteses.
    let src = "interface INDEXABLE::A\n    at :: Self Int => Result::(A, Err)";
    let m = parse_src(src);
    match first_item(&m) {
        Item::InterfaceDecl { signatures, .. } => match &signatures[0].ret.node {
            TypeExpr::ParamApp { name, params } => {
                assert_eq!(name, "Result");
                assert_eq!(params.len(), 2);
            }
            other => panic!("expected ParamApp for Result::(A, Err), got {other:?}"),
        },
        other => panic!("expected InterfaceDecl, got {other:?}"),
    }
}
