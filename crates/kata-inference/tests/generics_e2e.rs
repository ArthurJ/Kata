//! Testes E2E de generics (type params do usuário).
//!
//! Verifica que:
//! 1. `id :: T => T` registra type_params=["T"] no OverloadInfo
//! 2. `id 42` despacha via unify: T=Int, retorna Int
//! 3. `id 3.14` despacha via unify: T=Float, retorna Float
//! 4. `id "hello"` despacha via unify: T=Text, retorna Text
//! 5. Múltiplos type params: `pair :: A B => (A, B)` com T e E
//! 6. Type param no retorno: `first :: (T, E) => T`
//! 7. unify detecta inconsistência: `id` com 2 args de tipos diferentes
//! 8. Overload não-genérica vence sobre genérica (exact > generic)

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

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
    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_prelude().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn entry_typed(tmod: &kata_inference::TypedModule) -> &kata_inference::TypedExpr {
    &tmod.entry.node
}

// ── id :: T => T ──────────────────────────────────────────────────

/// `id 42` despacha via unify: T=Int, retorna Int.
#[test]
fn generic_id_int() {
    let src = "id :: T => T\nid 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// `id 3.14` despacha via unify: T=Float, retorna Float.
#[test]
fn generic_id_float() {
    let src = "id :: T => T\nid 3.14";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float());
}

/// `id "hello"` despacha via unify: T=Text, retorna Text.
#[test]
fn generic_id_text() {
    let src = "id :: T => T\nid \"hello\"";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::text());
}

// ── Múltiplos type params ──────────────────────────────────────────

/// `const :: A B => A` — ignora B, retorna A.
/// `const 42 "hello"` → Int
#[test]
fn generic_const_two_params() {
    let src = "const :: A B => A\nconst 42 \"hello\"";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

/// `const 3.14 42` → Float (A=Float, B=Int)
#[test]
fn generic_const_float_int() {
    let src = "const :: A B => A\nconst 3.14 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::float());
}

// ── Type param em Generic (Result/Optional) ────────────────────────

/// `identity_result :: Result::(T, E) => Result::(T, E)`
/// `identity_result (Result::Ok 42)` → Result<Int, ?>
///
/// Nota: T é inferido como Int do payload. E fica como Var("E") pois
/// não é inferido por esta chamada.
#[test]
fn generic_result_type_param() {
    let src =
        "identity_result :: Result::(T, E) => Result::(T, E)\nidentity_result (Result::Ok 42)";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    // T = Int (do payload), E não-inferido → mantém Var("E")
    assert!(matches!(entry.ty, Ty::Generic(ref n, ref args) if n == "Result" && args.len() == 2));
    if let Ty::Generic(_, args) = &entry.ty {
        assert_eq!(args[0], Ty::int(), "T deve ser Int");
    }
}

// ── unify detecta inconsistência ───────────────────────────────────

/// `id :: T => T` com 2 args — deve falhar (arity mismatch, não unify).
/// id tem arity 1, chamar com 2 args é erro de dispatch.
#[test]
fn generic_id_wrong_arity() {
    let src = "id :: T => T\nid 42 3.14";
    let err = infer_src_err(src);
    // Deve ser erro de dispatch (FunctionNotFound ou TypeMismatch por arity)
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
            | kata_diagnostics::MiddleError::UnboundName { .. }
            | kata_diagnostics::MiddleError::ArityMismatch { .. }
            | kata_diagnostics::MiddleError::NoOverload { .. }
    ));
}

// ── Overload não-genérica vence sobre genérica ─────────────────────

/// `id :: T => T` + `id :: Int => Int` — chamar `id 42` deve despachar
/// para a versão concreta (exact match), não a genérica.
#[test]
fn concrete_overload_beats_generic() {
    let src = "id :: Int => Int\nid :: T => T\nid 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── Função genérica com corpo (não-FFI) ────────────────────────────

/// `id :: T => T\nlambda x: T\nid 42`
///
/// Função genérica com corpo lambda. O typeck infere T=Int do arg,
/// aplica substitution no body, retorna Int.
#[test]
fn generic_function_with_body() {
    let src = "id :: T => T\nlambda x: x\nid 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

// ── unify: consistência do mesmo type param ───────────────────────

/// `duplicate :: T T => T` — `duplicate 42 42` → Int (ambos args mesmo T).
/// `duplicate 42 3.14` → erro (T=Int vs T=Float inconsistência).
#[test]
fn generic_duplicate_consistent() {
    let src = "duplicate :: T T => T\nduplicate 42 42";
    let tmod = infer_src(src);
    let entry = entry_typed(&tmod);
    assert_eq!(entry.ty, Ty::int());
}

#[test]
fn generic_duplicate_inconsistent() {
    let src = "duplicate :: T T => T\nduplicate 42 3.14";
    let err = infer_src_err(src);
    // unify detecta T=Int vs T=Float → TypeMismatch
    assert!(matches!(
        err,
        kata_diagnostics::MiddleError::TypeMismatch { .. }
    ));
}
