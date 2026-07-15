//! Testes E2E da Fase 8 — `|` fallback (coalescência de erro).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida DoD 22-25: `|` desempacota variantes com payload, avalia direita se
//! a cauda (última variante, unitária). Funciona em funções puras e Actions,
//! é desugared para Match no typeck (TAST nunca contém PipeFallback), e
//! effect = Puro.
//!
//! `|` é generalizado para qualquer enum cujas variantes (exceto a última)
//! carreguem payload e a última seja unitária. Result NÃO é compatível com
//! `|` (Err tem payload) — use `?` para Result. Optional é compatível (None
//! é unitária). User enums com cauda unitária também são compatíveis.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExpr, TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = optimize(typed);
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Executa o pipeline completo e retorna a TAST (para verificar desugar).
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
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
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry: InterfaceRegistry::new(),
        functions: user.functions,
        actions: user.actions,
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Conta nós Match na TAST recursivamente (o `|` desugar produz Match).
fn count_match(expr: &TypedExpr) -> usize {
    match &expr.kind {
        TypedExprKind::Match { scrutinee, arms } => {
            let mut c = 1 + count_match(&scrutinee.node);
            for arm in arms {
                c += count_match(&arm.body.node);
            }
            c
        }
        TypedExprKind::Let { value, .. } => count_match(&value.node),
        TypedExprKind::Return(inner) => count_match(&inner.node),
        _ => 0,
    }
}

/// Conta nós Match em todos os statements de todas as actions E na entry expr.
fn count_match_in_module(typed: &kata_inference::TypedModule) -> usize {
    let mut total = 0;
    for action in &typed.actions {
        for stmt in &action.body {
            total += count_match(&stmt.node);
        }
    }
    total += count_match(&typed.entry.node);
    total
}

// ── DoD 22: `|` desempacota payload e avalia direita se cauda unitária ──

/// DoD 22: `Optional::Some 42 | 99` desempacota Some(42) → 42.
#[test]
fn pipe_fallback_desempacota_optional_some() {
    let src = "Optional::Some 42 | 99";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "Optional::Some 42 | 99 deve ser 42");
}

/// DoD 22: `Optional::None | 99` cai em None (cauda), avalia rhs = 99.
#[test]
fn pipe_fallback_avalia_rhs_em_optional_none() {
    let src = "Optional::None | 99";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 99, "Optional::None | 99 deve ser 99");
}

/// DoD 22: User enum com cauda unitária — desempacota variante com payload.
#[test]
fn pipe_fallback_user_enum_desempacota_variante_com_payload() {
    let src = "enum Light\n    Red(Int)\n    Green(Int)\n    Off\nLight::Red 42 | 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "Light::Red 42 | 0 deve ser 42");
}

/// DoD 22: User enum — cauda unitária ativa fallback.
#[test]
fn pipe_fallback_user_enum_cauda_unitaria_ativa_fallback() {
    let src = "enum Light\n    Red(Int)\n    Green(Int)\n    Off\nLight::Off | 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0, "Light::Off | 0 deve ser 0 (fallback)");
}

/// DoD 22: User enum com 3 variantes — segunda variante (não-cauda) também desempacota.
#[test]
fn pipe_fallback_user_enum_segunda_variante() {
    let src = "enum Light\n    Red(Int)\n    Green(Int)\n    Off\nLight::Green 7 | 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7, "Light::Green 7 | 0 deve ser 7");
}

// ── DoD 23: `|` funciona em funções puras e Actions ─────────────────

/// DoD 23: `|` dentro de Action (com Optional).
#[test]
fn pipe_fallback_dentro_de_action() {
    let src = "action extrai -> Int\n    let r := Optional::Some 42\n    r | 0\nextrai!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "r | 0 dentro de Action deve ser 42");
}

/// DoD 23: `|` dentro de Action com None — avalia fallback.
#[test]
fn pipe_fallback_dentro_de_action_none() {
    let src = "action extrai -> Int\n    let r := Optional::None\n    r | 0\nextrai!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0, "r | 0 com None deve ser 0");
}

// ── DoD 24: `|` desugared para Match no typeck — TAST nunca contém PipeFallback ──

/// DoD 24: `|` é desugared para Match no typeck. A TAST contém Match, não PipeFallback.
#[test]
fn pipe_fallback_desugared_para_match_na_tast() {
    let src = "Optional::Some 42 | 99";
    let typed = infer_src(src);
    let match_count = count_match_in_module(&typed);
    assert!(
        match_count >= 1,
        "TAST deve conter pelo menos 1 Match (desugar de |), encontrados: {match_count}"
    );
}

// ── DoD 25: effect = Puro em `|` fallback ───────────────────────────

/// DoD 25: `|` é pura (coalescência, não aborta). O match sintético tem
/// effect = Puro (infer_match sempre retorna Puro).
/// Verificamos que o tipo do resultado é Int (não aborta, não retorna None).
#[test]
fn pipe_fallback_effect_puro_optional() {
    let src = "Optional::None | 99";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int), "effect deve ser Puro — tipo Int");
    assert_eq!(untag_smi(raw), 99);
}

// ── Result NÃO é compatível com `|` (Err tem payload, não é cauda unitária) ──

/// `Result::Ok 42 | 0` deve ser type error — Result não tem cauda unitária.
#[test]
fn pipe_fallback_result_nao_compativel() {
    let src = "Result::Ok 42 | 0";
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(
        result.is_err(),
        "Result::Ok 42 | 0 deve ser type error — Err tem payload, nao e cauda unitaria"
    );
}
