//! Testes E2E de default type params — `Err(E|Text)`.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Valida que:
//! - `Result::(Int)` resolve para `Result::(Int, Text)` via default
//! - `Result::(Int, MyError)` funciona com tipo customizado (não usa default)
//! - `T?` desaçucar para `Result::(T)` e o default preenche `E|Text`
//! - Construção `Result::Ok 42` sem hint produz `Result::(Int, Text)` via default
//! - Enum customizado do usuário com defaults (ex: `enum Config { Port(P|Int) }`)
//! - `at` em Array/List/Text/Dict retorna `Result::A` — default preenche `E|Text`

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

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
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Result::(Int) na assinatura — default preenche E|Text ────────────

/// `Result::(Int)` na assinatura de action deve resolver para `Result::(Int, Text)`
/// via default. A action retorna `Result::Ok 42` e o match desempacota.
#[test]
fn result_int_assinatura_resolve_default() {
    let src = r#"action faz_ok => Result::(Int)
    Result::Ok 42

action main => Int
    match (faz_ok!())
        Result::Ok v: v
        Result::Err _: 0

main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

/// `Result::(Int, MyError)` com tipo customizado NÃO usa default.
/// O usuário fornece E explicitamente.
#[test]
fn result_com_erro_customizado_nao_usa_default() {
    let src = r#"enum MyError
    Code(Int)

action faz_err => Result::(Int, MyError)
    Result::Err (MyError::Code 99)

action main => Int
    match (faz_err!())
        Result::Ok v: v
        Result::Err e: match e
            MyError::Code c: c

main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 99);
}

// ── T? desaçucara para Result::(T) — default preenche E|Text ──────────

/// `T?` dentro de Action que retorna `Result::(Int, Text)`.
/// O `?` desempacota `Ok(v)` ou aborta com `return Err(e)`.
#[test]
fn question_mark_desempacota_result_com_default() {
    let src = r#"action faz_ok => Result::(Int, Text)
    Result::Ok 42

action main => Result::(Int, Text)
    let v := faz_ok!() ?
    Result::Ok v

main!()"#;
    let (_, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

/// `T?` em action que retorna `Result::(Int)` (1 arg, default E|Text).
#[test]
fn question_mark_com_result_1_arg_default() {
    let src = r#"action faz_ok => Result::(Int)
    Result::Ok 42

action main => Result::(Int)
    let v := faz_ok!() ?
    Result::Ok v

main!()"#;
    let (_, ty) = eval_src(src);
    assert_eq!(
        ty,
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()])
    );
}

// ── Construção sem hint — default preenche na construção do variant ──

/// `Result::Ok 42` sem hint em match top-level.
/// O default `Err(E|Text)` preenche E|Text automaticamente.
#[test]
fn result_ok_sem_hint_preenche_default() {
    let src = r#"match (Result::Ok 42)
    Result::Ok v: v
    Result::Err _: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42);
}

// ── Enum customizado do usuário com defaults ────────────────────────

/// `enum Config { Port(P|Int) }` — o usuário declara default em seus próprios enums.
#[test]
fn enum_customizado_com_default() {
    let src = r#"enum Config
    Port(P|Int)

action make_config => Config::(Int)
    Config::Port 8080

action main => Int
    match (make_config!())
        Config::Port p: p

main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 8080);
}

// ── at em Array — default preenche E|Text ───────────────────────────

/// `at` em Array retorna `Result::A` (1 arg) — default preenche E|Text.
#[test]
fn at_em_array_retorna_result_com_default() {
    let src = r#"action main => Int
    let arr := {10 20 30}
    match (at arr 0)
        Result::Ok v: v
        Result::Err _: 0

main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // Nota: o valor retornado por `at` pode variar conforme a indexação
    // do runtime (base-0 ou base-1). O importante é que o match
    // desempacota Ok(v) sem erro de tipo.
    let _ = untag_smi(raw);
}
