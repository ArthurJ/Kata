//! Testes E2E de codegen de tipos refinados, smart constructors falíveis,
//! e ascription-refined.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//!
//! DoDs cobertos:
//! - DoD 1: `5::PositiveInt` é `PositiveInt` direto (ascription-refined)
//! - DoD 2: `(-5)::PositiveInt` é type error (predicado falha)
//! - DoD 3: `PositiveInt 25 ?` desempacota (construtor falível + `?`)
//! - DoD 4: `PositiveInt (-5)` retorna `Result::Err` (construtor falível)

use kata_codegen::jit_eval;
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
    }
}

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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn infer_fails(src: &str) -> bool {
    let tokens = match lex(src) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let module = match parse(tokens) {
        Ok(m) => m,
        Err(_) => return true,
    };
    let prelude = match load_prelude() {
        Ok(p) => p,
        Err(_) => return true,
    };
    let user = match resolve(&module) {
        Ok(u) => u,
        Err(_) => return true,
    };
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).is_err()
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// DoD 1: `5::PositiveInt` é `PositiveInt` direto.
/// Ascription-refined valida predicado `> _ 0` em compile-time, entrega
/// `Ty::Struct("PositiveInt")` sem `Result`.
#[test]
fn ascription_refined_valida_predicado() {
    let src = "data (Int, > _ 0) as PositiveInt\n5::PositiveInt";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("PositiveInt".into()));
    assert_eq!(untag_smi(raw), 5);
}

/// DoD 2: `(-5)::PositiveInt` é type error.
/// Predicado `> _ 0` falha para -5 em compile-time.
#[test]
fn ascription_refined_predicado_falha_type_error() {
    let src = "data (Int, > _ 0) as PositiveInt\n(-5)::PositiveInt";
    assert!(
        infer_fails(src),
        "(-5)::PositiveInt deve falhar (predicado > _ 0 não satisfeito)"
    );
}

/// DoD 1 com Float: `17.5::Positivo` valida predicado `> _ 0.0` sobre Float.
#[test]
fn ascription_refined_float_valida() {
    let src = "data (Float, > _ 0.0) as Positivo\n17.5::Positivo";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("Positivo".into()));
    let bits = f64::to_bits(17.5) as i64;
    assert_eq!(raw, bits);
}

/// DoD 2 com Float: `(-1.0)::Positivo` falha.
#[test]
fn ascription_refined_float_falha() {
    let src = "data (Float, > _ 0.0) as Positivo\n(-1.0)::Positivo";
    assert!(
        infer_fails(src),
        "(-1.0)::Positivo deve falhar (predicado > _ 0.0 não satisfeito)"
    );
}

/// DoD 3: Smart constructor falível com valor válido retorna `Result::Ok`.
/// `match PositiveInt 25 { Result::Ok v: v, Result::Err _: 42::PositiveInt }` → 25.
/// Ambos os braços produzem `PositiveInt` — sem widening, o match exige
/// o mesmo tipo em todos os braços.
#[test]
fn smart_constructor_falivel_ok_match_extrai_valor() {
    let src = "data (Int, > _ 0) as PositiveInt\nmatch PositiveInt 25\n    Result::Ok v: v\n    Result::Err _: 42::PositiveInt";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("PositiveInt".into()));
    assert_eq!(untag_smi(raw), 25);
}

/// DoD 4: Smart constructor falível com valor inválido retorna `Result::Err`.
/// `match PositiveInt (-5) { Result::Ok v: v, Result::Err _: 42::PositiveInt }` → 42.
/// Ambos os braços produzem `PositiveInt`.
#[test]
fn smart_constructor_falivel_err_match_cai_no_fallback() {
    let src = "data (Int, > _ 0) as PositiveInt\nmatch PositiveInt (-5)\n    Result::Ok v: v\n    Result::Err _: 42::PositiveInt";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("PositiveInt".into()));
    assert_eq!(untag_smi(raw), 42);
}

/// Smart constructor falível com valor válido retorna Ok.
/// `match PositiveInt 42 { Result::Ok v: v, Result::Err _: 42::PositiveInt }` → 42.
#[test]
fn smart_constructor_ok_retorna_tag_zero() {
    let src = "data (Int, > _ 0) as PositiveInt\nmatch PositiveInt 42\n    Result::Ok v: v\n    Result::Err _: 42::PositiveInt";
    let (raw, _ty) = eval_src(src);
    assert_eq!(untag_smi(raw), 42);
}

/// Múltiplos predicados: `data (Int, > _ 0, <= _ 100) as Percentage`.
/// `50::Percentage` passa ambos.
#[test]
fn multiplos_predicados_passa() {
    let src = "data (Int, > _ 0, <= _ 100) as Percentage\n50::Percentage";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Struct("Percentage".into()));
    assert_eq!(untag_smi(raw), 50);
}

/// Múltiplos predicados: `0::Percentage` falha (`> _ 0` não satisfeito).
#[test]
fn multiplos_predicados_falha_primeiro() {
    let src = "data (Int, > _ 0, <= _ 100) as Percentage\n0::Percentage";
    assert!(
        infer_fails(src),
        "0::Percentage deve falhar (predicado > _ 0 não satisfeito)"
    );
}

/// Múltiplos predicados: `150::Percentage` falha (`<= _ 100` não satisfeito).
#[test]
fn multiplos_predicados_falha_segundo() {
    let src = "data (Int, > _ 0, <= _ 100) as Percentage\n150::Percentage";
    assert!(
        infer_fails(src),
        "150::Percentage deve falhar (predicado <= _ 100 não satisfeito)"
    );
}

/// Ascription-refined exige literal — expr não-literal deve dar type error.
#[test]
fn ascription_refined_exige_literal() {
    let src = "data (Int, > _ 0) as PositiveInt\nlet x := 5\nx::PositiveInt";
    assert!(
        infer_fails(src),
        "x::PositiveInt deve falhar (ascription refined exige literal, não variável)"
    );
}

// ── DoDs 9-11: Enum predicado ───────────────────────────────────────

/// DoD 9: `IMC 17.0` despacha para `Magreza`.
/// O construtor sintetizado avalia predicados em runtime e despacha
/// para a variante cujo predicado satisfaz.
#[test]
fn enum_predicado_despacha_magreza() {
    let src = "enum IMC\n    Magreza(< _ 18.5)\n    Normal(<= _ 25.0)\n    Sobrepeso(<= _ 30.0)\n    Obesidade\nmatch IMC 17.0\n    IMC::Magreza x: x\n    IMC::Normal x: x\n    IMC::Sobrepeso x: x\n    IMC::Obesidade x: x";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Float));
    // O payload é Float 17.0 — o match extrai o valor
    let f = f64::from_bits(raw as u64);
    assert!((f - 17.0).abs() < 0.001, "esperado 17.0, got {f}");
}

/// DoD 10: `IMC 22.0` despacha para `Normal`.
#[test]
fn enum_predicado_despacha_normal() {
    let src = "enum IMC\n    Magreza(< _ 18.5)\n    Normal(<= _ 25.0)\n    Sobrepeso(<= _ 30.0)\n    Obesidade\nmatch IMC 22.0\n    IMC::Magreza x: x\n    IMC::Normal x: x\n    IMC::Sobrepeso x: x\n    IMC::Obesidade x: x";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Float));
    let f = f64::from_bits(raw as u64);
    assert!((f - 22.0).abs() < 0.001, "esperado 22.0, got {f}");
}

/// DoD 11: `IMC 35.0` despacha para `Obesidade` (fallback/default).
#[test]
fn enum_predicado_despacha_obesidade() {
    let src = "enum IMC\n    Magreza(< _ 18.5)\n    Normal(<= _ 25.0)\n    Sobrepeso(<= _ 30.0)\n    Obesidade\nmatch IMC 35.0\n    IMC::Magreza x: x\n    IMC::Normal x: x\n    IMC::Sobrepeso x: x\n    IMC::Obesidade x: x";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Float));
    let f = f64::from_bits(raw as u64);
    assert!((f - 35.0).abs() < 0.001, "esperado 35.0, got {f}");
}
