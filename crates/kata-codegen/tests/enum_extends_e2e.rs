//! Testes E2E de `enum extends` — herança composicional de enums.
//!
//! 1. `enum B extends A` — flattening de variantes (herdadas + próprias).
//! 2. `final enum` — bloqueia extensão (erro compile-time).
//! 3. Redefinição de variante herdada — erro compile-time.
//! 4. Transitividade (A → B → C) — variantes acumuladas.
//! 5. Enum base inexistente — erro compile-time.
//! 6. Ciclo de extends — erro compile-time.
//! 7. Enum final do prelude (Boolean, Result, Optional) não pode ser estendido.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve_with_prelude};
use kata_tree_shaking::tree_shake;

// ── Helpers ───────────────────────────────────────────────────────

fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve_with_prelude(
        &module,
        "__local__",
        kata_resolution::DirectiveRegistry::new(),
        &prelude.interface_registry,
        &prelude.directive_registry,
        &prelude.enum_registry,
    )
    .expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr())
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

fn resolve_src(src: &str) -> Result<ResolvedModule, String> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve_with_prelude(
        &module,
        "__local__",
        kata_resolution::DirectiveRegistry::new(),
        &prelude.interface_registry,
        &prelude.directive_registry,
        &prelude.enum_registry,
    );
    match user {
        Ok(r) => {
            let merged = merge_resolved(prelude, r);
            Ok(merged)
        }
        Err(errors) => {
            let msg = errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            Err(msg)
        }
    }
}

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

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Extends básico ────────────────────────────────────────────────

/// `enum B extends A` — B tem variantes de A + próprias.
/// Match exaustivo sobre B cobre todas as variantes.
#[test]
fn extends_basic_flattening() {
    let src = r#"enum Base
    Foo
    Bar

enum Deriv extends Base
    Baz

action main => Int
    let x := Deriv::Foo
    match x
        Foo: 1
        Bar: 2
        Baz: 3
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1);
}

/// Construção de variante herdada sem qualificar — `Foo` resolve como
/// variante de `Deriv` (único enum com `Foo` no escopo, já que `Base`
/// também tem `Foo` mas `Deriv` é o que está em uso).
#[test]
fn extends_inherited_variant_unqualified() {
    let src = r#"enum Base
    Foo
    Bar

enum Deriv extends Base
    Baz

action main => Int
    let x := Deriv::Foo
    match x
        Foo: 10
        Bar: 20
        Baz: 30
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 10);
}

/// Variante própria (`Baz`) também funciona.
#[test]
fn extends_own_variant() {
    let src = r#"enum Base
    Foo
    Bar

enum Deriv extends Base
    Baz

action main => Int
    let x := Deriv::Baz
    match x
        Foo: 10
        Bar: 20
        Baz: 30
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 30);
}

// ── Transitividade ────────────────────────────────────────────────

/// A → B → C: C tem variantes de A + B + próprias.
#[test]
fn extends_transitivity() {
    let src = r#"enum A
    X

enum B extends A
    Y

enum C extends B
    Z

action main => Int
    let x := C::X
    match x
        X: 1
        Y: 2
        Z: 3
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1);
}

/// Transitividade — variante do meio (B.Y) também funciona em C.
#[test]
fn extends_transitivity_middle_variant() {
    let src = r#"enum A
    X

enum B extends A
    Y

enum C extends B
    Z

action main => Int
    let x := C::Y
    match x
        X: 1
        Y: 2
        Z: 3
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 2);
}

// ── Final bloqueia extensão ───────────────────────────────────────

/// `final enum Base` — estender Base é erro compile-time.
#[test]
fn final_enum_blocks_extension() {
    let src = r#"final enum Base
    Foo
    Bar

enum Deriv extends Base
    Baz"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "estender enum final deve falhar");
    let msg = err.unwrap_err();
    assert!(msg.contains("final"), "erro deve mencionar final: {msg}");
    assert!(msg.contains("Base"), "erro deve mencionar Base: {msg}");
}

/// Enums do prelude (Boolean, Result, Optional) são final.
/// Estender Boolean deve falhar.
#[test]
fn prelude_boolean_is_final() {
    let src = r#"enum MeuBool extends Boolean
    Talvez"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "estender Boolean (final) deve falhar");
    let msg = err.unwrap_err();
    assert!(msg.contains("final"), "erro deve mencionar final: {msg}");
}

/// Estender Result (final) deve falhar.
#[test]
fn prelude_result_is_final() {
    let src = r#"enum MeuResult extends Result
    Pending"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "estender Result (final) deve falhar");
    let msg = err.unwrap_err();
    assert!(msg.contains("final"), "erro deve mencionar final: {msg}");
}

/// Estender Optional (final) deve falhar.
#[test]
fn prelude_optional_is_final() {
    let src = r#"enum MeuOpt extends Optional
    Unknown"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "estender Optional (final) deve falhar");
    let msg = err.unwrap_err();
    assert!(msg.contains("final"), "erro deve mencionar final: {msg}");
}

// ── Redefinição de variante ───────────────────────────────────────

/// Variante própria com mesmo nome de herdada — erro.
#[test]
fn extends_variant_redefinition_blocked() {
    let src = r#"enum Base
    Foo
    Bar

enum Deriv extends Base
    Foo"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "redefinir variante herdada deve falhar");
    let msg = err.unwrap_err();
    assert!(
        msg.contains("Foo"),
        "erro deve mencionar variante Foo: {msg}"
    );
    assert!(
        msg.contains("redef"),
        "erro deve mencionar redefinição: {msg}"
    );
}

// ── Enum base inexistente ─────────────────────────────────────────

/// `extends EnumInexistente` — erro compile-time.
#[test]
fn extends_unknown_base() {
    let src = r#"enum Deriv extends Inexistente
    Foo"#;
    let err = resolve_src(src);
    assert!(err.is_err(), "enum base inexistente deve falhar");
    let msg = err.unwrap_err();
    assert!(
        msg.contains("Inexistente"),
        "erro deve mencionar Inexistente: {msg}"
    );
}

// ── Enum aberto (sem final) permite extensão ──────────────────────

/// Enum sem `final` pode ser estendido livremente.
#[test]
fn open_enum_allows_extension() {
    let src = r#"enum Base
    Foo
    Bar

enum Deriv1 extends Base
    Baz

enum Deriv2 extends Base
    Qux

action main => Int
    let x := Deriv1::Baz
    match x
        Foo: 1
        Bar: 2
        Baz: 3
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 3);
}
