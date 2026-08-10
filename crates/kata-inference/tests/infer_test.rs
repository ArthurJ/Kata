//! Integration tests for kata-inference (Pass 2 — type-check).
//!
//! Testa `infer_module` consumindo `Module` (via parser) + `ResolvedModule`
//! (via prelude) e verificando o `TypedModule` resultante: tipos em cada nó,
//! dispatch de overloads, ascription, let, variant qual, erros.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_prelude;

// ── Helpers ───────────────────────────────────────────────────────

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    infer_module(&module, &prelude).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── Literais ──────────────────────────────────────────────────────

#[test]
fn infer_int_literal() {
    let tmod = infer_src("42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    assert!(matches!(entry.kind, TypedExprKind::IntLit { .. }));
    assert!(entry.tail_pos);
}

#[test]
fn infer_float_literal() {
    let tmod = infer_src("3.14");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float());
    assert!(matches!(entry.kind, TypedExprKind::FloatLit { .. }));
}

#[test]
fn infer_text_literal() {
    let tmod = infer_src("\"hello\"");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::text());
    assert!(matches!(entry.kind, TypedExprKind::TextLit { .. }));
}

#[test]
fn infer_unit_literal() {
    let tmod = infer_src("()");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::Unit);
    assert!(matches!(entry.kind, TypedExprKind::Unit));
}

// ── Apply (dispatch por dominância) ────────────────────────────────

#[test]
fn infer_int_add() {
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            assert!(matches!(callee.node.kind, TypedExprKind::Ident { .. }));
            assert_eq!(args.len(), 2);
            assert_eq!(args[0].node.ty, Ty::int());
            assert_eq!(args[1].node.ty, Ty::int());
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_float_add() {
    let tmod = infer_src("+ 3.14 2.71");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_fadd"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_rational_add() {
    // + :: Rational Rational => Rational via kata_rt_rat_add
    let tmod = infer_src("+ 3.14::Rational 1.0::Rational");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::rational());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_rat_add"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_int_comparison_returns_boolean() {
    let tmod = infer_src("= 1 1");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_eq"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_float_comparison_returns_boolean() {
    let tmod = infer_src("< 3.14 2.71");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_fcmp_lt"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_bigint_mul() {
    let tmod = infer_src("* 99999999999999999999 99999999999999999999");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_mul"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_nested_apply() {
    // + 1 (* 2 3) — Int
    let tmod = infer_src("+ 1 (* 2 3)");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

#[test]
fn infer_show_int_returns_text() {
    let tmod = infer_src("show 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::text());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_show"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_show_rational_returns_text() {
    // Sintaxe prefix-only: / 1::Rational 3::Rational
    let tmod = infer_src("show (/ 1::Rational 3::Rational)");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::text());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_rat_show"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_echo_returns_unit() {
    let tmod = infer_src("echo \"hello\"");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::Unit);
    // echo agora é uma Action Kata (não FFI direto) com body que despacha show.
    // Closure { callee: Ident("echo"), ffi_symbol: None } — despacha via kata_refs.
    match &entry.kind {
        TypedExprKind::Closure {
            callee, ffi_symbol, ..
        } => {
            assert_eq!(*ffi_symbol, None, "echo é Action Kata, não FFI direto");
            // callee é Ident("echo")
            match &callee.node.kind {
                TypedExprKind::Ident { name } => assert_eq!(name, "echo"),
                other => panic!("expected Ident callee, got {other:?}"),
            }
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

// ── Let binding ───────────────────────────────────────────────────

#[test]
fn infer_let_binds_name_in_scope() {
    // let x := 42
    let tmod = infer_src("let x := 42");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::Unit); // let retorna Unit
    match &entry.kind {
        TypedExprKind::Let { name, value } => {
            assert_eq!(name, "x");
            assert_eq!(value.node.ty, Ty::int());
        }
        other => panic!("expected Let, got {other:?}"),
    }
}

#[test]
fn infer_let_then_use_in_apply() {
    // let x := 42
    // + x 1
    // (EntryExpr é a última — let e apply no mesmo módulo)
    let src = "let x := 42\n+ x 1";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure { args, .. } => {
            assert_eq!(args[0].node.ty, Ty::int());
            assert_eq!(args[1].node.ty, Ty::int());
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

// ── TypeAscription ────────────────────────────────────────────────

#[test]
fn infer_ascription_float_to_rational() {
    let tmod = infer_src("3.14::Rational");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::rational());
    match &entry.kind {
        TypedExprKind::TypeAscription { target_ty, .. } => {
            assert_eq!(*target_ty, Ty::rational());
        }
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

#[test]
fn infer_ascription_same_type_noop() {
    let tmod = infer_src("42::Int");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

#[test]
fn infer_ascription_int_to_float_rebaixa() {
    // IntLit rebaixa para Float — o literal "42" nasce como f64.
    let tmod = infer_src("42::Float");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float());
    match &entry.kind {
        TypedExprKind::TypeAscription { target_ty, .. } => {
            assert_eq!(*target_ty, Ty::float());
        }
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

#[test]
fn infer_ascription_mismatch_error() {
    // Text não pode ser ascribed como Int — não há rebaixamento entre
    // tipos de natureza diferente (apenas dentro da mesma categoria).
    let err = infer_src_err("\"hello\"::Int");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── Grouping ──────────────────────────────────────────────────────

#[test]
fn infer_grouping_transparent() {
    let tmod = infer_src("(+ 1 2)");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Grouping { inner } => {
            assert_eq!(inner.node.ty, Ty::int());
        }
        other => panic!("expected Grouping, got {other:?}"),
    }
}

// ── VariantQual ───────────────────────────────────────────────────

#[test]
fn infer_boolean_true() {
    let tmod = infer_src("Boolean::True");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
    match &entry.kind {
        TypedExprKind::VariantQual {
            enum_name, variant, ..
        } => {
            assert_eq!(enum_name, "Boolean");
            assert_eq!(variant, "True");
        }
        other => panic!("expected VariantQual, got {other:?}"),
    }
}

#[test]
fn infer_boolean_false() {
    let tmod = infer_src("Boolean::False");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

// ── Tuple (suportado — Ty::Tuple antecipado) ─────────────

#[test]
fn infer_tuple_three_elements() {
    let tmod = infer_src("(1, 2, 3)");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::Tuple(vec![Ty::int(), Ty::int(), Ty::int()]));
    match &entry.kind {
        TypedExprKind::Tuple { elements } => {
            assert_eq!(elements.len(), 3);
            assert_eq!(elements[0].node.ty, Ty::int());
            assert_eq!(elements[1].node.ty, Ty::int());
            assert_eq!(elements[2].node.ty, Ty::int());
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
}

// ── Erros ─────────────────────────────────────────────────────────

#[test]
fn infer_unbound_name_error() {
    let err = infer_src_err("+ x 1");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::UnboundName { .. }
    ));
}

#[test]
fn infer_no_overload_for_mixed_types() {
    // + :: Int Float => Float (cross-type overload) — agora succeeds
    // + 1 3.14 — Int e Float dão match com a cross-type overload Int Float => Float
    let tmod = infer_src("+ 1 3.14");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float(), "+ 1 3.14 deve retornar Float via cross-type overload");
}

#[test]
fn infer_unknown_function_error() {
    let err = infer_src_err("foobar 1 2");
    // `foobar` não existe no DispatchTable — UnboundName, não NoOverload.
    // NoOverload é "a função existe mas nenhuma sobrecarga casa com os tipos".
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::UnboundName { .. }
    ));
}

// ── TAST enriquecida (tail_pos) ──────────────────────────────────

#[test]
fn tast_enriched_tail_pos_true_for_entry() {
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    assert!(entry.tail_pos);
}

// ── DispatchTable populado ────────────────────────────────────────

#[test]
fn dispatch_table_has_prelude_signatures() {
    let tmod = infer_src("42");
    assert!(tmod.dispatch_table.has_function("+"));
    assert!(tmod.dispatch_table.has_function("-"));
    assert!(tmod.dispatch_table.has_function("*"));
    assert!(tmod.dispatch_table.has_function("/"));
    assert!(tmod.dispatch_table.has_function("show"));
    assert!(tmod.dispatch_table.has_function("echo"));
}

#[test]
fn dispatch_table_multiple_overloads_for_plus() {
    let tmod = infer_src("42");
    let overloads = tmod
        .dispatch_table
        .get_overloads("+")
        .expect("+ deve ter overloads");
    // + tem 12 overloads: Int, Float, Rational, List, Set+Set, Set+elem, Dict+Dict, Bytes+Bytes
    // + 4 cross-type: Int Float, Int Rational, Float Rational, Rational Float
    assert_eq!(overloads.len(), 12);
}

#[test]
fn dispatch_table_resolves_int_over_float() {
    let tmod = infer_src("+ 1 2");
    // Deve resolver para Int overload (kata_rt_bi_add), não Float
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn infer_grouping_callee() {
    // `(+)` é Grouping(Ident("+")). O callee descascado deve ser
    // despachado corretamente via DispatchTable.
    let tmod = infer_src("(+) (3) (4)");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
        }
        other => panic!("expected Closure (Apply), got {other:?}"),
    }
}
