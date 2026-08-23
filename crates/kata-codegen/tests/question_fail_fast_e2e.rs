//! Testes E2E de codegen de `?` fail-fast.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoD 20-21: `?` desempacota Result/Optional, aborta em Err/None,
//! e é desugared para Match + Return (TAST nunca contém Question).

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Executa o pipeline completo e retorna a TAST (para verificar desugar).
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// Combina prelude + módulo do usuário (replica do driver).
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Conta nós Match na TAST recursivamente (o `?` desugar produz Match).
fn count_match(expr: &kata_inference::TypedExpr) -> usize {
    use kata_inference::TypedExprKind;
    match &expr.kind {
        TypedExprKind::Match { scrutinee, arms } => {
            let mut c = 1 + count_match(&scrutinee.node);
            for arm in arms {
                c += count_match(&arm.body.node);
            }
            c
        }
        TypedExprKind::Return(inner) => count_match(&inner.node),
        TypedExprKind::Let { value, .. } => count_match(&value.node),
        _ => 0,
    }
}

/// Conta nós Match em todos os statements de todas as actions.
fn count_match_in_module(typed: &kata_inference::TypedModule) -> usize {
    let mut total = 0;
    for action in &typed.actions {
        for stmt in &action.body {
            total += count_match(&stmt.node);
        }
    }
    total
}

// ── DoD 20: `?` desempacota Ok e continua ──────────────────

/// DoD 20: `?` em `Ok 42` desempacota o valor (42) e continua.
/// A action retorna `Result::(Int, Text)` — o `?` desempacota, e o body
/// continua com `Ok 0`. O resultado final é `Ok(0)`.
/// E|Text é o default do prelude (`Err(E|Text)`).
#[test]
fn question_desempacota_result_ok() {
    let src = r#"action extrai => Result::(Int, Text)
    let r := Ok 42
    r ?
    Ok 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    // O tipo de retorno é Result::(Int, Text) = Generic
    assert_eq!(
        ty,
        Ty::Generic(
            "Result".into(),
            vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]
        )
    );
    // O valor é um Sum (ponteiro) — verificamos que não é SMI
    // (bit 0 = 0 indica ponteiro; SMI tem bit 0 = 1)
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

/// DoD 20: `?` em `Err 99` aborta com `return Err(99)`.
/// A action retorna `Result::(Int, Text)` — o `?` aborta, o body
/// não continua. O resultado final é `Err(99)`.
/// E|Text é o default do prelude (`Err(E|Text)`).
#[test]
fn question_aborta_result_err() {
    let src = r#"action extrai => Result::(Int, Text)
    let r := Err 99
    r ?
    Ok 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic(
            "Result".into(),
            vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]
        )
    );
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

// ── DoD 20: `?` desempacota Some e aborta em None ─────────

/// DoD 20: `?` em `Some 42` desempacota o valor e continua.
#[test]
fn question_desempacota_optional_some() {
    let src = r#"action extrai => Optional::(Int)
    let r := Some 42
    r ?
    None
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic("Optional".into(), vec![Ty::Prim(PrimTy::Int)])
    );
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

/// DoD 20: `?` em `None` aborta com `return None`.
#[test]
fn question_aborta_optional_none() {
    let src = r#"action extrai => Optional::(Int)
    let r := None
    r ?
    Some 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic("Optional".into(), vec![Ty::Prim(PrimTy::Int)])
    );
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}

// ── DoD 21: TAST nunca contém Question (desugar para Match) ────────

/// DoD 21: `?` é desugared para Match + Return no typeck.
/// A TAST nunca contém `Question` — sempre `Match`.
#[test]
fn question_desugared_para_match_na_tast() {
    let src = r#"action extrai => Result::(Int, Text)
    let r := Ok 42
    r ?
    Ok 0
extrai!()"#;
    let typed = infer_src(src);
    let match_count = count_match_in_module(&typed);
    assert!(
        match_count >= 1,
        "TAST deve conter pelo menos 1 Match (desugar de ?)"
    );
}

// ── Testes de infraestrutura (match explícito com generics do prelude) ──

/// DoD 20 (infra): Match em Ok do prelude dentro de Action.
#[test]
fn match_result_ok_dentro_de_action() {
    let src = r#"action extrai => Int
    match Ok 42
        Ok v: v
        Err e: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 20 (infra): Match em Some do prelude dentro de Action.
#[test]
fn match_optional_some_dentro_de_action() {
    let src = r#"action extrai => Int
    match Some 42
        Some v: v
        None: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// DoD 20 (infra): Match em Err do prelude dentro de Action.
#[test]
fn match_result_err_dentro_de_action() {
    let src = r#"action extrai => Int
    match Err 99
        Ok v: v
        Err e: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// DoD 20 (infra): Match em None do prelude dentro de Action.
#[test]
fn match_optional_none_dentro_de_action() {
    let src = r#"action extrai => Int
    match None
        Some v: v
        None: 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}
