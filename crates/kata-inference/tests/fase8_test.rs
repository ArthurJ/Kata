//! Testes da Fase 8 — inference de lambda, match, patterns, guards,
//! exaustividade, e renomeação Apply → Closure.
//!
//! Estes testes focam no typeck (Pass 2), não no codegen (Fase 9).
//! O codegen de Lambda/Match será implementado na Fase 9.

use kata_core::ty::Ty;
use kata_inference::{infer_module, Effect, TypedExprKind};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::load_prelude;

// ── Helpers (duplicados de infer_test.rs para isolamento) ─────────

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

// ── Renomeação Apply → Closure ────────────────────────────────────

#[test]
fn closure_rename_int_add() {
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Closure {
            ffi_symbol,
            captures,
            escapes,
            ..
        } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
            assert!(captures.is_empty(), "Fio 2: captures sempre vazio");
            assert!(!escapes, "Fio 2: escapes sempre false");
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

// ── Match em Boolean (exaustivo) ──────────────────────────────────

#[test]
fn match_boolean_exhaustive() {
    let tmod = infer_src("match Boolean::True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Match { scrutinee, arms } => {
            assert_eq!(scrutinee.node.ty, Ty::boolean());
            assert_eq!(arms.len(), 2);
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

// ── Match em Boolean (não-exaustivo → erro) ──────────────────────

#[test]
fn match_boolean_non_exhaustive_error() {
    let err = infer_src_err("match Boolean::True\n    True: 1");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
    ));
}

// ── Match com otherwise (sempre exaustivo) ────────────────────────

#[test]
fn match_with_otherwise_is_exhaustive() {
    let tmod = infer_src("match Boolean::True\n    True: 1\n    otherwise: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match em Int (tipo infinito) exige otherwise ─────────────────

#[test]
fn match_int_without_otherwise_errors() {
    let err = infer_src_err("match 42\n    0: 1");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::MissingOtherwise { .. }
    ));
}

#[test]
fn match_int_with_otherwise_ok() {
    let tmod = infer_src("match 42\n    0: 1\n    otherwise: 99");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match com variantes sem qualificação ─────────────────────────

#[test]
fn match_unqualified_variants_resolved() {
    // True e False sem qualificação são resolvidos pelo EnumRegistry
    // como variantes de Boolean (tipo do scrutinee).
    let tmod = infer_src("match Boolean::True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Match { arms, .. } => {
            // True e False devem ser resolvidos como Variant, não Ident.
            for arm in arms {
                if let Some(pat) = &arm.pattern {
                    assert!(
                        matches!(
                            &pat.node,
                            kata_inference::TypedPattern::Variant { .. }
                        ),
                        "pattern deve ser Variant, não Ident"
                    );
                }
            }
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

// ── Match com qualified variants ──────────────────────────────────

#[test]
fn match_qualified_variants() {
    let tmod =
        infer_src("match Boolean::True\n    Boolean::True: 1\n    Boolean::False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match: braços devem retornar mesmo tipo ──────────────────────

#[test]
fn match_arms_type_mismatch_error() {
    let err = infer_src_err("match Boolean::True\n    True: 1\n    False: \"texto\"");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── Match com wildcard ───────────────────────────────────────────

#[test]
fn match_with_wildcard_is_exhaustive() {
    // Wildcard (_) cobre tudo — não precisa de otherwise.
    let tmod = infer_src("match 42\n    0: 1\n    _: 99");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match com Ident (binding) ────────────────────────────────────

#[test]
fn match_with_ident_binding() {
    // x liga o valor — otherwise implícito (cobre qualquer valor).
    let tmod = infer_src("match 42\n    0: 1\n    x: x");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Lambda anônimo: tipo Function ────────────────────────────────

#[test]
fn lambda_anon_has_function_type() {
    // lambda x: x — identidade. x é InferVar, ret é InferVar.
    // O tipo do lambda é Function([InferVar(0)], InferVar(0)).
    let tmod = infer_src("lambda x: x");
    let entry = entry_typed(&tmod);
    match &entry.ty {
        Ty::Function(params, ret) => {
            assert_eq!(params.len(), 1);
            // InferVar — não sabemos o tipo real sem inferência bidirecional.
            let _ = ret;
        }
        other => panic!("expected Function type, got {other:?}"),
    }
    match &entry.kind {
        TypedExprKind::Lambda {
            param_types,
            clauses,
            ..
        } => {
            assert_eq!(param_types.len(), 1);
            assert_eq!(clauses.len(), 1, "lambda anônimo tem 1 cláusula");
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

// ── Lambda anônimo com 2 parâmetros ──────────────────────────────

#[test]
fn lambda_anon_two_params() {
    let tmod = infer_src("lambda a b: a");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Lambda { param_types, .. } => {
            assert_eq!(param_types.len(), 2);
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

// ── Guards: verificação de tipo Boolean ───────────────────────────

#[test]
fn guard_condition_must_be_boolean() {
    // Guard com condição Boolean válida: > x 0 onde x é Int.
    // Usa match em Boolean para evitar InferVar (tipo conhecido).
    // O lambda tem guarda com condição Boolean.
    // Como x é InferVar, > x 0 não resolvable. Usa uma constante Boolean.
    let src = "lambda x:\n    Boolean::True: x\n    otherwise: x";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Lambda { clauses, .. } => {
            assert_eq!(clauses.len(), 1);
            assert!(
                !clauses[0].guards.is_empty(),
                "deve ter guards"
            );
        }
        other => panic!("expected Lambda, got {other:?}"),
    }
}

#[test]
fn guard_condition_non_boolean_error() {
    // Guard cuja condição é Int (não Boolean) deve dar TypeMismatch.
    // Condição: 42 (IntLit, não Boolean)
    let src = "lambda x:\n    42: x\n    otherwise: x";
    let err = infer_src_err(src);
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── Tuple patterns em match ──────────────────────────────────────

#[test]
fn match_tuple_pattern() {
    // match (1, 2) com pattern (a, b) — a e b são Int, ret é Int.
    let tmod = infer_src("match (1, 2)\n    (a, b): a\n    otherwise: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Cons pattern rejeitado (Fio 8) ────────────────────────────────

#[test]
fn cons_pattern_rejected_in_fio2() {
    // [h : t] é reconhecido pelo parser mas rejeitado pelo typeck.
    let err = infer_src_err("match 42\n    [h : t]: h\n    otherwise: 0");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}

// ── tail_pos propagação ───────────────────────────────────────────

#[test]
fn match_tail_pos_propagated() {
    // match em tail position — body de cada arm é tail_pos = true.
    let tmod = infer_src("match Boolean::True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert!(entry.tail_pos, "entry point é tail_pos");
    match &entry.kind {
        TypedExprKind::Match { arms, .. } => {
            for arm in arms {
                assert!(
                    arm.body.node.tail_pos,
                    "body de match arm em tail_pos deve ser true"
                );
            }
        }
        other => panic!("expected Match, got {other:?}"),
    }
}

#[test]
fn let_value_not_tail_pos() {
    // let value é tail_pos = false.
    let tmod = infer_src("let x := 42");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Let { value, .. } => {
            assert!(
                !value.node.tail_pos,
                "let value deve ser tail_pos = false"
            );
        }
        other => panic!("expected Let, got {other:?}"),
    }
}

#[test]
fn apply_args_not_tail_pos() {
    // Argumentos de Apply são tail_pos = false.
    let tmod = infer_src("+ 1 2");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::Closure { args, .. } => {
            for arg in args {
                assert!(
                    !arg.node.tail_pos,
                    "argumento de Closure deve ser tail_pos = false"
                );
            }
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

// ── Lambda como valor (call_indirect) ─────────────────────────────

#[test]
fn lambda_assigned_to_var_has_function_type() {
    // let f := lambda x: x
    // f é Ty::Function no TypeEnv. O typeck aceita.
    // Não chamamos f ainda — call_indirect com InferVar precisa de
    // inferência bidirecional (não implementada em Fio 2).
    let tmod = infer_src("let f := lambda x: x\n42");
    let entry = entry_typed(&tmod);
    // Entry é 42 (Int) — o let define f mas entry é a última expr.
    assert_eq!(entry.ty, Ty::int());
    // f está no TypeEnv como Ty::Function.
    let f_ty = tmod.type_env.lookup("f");
    assert!(matches!(f_ty, Some(Ty::Function(_, _))));
}

// ── Effect sempre Puro em Fio 2 ───────────────────────────────────

#[test]
fn lambda_effect_is_puro() {
    let tmod = infer_src("lambda x: x");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.effect, Effect::Puro);
}

#[test]
fn match_effect_is_puro() {
    let tmod = infer_src("match Boolean::True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.effect, Effect::Puro);
}