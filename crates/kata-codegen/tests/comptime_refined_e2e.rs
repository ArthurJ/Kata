//! Testes E2E da Fase 4: ascription refined com predicados complexos delegados
//! ao comptime pass.
//!
//! Pipeline: lex → parse → resolve → infer → monomorph → optimize → tree_shake
//!           → comptime_pass → codegen → JIT
//!
//! DoDs cobertos:
//! - `5::Prime` com `is_prime` definido → valida em compile-time via JIT
//! - `4::Prime` → erro de compilação (predicado falha no comptime pass)
//! - Predicado trivial (`> _ 0`) continua validando localmente no typeck

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_comptime::run_comptime_pass;
use kata_core::StructKey;
use kata_core::ty::Ty;
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

/// Pipeline completo com comptime pass — retorna (raw, ty) ou erro.
fn eval_with_comptime(src: &str) -> Result<(i64, Ty), String> {
    let tokens = lex(src).map_err(|e| format!("lex: {e:?}"))?;
    let module = parse(tokens).map_err(|e| format!("parse: {e:?}"))?;
    let prelude = load_prelude().map_err(|e| format!("prelude: {e:?}"))?;
    let user = resolve(&module).map_err(|e| format!("resolve: {e:?}"))?;
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).map_err(|e| format!("infer: {e:?}"))?;
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let typed = run_comptime_pass(typed.inner, &resolved.enum_registry)
        .map_err(|e| format!("comptime: {e:?}"))?;
    let typed = kata_monomorph::MonoModule::from(typed);
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .map_err(|e| format!("codegen: {e:?}"))?;
    Ok((jit.raw, jit.ty))
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// `is_prime` simples — predicado complexo que o const_eval não consegue
/// avaliar (envolve chamada de função). O comptime pass deve JIT-executar.
///
/// is_prime :: Int => Boolean
/// lambda 0: False
/// lambda 1: False
/// lambda 2: True
/// lambda 3: True
/// lambda 4: False
/// lambda 5: True
/// lambda _: False
const IS_PRIME_SRC: &str = "\
data (Int, is_prime _) as Prime
is_prime :: Int => Boolean
lambda 0: False
lambda 1: False
lambda 2: True
lambda 3: True
lambda 4: False
lambda 5: True
lambda _: False
";

/// DoD Fase 4: `5::Prime` valida em compile-time via JIT (predicado complexo).
///
/// O typeck não consegue avaliar `is_prime _` localmente (const_eval retorna None).
/// O predicado é tipado, armazenado como pending, e o comptime pass o JIT-executa.
/// Resultado: True → ascription válida.
#[test]
fn ascription_refined_predicado_complexo_passa() {
    let src = format!("{IS_PRIME_SRC}\n5::Prime");
    let (raw, ty) = eval_with_comptime(&src).expect("deve compilar e executar");
    assert_eq!(ty, Ty::Struct(StructKey::Plain("Prime".into())));
    assert_eq!(untag_smi(raw), 5);
}

/// DoD Fase 4: `4::Prime` falha — predicado complexo retorna False.
///
/// O comptime pass JIT-executa `is_prime 4`, obtém False (tag 0),
/// e emite erro.
#[test]
fn ascription_refined_predicado_complexo_falha() {
    let src = format!("{IS_PRIME_SRC}\n4::Prime");
    let result = eval_with_comptime(&src);
    assert!(
        result.is_err(),
        "4::Prime deve falhar — is_prime 4 retorna False"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("predicado") || err.contains("comptime") || err.contains("falhou"),
        "erro deve mencionar predicado/falha: {err}"
    );
}

/// Invariante Fio 6: predicado trivial continua validando localmente no typeck.
/// `5::PositiveInt` (predicado `> _ 0`) não precisa do comptime pass — o const_eval
/// resolve localmente. Este teste não chama run_comptime_pass.
#[test]
fn predicado_trivial_continua_local() {
    let src = "data (Int, > _ 0) as PositiveInt\n5::PositiveInt";
    let tokens = lex(src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_prelude().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    // Sem comptime pass — predicado trivial já foi validado no typeck.
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false).expect("codegen");
    assert_eq!(jit.ty, Ty::Struct(StructKey::Plain("PositiveInt".into())));
    assert_eq!(untag_smi(jit.raw), 5);
}

/// Predicado trivial que falha também continua funcionando sem comptime pass.
/// `(-5)::PositiveInt` → type error no typeck (const_eval retorna Some(false)).
#[test]
fn predicado_trivial_falha_local() {
    let src = "data (Int, > _ 0) as PositiveInt\n(-5)::PositiveInt";
    let tokens = lex(src).expect("lex");
    let module = parse(tokens).expect("parse");
    let prelude = load_prelude().expect("prelude");
    let user = resolve(&module).expect("resolve");
    let resolved = merge_resolved(prelude, user);
    let result = infer_module(&module, &resolved);
    assert!(result.is_err(), "(-5)::PositiveInt deve falhar no typeck");
}
