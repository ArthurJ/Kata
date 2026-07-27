//! Testes E2E do T? — açúcar sintático para `Result::(T, Text)`.
//!
//! `T?` desaçuca para `Result::(T, Text)` em qualquer posição de tipo.
//! Não cria subtyping, não cria Ok implícito, não muda o operador `?`
//! de runtime.
//!
//! DoDs cobertos:
//! - DoD 11: `PositiveInt?` em assinatura ≡ `Result::(PositiveInt, Text)`
//! - DoD 12: `Int` não satisfaz `=> Int?` sem wrap explícito (sem Ok implícito)
//! - DoD 13: `?` em runtime continua sendo desempacotamento de Result

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

/// Tenta o pipeline até infer. Retorna true se infer falhou (erro compile-time).
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

/// Helper: constrói `Result::(T, Text)`.
fn result_text_ty(inner: Ty) -> Ty {
    Ty::Generic("Result".into(), vec![inner, Ty::Prim(PrimTy::Text)])
}

// ── DoD 11: `PositiveInt?` ≡ `Result::(PositiveInt, Text)` ──

/// `T?` em assinatura de action é açúcar para `Result::(T, Text)`.
/// O typeck resolve `PositiveInt?` para `Result::(PositiveInt, Text)`
/// antes de qualquer verificação. O tipo de retorno da action é
/// `Result::(PositiveInt, Text)`.
#[test]
fn t_question_desugar_result_text() {
    let src = r#"data (Int, > _ 0) as PositiveInt
PositiveInt refines NUM
action soma_pos => PositiveInt?
    let a := 5::PositiveInt
    let b := 3::PositiveInt
    PositiveInt (+ a b)
soma_pos!()"#;
    let (_raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        result_text_ty(Ty::Struct("PositiveInt".into())),
        "PositiveInt? deve desaçucar para Result::(PositiveInt, Text)"
    );
}

// ── DoD 12: `Int` não satisfaz `=> Int?` sem wrap explícito ──

/// Sem Ok implícito: uma action que retorna `Int` nu não satisfaz
/// `=> Int?` (que é `=> Result::(Int, Text)`). O typeck deve rejeitar.
#[test]
fn t_question_sem_ok_implicito() {
    let src = r#"action f => Int?
    42
f!()"#;
    assert!(
        infer_fails(src),
        "Int não satisfaz => Int? sem wrap explícito — sem Ok implícito"
    );
}

// ── DoD 13: `?` em runtime desempacota Result ──

/// `?` em runtime desempacota Result. Se Ok, continua executando o body.
/// Se Err, aborta a action com return Err. O `?` não é no-op em não-Result.
/// Este teste verifica que `?` extrai o valor de `Result::Ok 42` e o
/// body continua, retornando `Result::Ok 0`.
#[test]
fn t_question_runtime_desempacota_result() {
    let src = r#"action extrai => Result::(Int, Text)
    let r := Result::Ok 42
    r ?
    Result::Ok 0
extrai!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic(
            "Result".into(),
            vec![Ty::Prim(PrimTy::Int), Ty::Prim(PrimTy::Text)]
        ),
        "? deve desempacotar Result::Ok e o body continua"
    );
    // Result::Ok 0 é um Sum (ponteiro), não SMI
    assert_eq!(raw & 1, 0, "esperado ponteiro (Sum), não SMI");
}
