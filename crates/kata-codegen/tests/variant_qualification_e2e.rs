//! Testes E2E de qualificação de variantes em posição de expressão.
//!
//! 1. Variantes desqualificadas (`Ok 42`, `True`, `None`) funcionam quando
//!    exatamente 1 enum no escopo tem aquela variante.
//! 2. Quando múltiplos enums têm a mesma variante, a forma desqualificada
//!    falha com erro de ambiguidade — qualificação (`Result::Ok`) é obrigatória.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

// ── Helpers ───────────────────────────────────────────────────────

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Executa o pipeline até inferência e retorna o erro (espera falhar).
fn infer_src_err(src: &str) -> MiddleError {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Construtor desqualificado em expressão ─────────────────────────

/// `Ok 42` em posição de expressão — sem qualificar `Result::Ok`.
/// `Ok` só existe em `Result` (único enum no prelude com essa variante),
/// então `resolve_unqual_variant` encontra exatamente 1 candidato.
#[test]
fn unqualified_ok_in_expression_builds_result() {
    let src = r#"action main => Int
    let r := Ok 42
    match r
        Ok v: v
        Err _: 0
        otherwise: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// `Err 99` desqualificado em expressão.
#[test]
fn unqualified_err_in_expression_builds_result() {
    let src = r#"action main => Int
    let r := Err 99
    match r
        Ok v: v
        Err _: 0
        otherwise: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// `True` desqualificado em expressão — `Boolean` é o único enum com `True`.
#[test]
fn unqualified_true_in_expression() {
    let src = r#"action main => Int
    let b := True
    match b
        True: 1
        False: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1);
}

/// `False` desqualificado em expressão.
#[test]
fn unqualified_false_in_expression() {
    let src = r#"action main => Int
    let b := False
    match b
        True: 1
        False: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0);
}

/// `Some 7` e `None` desqualificados em expressão.
#[test]
fn unqualified_some_none_in_expression() {
    let src = r#"action main => Int
    let a := Some 7
    let b := None
    match a
        Some n: n
        None: match b
            Some m: m
            None: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7);
}

// ── Ambiguidade exige qualificação ─────────────────────────────────

/// Dois enums com variante `Ok` no escopo → `Ok 42` desqualificado é ambíguo.
/// `Result` (prelude) e `MeuResult` (usuário) ambos têm `Ok`.
/// Erro esperado: `UnboundName` com mensagem de ambiguidade.
#[test]
fn ambiguous_variant_requires_qualification() {
    let src = r#"enum MeuResult
    Ok(Int)
    Falhou

action main => Int
    let r := Ok 42
    match r
        Ok v: v
        Falhou: 0
main!()"#;
    let err = infer_src_err(src);
    assert!(
        matches!(err, MiddleError::UnboundName { .. }),
        "esperado UnboundName (ambiguidade), recebido: {err:?}"
    );
    // A mensagem deve mencionar ambiguidade e listar os enums conflitantes.
    if let MiddleError::UnboundName { name, .. } = err {
        assert!(
            name.contains("ambígua"),
            "mensagem deve mencionar ambiguidade: {name}"
        );
        assert!(
            name.contains("Result"),
            "mensagem deve listar Result: {name}"
        );
        assert!(
            name.contains("MeuResult"),
            "mensagem deve listar MeuResult: {name}"
        );
    }
}

/// Mesmo cenário ambíguo, mas qualificando `Result::Ok` → funciona.
#[test]
fn qualified_variant_resolves_ambiguity() {
    let src = r#"enum MeuResult
    Ok(Int)
    Falhou

action main => Int
    let r := Result::Ok 42
    match r
        Ok v: v
        Err _: 0
        otherwise: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// Dois enums com variante `Ok`, qualificando `MeuResult::Ok` → funciona.
#[test]
fn qualified_custom_enum_variant_resolves() {
    let src = r#"enum MeuResult
    Ok(Int)
    Falhou

action main => Int
    let r := MeuResult::Ok 42
    match r
        MeuResult::Ok v: v
        MeuResult::Falhou: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}
