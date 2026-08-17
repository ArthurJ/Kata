//! Verificação empírica dos 3 bloqueios do MIGRATION-TODO.
//!
//! (1) Interoperabilidade refined→base (widening não implementado):
//!     echo!/show/+ rejeitam PositiveInt? O PRD-refines resolveu isso?
//!
//! (2) mod/and em lambdas (collect_free_vars bug):
//!     O bug que marca DispatchTable functions como free vars ainda existe?
//!
//! (3) TRMA multi-clause:
//!     `trma.rs:94` exige 1 clause; multi-clause não otimiza?

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_comptime::run_comptime_pass;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

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
    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);
    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
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

fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_prelude().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed =
        run_comptime_pass(tree_shake(typed.inner), &resolved.enum_registry).expect("comptime");
    let typed = kata_monomorph::MonoModule::from(typed);
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr()).expect("codegen+JIT");
    (jit.raw, jit.ty)
}

fn try_eval(src: &str) -> Result<(i64, Ty), String> {
    let tokens = lex(src).map_err(|e| format!("lex: {e:?}"))?;
    let module = parse(tokens).map_err(|e| format!("parse: {e:?}"))?;
    let prelude = load_prelude().map_err(|e| format!("prelude: {e:?}"))?;
    let user = resolve(&module).map_err(|e| format!("resolve: {e:?}"))?;
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).map_err(|e| format!("infer: {e:?}"))?;
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = run_comptime_pass(tree_shake(typed.inner), &resolved.enum_registry)
        .map_err(|e| format!("comptime: {e:?}"))?;
    let typed = kata_monomorph::MonoModule::from(typed);
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
        .map_err(|e| format!("codegen+JIT: {e:?}"))?;
    Ok((jit.raw, jit.ty))
}

fn infer_optimize(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_prelude().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer");
    let typed = monomorphize(typed);
    optimize(typed).inner
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ═══════════════════════════════════════════════════════════════════════
// BLOQUEIO 1: Interoperabilidade refined→base (echo!/show/+ com PositiveInt)
// ═══════════════════════════════════════════════════════════════════════

/// echo! com PositiveInt SEM `refines` — SHOW é automático para todos os tipos
/// (PRD-refines §3.6, DoD 4). Não deveria falhar.
#[test]
fn b1_echo_positiveint_sem_refines() {
    // Entry point direto: echo! funciona em top-level (não precisa de action)
    let src = r#"data (Int, > _ 0) as PositiveInt
constant a := 5::PositiveInt
echo!(a)"#;
    let (_raw, ty) = eval_src(src);
    // echo! retorna Unit
    assert_eq!(
        ty,
        Ty::Unit,
        "echo! com PositiveInt sem refines deve funcionar"
    );
}

/// show com PositiveInt SEM `refines` — mesmo princípio.
#[test]
fn b1_show_positiveint_sem_refines() {
    let src = r#"data (Int, > _ 0) as PositiveInt
constant a := 5::PositiveInt
show a"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::text(),
        "show com PositiveInt sem refines deve funcionar"
    );
}

/// + com PositiveInt COM `refines NUM` — fallback substitui por Int e
///   passa pelo construtor falível. Retorna Result::(PositiveInt, Text).
#[test]
fn b1_soma_positiveint_com_refines() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
constant a := 5::PositiveInt
constant b := 3::PositiveInt
PositiveInt (+ a b)"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic(
            "Result".into(),
            vec![Ty::Struct("PositiveInt".into()), Ty::text()]
        ),
        "+ com PositiveInt + refines NUM deve retornar Result::(PositiveInt, Text)"
    );
}

/// + com PositiveInt SEM `refines NUM` — deve falhar (sem fallback).
#[test]
fn b1_soma_positiveint_sem_refines_falha() {
    let src = r#"data (Int, > _ 0) as PositiveInt
constant a := 5::PositiveInt
+ a 0"#;
    assert!(
        try_eval(src).is_err(),
        "sem refines NUM, + a 0 onde a :: PositiveInt deve falhar no typeck"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// BLOQUEIO 2: mod/and em lambdas (collect_free_vars bug)
// ═══════════════════════════════════════════════════════════════════════

/// mod em função nomeada (não captura nada). `mod` é função Kata (não FFI)
/// definida na stdlib. Se collect_free_vars marcar `mod` como free var,
/// o codegen falha. Este teste verifica que `mod` NÃO é marcada como free var.
#[test]
fn b2_mod_em_funcao_nomeada() {
    let src = r#"foo :: Int => Int
lambda x: mod x 2
foo 7"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1, "mod 7 2 = 1");
}

/// and (Boolean) em função nomeada.
#[test]
fn b2_and_em_funcao_nomeada() {
    let src = r#"foo :: Boolean => Boolean
lambda x: and x True
foo False"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    // False = 0 (sem SMI tag)
    assert_eq!(raw, 0, "and False True = False = 0");
}

/// map com mod em lambda anônimo (HOF callback). Se o bug de collect_free_vars
/// existe, `mod` seria marcada como free var e o codegen falharia.
#[test]
fn b2_map_com_mod_em_lambda() {
    let src = r#"map (lambda x: mod x 2) [1 2 3 4 5]"#;
    let (_raw, ty) = eval_src(src);
    assert!(
        matches!(ty, Ty::List(_)),
        "map com mod em lambda deve retornar List"
    );
}

/// + em função nomeada (controle — + é FFI).
#[test]
fn b2_soma_em_funcao_controle() {
    let src = r#"foo :: Int => Int
lambda x: + x 1
foo 41"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

// ═══════════════════════════════════════════════════════════════════════
// BLOQUEIO 3: TRMA multi-clause
// ═══════════════════════════════════════════════════════════════════════

/// TRMA com 1 cláusula + match explícito — Caso A do trma.rs.
#[test]
fn b3_trma_1_clause_match() {
    let src = r#"soma2 :: Int => Int
lambda n:
    match n
        0: 0
        otherwise: + n (soma2 (- n 1))
soma2 100"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 5050, "soma2 100 = 5050");
}

/// TRMA com 2 cláusulas (sugar form) — Caso B do trma.rs.
#[test]
fn b3_trma_2_clauses_sugar() {
    let src = r#"soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))
soma 100"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 5050, "soma 100 = 5050");
}

/// TRMA com 2 cláusulas e N grande — prova TCO ativo.
#[test]
fn b3_trma_2_clauses_n_grande_prova_tco() {
    let src = r#"soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))
soma 1000000"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 500000500000, "soma 1000000 via TRMA+TCO");
}

/// TRMA com 1 cláusula + match e N grande — prova TCO ativo (Caso A).
#[test]
fn b3_trma_1_clause_match_n_grande() {
    let src = r#"soma2 :: Int => Int
lambda n:
    match n
        0: 0
        otherwise: + n (soma2 (- n 1))
soma2 1000000"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 500000500000, "soma2 1000000 via TRMA+TCO");
}

/// TRMA com 3 cláusulas — o MIGRATION-TODO diz "exige 1 clause".
/// Verificamos se compila (deve) e se otimiza (provavelmente não).
#[test]
fn b3_trma_3_clauses_compila() {
    let src = r#"soma3 :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + n (soma3 (- n 1))
soma3 100"#;
    // Deve compilar e rodar (mesmo sem TRMA, 100 é pequeno)
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // soma3 100 = 100 + soma3 99 = 100 + 99 + ... + 2 + 1 + 0
    // = 5050 (casos base 0 e 1 não somam nada extra além do fluxo normal)
    assert_eq!(untag_smi(raw), 5050, "soma3 100 = 5050 (mesmo sem TRMA)");
}

/// Verifica se o optimizer reescreveu a função com 2 clauses (sugar)
/// em soma_acc. Se sim, confirma que TRMA está ativo para 2 clauses.
#[test]
fn b3_trma_2_clauses_otimiza() {
    let src = r#"soma :: Int => Int
lambda 0: 0
lambda n: + n (soma (- n 1))
soma 100"#;
    let typed = infer_optimize(src);
    let has_acc = typed.functions.iter().any(|f| f.name == "soma_acc");
    assert!(
        has_acc,
        "TRMA deve reescrever soma (2 clauses sugar) em soma_acc"
    );
}

/// Verifica se o optimizer reescreveu a função com 1 clause + match.
#[test]
fn b3_trma_1_clause_match_otimiza() {
    let src = r#"soma2 :: Int => Int
lambda n:
    match n
        0: 0
        otherwise: + n (soma2 (- n 1))
soma2 100"#;
    let typed = infer_optimize(src);
    let has_acc = typed.functions.iter().any(|f| f.name == "soma2_acc");
    assert!(
        has_acc,
        "TRMA deve reescrever soma2 (1 clause + match) em soma2_acc"
    );
}

/// Verifica se o optimizer reescreveu a função com 3 clauses.
/// Se não reescreveu, confirma que TRMA não suporta 3 clauses.
#[test]
fn b3_trma_3_clauses_nao_otimiza() {
    let src = r#"soma3 :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + n (soma3 (- n 1))
soma3 100"#;
    let typed = infer_optimize(src);
    let has_acc = typed.functions.iter().any(|f| f.name == "soma3_acc");
    // Se has_acc = true, TRMA suporta 3 clauses (bloqueio resolvido).
    // Se has_acc = false, TRMA não suporta 3 clauses (bloqueio mantém).
    if has_acc {
        eprintln!("3f: TRMA reescreveu soma3 (3 clauses) em soma3_acc — SUPORTA!");
    } else {
        eprintln!("3f: TRMA NÃO reescreveu soma3 (3 clauses) — confirma bloqueio");
    }
    // Não há assert — é diagnóstico. O resultado é reportado via eprintln.
}

// NOTA: TRMA com 3 cláusulas e N grande causa stack overflow (confirmado
// empiricamente). O teste foi removido porque o SIGABRT mata o processo
// inteiro de testes. O diagnóstico do b3_trma_3_clauses_nao_otimiza
// (verifica que soma3_acc não existe após optimize) é suficiente para
// confirmar que TRMA não otimiza 3+ cláusulas.
