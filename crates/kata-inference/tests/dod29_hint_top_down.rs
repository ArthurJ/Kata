//! DoD 29 — Hint top-down via ascription em lambda.
//!
//! `(lambda x: + x 1)::(Int -> Int)` extrai `x: Int` do tipo anotado.
//! O typeck propaga o tipo anotado como hint top-down para o lambda,
//! permitindo que os parâmetros sejam tipados sem partial dispatch.
//!
//! Sintaxe de type expression de função: `(A B C -> D)` — params separados
//! por espaço, `->` separa params do retorno, tudo dentro dos parênteses.

use kata_core::ty::Ty;
use kata_inference::{infer_module, TypedExprKind};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_prelude;

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect("inferência deve succeed")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── Casos do DoD 29 ────────────────────────────────────────────────

/// `(lambda x: + x 1)::(Int -> Int)` — ascription em lambda.
/// O hint `Int -> Int` propaga `x: Int` para dentro do lambda.
/// Nota: o parser wrapping `()` produz Grouping em volta do lambda na TAST.
#[test]
fn hint_top_down_lambda_ascription() {
    let tmod = infer_src("(lambda x: + x 1)::(Int -> Int)");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int()], Box::new(Ty::int())),
        "lambda deve ter tipo Int -> Int"
    );

    // Descasca Grouping (wrapping de parênteses) para chegar ao Lambda.
    fn peel_grouping_typed<'a>(expr: &'a TypedExprKind) -> &'a TypedExprKind {
        match expr {
            TypedExprKind::Grouping { inner } => &inner.node.kind,
            other => other,
        }
    }

    match &entry.kind {
        TypedExprKind::TypeAscription { expr, .. } => match peel_grouping_typed(&expr.node.kind) {
            TypedExprKind::Lambda {
                param_types,
                ret_ty,
                ..
            } => {
                assert_eq!(param_types, &[Ty::int()], "x deve ser Int via hint");
                assert_eq!(*ret_ty, Ty::int(), "retorno deve ser Int");
            }
            other => panic!("expected Lambda inside ascription, got {other:?}"),
        },
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

/// `(lambda x y: + x y)::(Int Int -> Int)` — dois parâmetros.
#[test]
fn hint_top_down_two_params() {
    let tmod = infer_src("(lambda x y: + x y)::(Int Int -> Int)");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())),
        "lambda de 2 params deve ter tipo (Int Int -> Int)"
    );
}

/// `(lambda x: + x 1.0)::(Float -> Float)` — hint Float.
#[test]
fn hint_top_down_float() {
    let tmod = infer_src("(lambda x: + x 1.0)::(Float -> Float)");
    let entry = entry_typed(&tmod);

    assert_eq!(
        entry.ty,
        Ty::Function(vec![Ty::float()], Box::new(Ty::float())),
        "lambda deve ter tipo Float -> Float"
    );
}

/// `let f := (lambda x: + x 1)::(Int -> Int); f 5` — aplicação do lambda tipado via hint.
#[test]
fn hint_top_down_then_apply() {
    let tmod = infer_src("let f := (lambda x: + x 1)::(Int -> Int)\nf 5");
    let entry = entry_typed(&tmod);

    assert_eq!(entry.ty, Ty::int(), "f 5 deve retornar Int");
}

/// `((lambda x: + x 1)::(Int -> Int)) 42` — aplica o lambda tipado.
///
/// TODO: requer DoD 31 (apply de lambda inline) — `infer_apply` só aceita
/// `Expr::Ident` como callee. Quando o callee é `TypeAscription(Grouping(Lambda))`,
/// precisa dispatch indireto via TypeEnv.
#[test]
#[ignore = "requer DoD 31 — apply de lambda inline"]
fn hint_top_down_direct_apply() {
    let tmod = infer_src("((lambda x: + x 1)::(Int -> Int)) 42");
    let entry = entry_typed(&tmod);

    assert_eq!(entry.ty, Ty::int(), "aplicação deve retornar Int");
}