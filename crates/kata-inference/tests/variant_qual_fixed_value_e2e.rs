//! Testes E2E — VariantQual com fixed_value em enum genérico.
//!
//! Quando um enum genérico tem uma variante com valor fixo constante
//! (ex: `DefaultError(500)`) e o usuário a referencia sem Apply
//! (`ResultOrError::DefaultError`), a inferência deve produzir
//! `VariantConstruct` com o payload literal — não `VariantQual` (que
//! armazena payload = 0 no codegen).

use kata_core::ty::{PrimTy, Ty};
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

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
    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry,
        type_graph: prelude.type_graph.clone(),
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// Enum genérico com variante fixed_value: `DefaultError(500)`.
/// `Ok(T)` faz o enum ser genérico (type param T).
/// `DefaultError(500)` é variante constante — escrever
/// `ResultOrError::DefaultError` sem Apply deve construir com payload 500.
const SRC_GENERIC_FIXED: &str = r#"enum ResultOrError
    Ok(T)
    DefaultError(500)
ResultOrError::DefaultError"#;

// ── TAST: VariantConstruct com payload literal ──────────────────────

#[test]
fn generic_fixed_value_produz_variant_construct() {
    let typed = infer_src(SRC_GENERIC_FIXED);
    match &typed.entry.node.kind {
        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            tag,
            ..
        } => {
            assert_eq!(enum_name, "ResultOrError");
            assert_eq!(variant, "DefaultError");
            // Payload deve ser IntLit { text: "500" }
            match &payload.node.kind {
                TypedExprKind::IntLit { text } => {
                    assert_eq!(text, "500", "payload deve ser literal 500");
                }
                other => panic!("payload deve ser IntLit, foi: {other:?}"),
            }
            assert_eq!(tag, &1, "DefaultError é a segunda variante (tag 1)");
        }
        other => panic!("esperava VariantConstruct, foi: {other:?}"),
    }
}

#[test]
fn generic_fixed_value_tipo_eh_generic_com_var() {
    // O tipo deve ser Ty::Generic("ResultOrError", [Ty::Var("T")])
    // — type args não-inferidos porque a variante fixed_value não
    // menciona T.
    let typed = infer_src(SRC_GENERIC_FIXED);
    let ty = &typed.entry.node.ty;
    match ty {
        Ty::Generic(name, type_args) => {
            assert_eq!(name, "ResultOrError");
            assert_eq!(
                type_args,
                &[Ty::Var("T".into())],
                "type args devem ser [Var(\"T\")] — não-inferidos"
            );
        }
        other => panic!("tipo deve ser Ty::Generic, foi: {other:?}"),
    }
}

#[test]
fn generic_fixed_value_payload_eh_int() {
    // O payload do VariantConstruct deve ter tipo Int (inferido do
    // literal 500).
    let typed = infer_src(SRC_GENERIC_FIXED);
    match &typed.entry.node.kind {
        TypedExprKind::VariantConstruct { payload, .. } => {
            assert_eq!(
                payload.node.ty,
                Ty::Prim(PrimTy::Int),
                "payload do fixed_value 500 deve ter tipo Int"
            );
        }
        other => panic!("esperava VariantConstruct, foi: {other:?}"),
    }
}

// ── Variante sem fixed_value no mesmo enum continua unitária ─────────

#[test]
fn generic_variante_unitaria_sem_fixed_value_continua_variant_qual() {
    // `Ok` sem Apply tem payload T → deve dar erro (exige Apply).
    // `DefaultError` sem Apply tem fixed_value → VariantConstruct.
    // Mas se escrevermos apenas o enum sem usar Ok, Ok não é testado.
    // Aqui testamos que uma variante unitária sem fixed_value em enum
    // genérico ainda produz VariantQual (não regressão).
    let src = r#"enum OptionalExtended
    Some(T)
    None
OptionalExtended::None"#;
    let typed = infer_src(src);
    match &typed.entry.node.kind {
        TypedExprKind::VariantQual {
            enum_name, variant, ..
        } => {
            assert_eq!(enum_name, "OptionalExtended");
            assert_eq!(variant, "None");
        }
        other => {
            panic!("None (unitária, sem fixed_value) deve produzir VariantQual, foi: {other:?}")
        }
    }
}

// ── fixed_value com Float ────────────────────────────────────────────

#[test]
fn generic_fixed_value_float_produz_variant_construct() {
    let src = r#"enum Measurement
    Reading(T)
    DefaultValue(0.0)
Measurement::DefaultValue"#;
    let typed = infer_src(src);
    match &typed.entry.node.kind {
        TypedExprKind::VariantConstruct {
            variant, payload, ..
        } => {
            assert_eq!(variant, "DefaultValue");
            match &payload.node.kind {
                TypedExprKind::FloatLit { text } => {
                    assert_eq!(text, "0.0", "payload deve ser literal 0.0");
                }
                other => panic!("payload deve ser FloatLit, foi: {other:?}"),
            }
            assert_eq!(
                payload.node.ty,
                Ty::Prim(PrimTy::Float),
                "payload float deve ter tipo Float"
            );
        }
        other => panic!("esperava VariantConstruct, foi: {other:?}"),
    }
}

// ── fixed_value com Text ─────────────────────────────────────────────

#[test]
fn generic_fixed_value_text_produz_variant_construct() {
    let src = r#"enum Response
    Body(T)
    DefaultMsg("ok")
Response::DefaultMsg"#;
    let typed = infer_src(src);
    match &typed.entry.node.kind {
        TypedExprKind::VariantConstruct {
            variant, payload, ..
        } => {
            assert_eq!(variant, "DefaultMsg");
            match &payload.node.kind {
                TypedExprKind::TextLit { text } => {
                    assert_eq!(text, "ok", "payload deve ser literal \"ok\"");
                }
                other => panic!("payload deve ser TextLit, foi: {other:?}"),
            }
            assert_eq!(
                payload.node.ty,
                Ty::Prim(PrimTy::Text),
                "payload text deve ter tipo Text"
            );
        }
        other => panic!("esperava VariantConstruct, foi: {other:?}"),
    }
}
