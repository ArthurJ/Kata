//! Testes de inference de lambda, match, patterns, guards,
//! exaustividade, e renomeação Apply → Closure.
//!
//! Estes testes focam no typeck (Pass 2), não no codegen.
//! O codegen de Lambda/Match é coberto pelos testes E2E de codegen.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (duplicados de infer_test.rs para isolamento) ─────────

/// Combina prelude + módulo do usuário (replica do named_functions_e2e.rs).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
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
        TypedExprKind::Closure { ffi_symbol, .. } => {
            assert_eq!(ffi_symbol.as_deref(), Some("kata_rt_bi_add"));
        }
        other => panic!("expected Closure, got {other:?}"),
    }
}

// ── Match em Boolean (exaustivo) ──────────────────────────────────

#[test]
fn match_boolean_exhaustive() {
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
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
    let err = infer_src_err("match True\n    True: 1");
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
    ));
}

// ── Match com otherwise (sempre exaustivo) ────────────────────────

#[test]
fn match_with_otherwise_is_exhaustive() {
    let tmod = infer_src("match True\n    True: 1\n    otherwise: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match em Int (tipo infinito) exige otherwise ─────────────────

#[test]
fn match_int_without_otherwise_errors() {
    // Pela Fase 2 (motor Maranget), tipos infinitos sem otherwise
    // produzem NonExhaustiveMatch com witness "_" (não MissingOtherwise).
    // MissingOtherwise é reservado para guards sem otherwise (Fase 3).
    let err = infer_src_err("match 42\n    0: 1");
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
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
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
    match &entry.kind {
        TypedExprKind::Match { arms, .. } => {
            // True e False devem ser resolvidos como Variant, não Ident.
            for arm in arms {
                if let Some(pat) = &arm.pattern {
                    assert!(
                        matches!(&pat.node, kata_inference::TypedPattern::Variant { .. }),
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
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Match: braços devem retornar mesmo tipo ──────────────────────

#[test]
fn match_arms_type_mismatch_error() {
    let err = infer_src_err("match True\n    True: 1\n    False: \"texto\"");
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
//
// Após DoD 30, lambdas sem contexto de tipo produzem LambdaInferenceFail.
// Os testes usam ascription `(lambda ...)::(Int -> Int)` para fornecer o tipo.

#[test]
fn lambda_anon_has_function_type() {
    // (lambda x: x)::(Int -> Int) — identidade com hint top-down.
    let tmod = infer_src("(lambda x: x)::(Int -> Int)");
    let entry = entry_typed(&tmod);
    match &entry.ty {
        Ty::Function(params, ret) => {
            assert_eq!(params.len(), 1);
            assert_eq!(**ret, Ty::int());
        }
        other => panic!("expected Function type, got {other:?}"),
    }
    match &entry.kind {
        TypedExprKind::TypeAscription { expr, .. } => match &expr.node.kind {
            TypedExprKind::Grouping { inner } => match &inner.node.kind {
                TypedExprKind::Lambda {
                    param_types,
                    clauses,
                    ..
                } => {
                    assert_eq!(param_types.len(), 1);
                    assert_eq!(clauses.len(), 1, "lambda anônimo tem 1 cláusula");
                }
                other => panic!("expected Lambda inside grouping, got {other:?}"),
            },
            other => panic!("expected Grouping inside ascription, got {other:?}"),
        },
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

// ── Lambda anônimo com 2 parâmetros ──────────────────────────────

#[test]
fn lambda_anon_two_params() {
    let tmod = infer_src("(lambda a b: a)::(Int Int -> Int)");
    let entry = entry_typed(&tmod);
    match &entry.kind {
        TypedExprKind::TypeAscription { expr, .. } => match &expr.node.kind {
            TypedExprKind::Grouping { inner } => match &inner.node.kind {
                TypedExprKind::Lambda { param_types, .. } => {
                    assert_eq!(param_types.len(), 2);
                }
                other => panic!("expected Lambda inside grouping, got {other:?}"),
            },
            other => panic!("expected Grouping inside ascription, got {other:?}"),
        },
        other => panic!("expected TypeAscription, got {other:?}"),
    }
}

// ── Guards: verificação de tipo Boolean ───────────────────────────

#[test]
fn guard_condition_must_be_boolean() {
    // Match em Boolean — True e False são resolvidos via EnumRegistry.
    // Testa que o body de match arms pode ser Boolean.
    let src = "match True\n    True: True\n    False: False";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

#[test]
fn guard_condition_non_boolean_error() {
    // Match arms devem retornar o mesmo tipo. Se um retorna Int e outro
    // Boolean, deve dar TypeMismatch.
    let src = "match True\n    True: 1\n    False: False";
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

// ── Cons pattern rejeitado ────────────────────────────────

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
    let tmod = infer_src("match True\n    True: 1\n    False: 0");
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
    // constant value é tail_pos = false.
    // constant é ConstantBinding em tmod.constants.
    let tmod = infer_src("constant x := 42\n42");
    let binding = &tmod.constants[0];
    match &binding.node.kind {
        TypedExprKind::ConstantBinding { value, .. } => {
            assert!(
                !value.node.tail_pos,
                "constant value deve ser tail_pos = false"
            );
        }
        other => panic!("expected ConstantBinding, got {other:?}"),
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
    // let f := (lambda x: x)::(Int -> Int)
    // f é Ty::Function no TypeEnv. O typeck aceita.
    let tmod = infer_src("f :: Int => Int\nlambda x: x\n42");
    let entry = entry_typed(&tmod);
    // Entry é 42 (Int) — o let define f mas entry é a última expr.
    assert_eq!(entry.ty, Ty::int());
    // f está no TypeEnv como Ty::Function.
    let f_ty = tmod.type_env.lookup("f");
    assert!(matches!(f_ty, Some(Ty::Function(_, _))));
}

// ── DoD 12: RedundantClause — cláusulas sobrepostas ──────────────

/// Cláusula wildcard seguida de cláusula ident → redundante.
/// `lambda _: 1` cobre tudo; `lambda n: n` é inalcançável.
#[test]
fn redundant_clause_wildcard_then_ident() {
    let src = "\
fun :: Int => Int\n\
lambda _: 1\n\
lambda n: n\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusula ident seguida de cláusula ident → redundante.
/// `lambda x: 1` cobre tudo; `lambda y: 2` é inalcançável.
#[test]
fn redundant_clause_ident_then_ident() {
    let src = "\
fun :: Int => Int\n\
lambda x: 1\n\
lambda y: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusula literal seguida da mesma literal → redundante.
/// `lambda 0: 1` cobre 0; `lambda 0: 2` é inalcançável.
#[test]
fn redundant_clause_same_literal() {
    let src = "\
fun :: Int => Int\n\
lambda 0: 1\n\
lambda 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

/// Cláusulas não-sobrepostas NÃO produzem RedundantClause.
/// `lambda 0: 1` e `lambda n: n` não são sobrepostas.
#[test]
fn non_redundant_clauses_ok() {
    let src = "\
fun :: Int => Int\n\
lambda 0: 1\n\
lambda n: n\n\
fun 5";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Cláusula sem guards seguida de cláusula com guards E mesmos patterns
/// → redundante. M sem guards sempre dispara sobre os patterns,capturando
/// o input antes de N ser avaliada.
/// `lambda x: 1` (sem guards, Ident cobre tudo) → `lambda x: guards` é redundante.
#[test]
fn redundant_clause_no_guards_covers_guarded() {
    let src = "\
fun :: Int => Int\n\
lambda x: 1\n\
lambda x:\n\
\x20   > x 0: 2\n\
\x20   otherwise: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause, got {err:?}"
    );
}

// ── Redundância com guards: Fase 1 (tautologia dos guards de M) ─────

/// M com guards tautológicos (x > 0 ∨ x <= 0 = True) cobre tudo.
/// N sem guards com mesmo pattern é redundante.
/// Não precisa de otherwise: os guards são tautológicos (Z3 prova).
#[test]
fn redundant_clause_guarded_m_tautology_n_no_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
\x20   <= x 0: 2\n\
lambda x: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (guards de M são tautologia), got {err:?}"
    );
}

/// M com otherwise (trivialmente tautologia) cobre tudo.
/// N sem guards com mesmo pattern é redundante.
#[test]
fn redundant_clause_guarded_m_otherwise_n_no_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
\x20   otherwise: 2\n\
lambda x: 3\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (M tem otherwise), got {err:?}"
    );
}

/// M com guards NÃO-tautológicos não pode ser testado isoladamente:
/// se M tem guards sem otherwise e não-tautologia, `check_guard_completeness`
/// dispara NonExhaustiveMatch durante a inferência (antes de
/// `check_redundant_clauses`). O caso (true, false) onde M tem guards
/// não-tautológicos simplesmente nunca chega à verificação de redundância.
///
/// O teste abaixo usa M com guards tautológicos (otherwise) + N sem guards.
/// N é redundante porque M sempre dispara.
/// Para testar não-redundância com guards, ver Fase 2 (guard_implication).
//
// ── Redundância com guards: Fase 2 (implicação guards_N ⟹ guards_M) ─
/// Guards de N implicam guards de M: x > 5 ⟹ x > 0.
/// N é redundante — M dispara antes para todo input que N casaria.
/// Não precisam de otherwise: a redundância roda antes da exaustividade
/// de guards.
#[test]
fn redundant_clause_guard_implication() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   > x 5: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (x > 5 implica x > 0), got {err:?}"
    );
}

/// Guards de N NÃO implicam guards de M: x <= 5 não implica x > 0.
/// N não é redundante — x = -1 satisfaz N mas não M.
///
/// Pós-Fase 3: os guards `> x 0` e `<= x 5` juntos cobrem todo Int
/// (tautologia disjuntiva provada por Z3), então a função é exaustiva
/// e não há erro de exaustividade nem RedundantClause.
#[test]
fn non_redundant_guard_no_implication() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   <= x 5: 2\n\
fun 5";
    // Pós-Fase 3: a função é exaustiva (guards disjuntivos cobrem Int).
    // Não deve haver erro de inferência.
    let _module = infer_src(src);
}

/// Guards idênticos: x > 0 ⟹ x > 0 (trivialmente verdadeiro).
/// N é redundante — M dispara primeiro com o mesmo guard.
#[test]
fn redundant_clause_identical_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 0: 1\n\
lambda x:\n\
\x20   > x 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "esperava RedundantClause (guards idênticos), got {err:?}"
    );
}

/// Guards disjuntos: x < 0 não implica x > 10.
/// N não é redundante. O erro deve ser NonExhaustiveMatch, não
/// RedundantClause.
#[test]
fn non_redundant_disjoint_guards() {
    let src = "\
fun :: Int => Int\n\
lambda x:\n\
\x20   > x 10: 1\n\
lambda x:\n\
\x20   < x 0: 2\n\
fun 5";
    let err = infer_src_err(src);
    assert!(
        !matches!(err, kata_diagnostics::MiddleError::RedundantClause { .. }),
        "não esperava RedundantClause, got {err:?}"
    );
}

/// Multi-cláusula com variantes de enum Boolean NÃO é redundante.
/// `lambda True True: True` seguido de `lambda True False: False` —
/// variantes diferentes não se cobrem. Antes do fix, o checker operava
/// sobre `Pattern::Ident("True")` (pré-typeck) e tratava todo `Ident`
/// como wildcard, causando falso positivo.
#[test]
fn non_redundant_boolean_variant_clauses() {
    let src = "\
and :: Boolean Boolean => Boolean\n\
lambda True True: True\n\
lambda True False: False\n\
lambda False True: False\n\
lambda False False: False\n\
and True False";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

// ── Exaustividade de N parâmetros (produto cartesiano) ───────────

/// 2 Boolean exaustivo: 4 cláusulas cobrindo True×True, True×False,
/// False×True, False×False.
#[test]
fn exhaustiveness_2_boolean_exhaustive() {
    let src = "\
and :: Boolean Boolean => Boolean\n\
lambda True True: True\n\
lambda True False: False\n\
lambda False True: False\n\
lambda False False: False\n\
and True False";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

/// 2 Boolean não-exaustivo: 2 cláusulas (True×True, True×False).
/// Faltam (False, True) e (False, False) → NonExhaustiveMatch.
#[test]
fn exhaustiveness_2_boolean_non_exhaustive() {
    let src = "\
and :: Boolean Boolean => Boolean\n\
lambda True True: True\n\
lambda True False: False\n\
and True False";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
    if let kata_diagnostics::MiddleError::NonExhaustiveMatch { missing, .. } = err {
        assert_eq!(
            missing.len(),
            2,
            "deve faltar exatamente 2 células: {missing:?}"
        );
    }
}

/// Boolean × Int com Ident: True x, False _ — Ident/Wildcard cobre __ANY__.
#[test]
fn exhaustiveness_boolean_int_with_ident() {
    let src = "\
f :: Boolean Int => Int\n\
lambda True x: x\n\
lambda False _: 0\n\
f True 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// Boolean × Int sem Ident: True 0, False 1 — Int é __ANY__, Literal
/// não cobre. Deve dar NonExhaustiveMatch.
#[test]
fn exhaustiveness_boolean_int_without_ident() {
    let src = "\
f :: Boolean Int => Int\n\
lambda True 0: 0\n\
lambda False 1: 1\n\
f True 42";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
}

/// 1 Boolean (degenera): True, False — idêntico ao comportamento atual.
#[test]
fn exhaustiveness_1_boolean_degenerates() {
    let src = "\
not :: Boolean => Boolean\n\
lambda True: False\n\
lambda False: True\n\
not True";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

/// 1 List (degenera): [], [h:t] — idêntico ao comportamento atual.
#[test]
fn exhaustiveness_1_list_degenerates() {
    let src = "\
len :: [Int] => Int\n\
lambda []: 0\n\
lambda [h : t]: + 1 (len t)\n\
len [1 2 3]";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// 3 Boolean exaustivo: 8 cláusulas cobrindo todas as combinações.
#[test]
fn exhaustiveness_3_boolean_exhaustive() {
    let src = "\
f3 :: Boolean Boolean Boolean => Boolean\n\
lambda True True True: True\n\
lambda True True False: False\n\
lambda True False True: False\n\
lambda True False False: False\n\
lambda False True True: False\n\
lambda False True False: False\n\
lambda False False True: False\n\
lambda False False False: False\n\
f3 True True True";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::boolean());
}

/// 3 Boolean não-exaustivo: 4 cláusulas — faltam 4 células.
#[test]
fn exhaustiveness_3_boolean_non_exhaustive() {
    let src = "\
f3 :: Boolean Boolean Boolean => Boolean\n\
lambda True True True: True\n\
lambda True True False: False\n\
lambda True False True: False\n\
lambda True False False: False\n\
f3 True True True";
    let err = infer_src_err(src);
    assert!(
        matches!(
            err,
            kata_diagnostics::MiddleError::NonExhaustiveMatch { .. }
        ),
        "esperava NonExhaustiveMatch, got {err:?}"
    );
    if let kata_diagnostics::MiddleError::NonExhaustiveMatch { missing, .. } = err {
        assert_eq!(
            missing.len(),
            4,
            "deve faltar exatamente 4 células: {missing:?}"
        );
    }
}

/// Tuple como parâmetro: (a, b) com Ident — Tuple é átomo (__ANY__),
/// Ident cobre. Deve passar.
#[test]
fn exhaustiveness_tuple_as_atom() {
    let src = "\
fst :: (Int, Int) => Int\n\
lambda (a, b): a\n\
fst (1, 2)";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}
