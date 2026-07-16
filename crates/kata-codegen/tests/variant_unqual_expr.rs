//! Testes E2E — Variantes desqualificadas em posição de expressão.
//!
//! `True` (sem `Boolean::`) funciona em pattern matching. Estes testes
//! validam que o mesmo funciona em posição de expressão, com fallback
//! no EnumRegistry quando `env.lookup(name)` falha.
//!
//! Casos cobertos:
//! - Boolean::True/False desqualificado como entry → raw 1/0
//! - True/False como arm body em match
//! - None desqualificado (Optional genérico)
//! - Ok 42 desqualificado com Apply (Result genérico)
//! - Some 42 desqualificado com Apply (Optional genérico)
//! - Enum do usuário: variante unitária desqualificada
//! - Enum do usuário: variante com payload desqualificada com Apply
//! - Ambiguidade: dois enums com mesma variante → erro

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_diagnostics::MiddleError;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
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
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Inferência sem codegen — para inspecionar a TAST.
fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("infer deve succeed")
}

/// Roda o pipeline até infer_module e retorna o erro.
fn infer_err(src: &str) -> MiddleError {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("deve produzir erro")
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

// ── Boolean desqualificado ───────────────────────────────────────────

#[test]
fn true_desqualificado_entry() {
    // `True` sozinho como entry expression → Boolean::True = raw 1
    let (raw, ty) = eval_src("True");
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    assert_eq!(raw, 1, "True desqualificado deve ser 1");
}

#[test]
fn false_desqualificado_entry() {
    let (raw, ty) = eval_src("False");
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    assert_eq!(raw, 0, "False desqualificado deve ser 0");
}

#[test]
fn true_como_arm_body_em_match() {
    // match Boolean::True
    //     True: True    ← arm body é expressão (True desqualificado)
    //     False: False
    let (raw, ty) = eval_src(
        r#"match Boolean::True
    True: True
    False: False"#,
    );
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    assert_eq!(raw, 1, "arm body True deve resolver para 1");
}

#[test]
fn false_como_arm_body_em_match() {
    // match Boolean::False
    //     True: False   ← arm body False = 0
    //     False: True   ← arm body True = 1
    // Boolean::False casa com pattern False → arm body True = 1
    let (raw, ty) = eval_src(
        r#"match Boolean::False
    True: False
    False: True"#,
    );
    assert_eq!(ty, Ty::Sum("Boolean".into()));
    assert_eq!(raw, 1, "False casa com pattern False, arm body True = 1");
}

// ── Optional desqualificado ──────────────────────────────────────────

#[test]
fn none_desqualificado_entry() {
    // `None` sozinho → Optional::None (genérico, type_args não-inferidos)
    let (_, ty) = eval_src("None");
    assert_eq!(
        ty,
        Ty::Generic("Optional".into(), vec![Ty::Var("T".into())]),
        "None desqualificado deve resolver para Optional::(T)"
    );
}

#[test]
fn some_desqualificado_com_apply() {
    // `Some 42` desqualificado → Optional::Some(42)
    let (_, ty) = eval_src("Some 42");
    assert_eq!(
        ty,
        Ty::Generic("Optional".into(), vec![Ty::Prim(PrimTy::Int)]),
        "Some 42 desqualificado deve resolver para Optional::(Int)"
    );
}

// ── Result desqualificado ────────────────────────────────────────────

#[test]
fn ok_desqualificado_com_apply() {
    // `Ok 42` desqualificado → Result::Ok(42)
    let (_, ty) = eval_src("Ok 42");
    assert_eq!(
        ty,
        Ty::Generic(
            "Result".into(),
            vec![Ty::Prim(PrimTy::Int), Ty::Var("E".into())]
        ),
        "Ok 42 desqualificado deve resolver para Result::(Int, E)"
    );
}

// ── Enum do usuário desqualificado ───────────────────────────────────

#[test]
fn variante_unitaria_enum_do_usuario() {
    // enum Cor
    //     Vermelho
    //     Verde
    //     Azul
    // Vermelho
    let src = r#"enum Cor
    Vermelho
    Verde
    Azul
Vermelho"#;
    let (_, ty) = eval_src(src);
    assert_eq!(ty, Ty::Sum("Cor".into()));
}

// Nota: O parser de Kata5 atualmente não suporta `enum Forma\n    Circulo Int`
// como variante com payload — ele separa `Circulo` e `Int` como variantes
// unitárias distintas. Variantes com payload só funcionam para enums do
// prelude (Result/Optional), que são registrados manualmente em prelude_sigs.rs.
// Quando o parser for estendido para suportar payloads em enums do usuário,
// o fallback de infer_apply já estará pronto para resolver `Circulo 42`.

// ── Ambiguidade ──────────────────────────────────────────────────────

#[test]
fn ambiguidade_dois_enums_mesma_variante() {
    // Dois enums com variante `True` → erro de ambiguidade
    let src = r#"enum Flag
    True
    Off
True"#;
    let err = infer_err(src);
    assert!(
        matches!(err, MiddleError::UnboundName { ref name, .. } if name.contains("ambí")),
        "deve ser erro de ambiguidade, foi: {err:?}"
    );
}

#[test]
fn variante_com_payload_sem_apply_erro() {
    // `Ok` sozinho (sem Apply) tem payload → erro
    let err = infer_err("Ok");
    assert!(
        matches!(err, MiddleError::UnboundName { ref name, .. } if name.contains("payload")),
        "deve mencionar payload, foi: {err:?}"
    );
}

// ── TAST inspection: confirma VariantQual na TAST ────────────────────

#[test]
fn tast_true_desqualificado_gera_variantqual() {
    use kata_inference::TypedExprKind;

    let typed = infer_src("True");
    let entry = &typed.entry;
    match &entry.node.kind {
        TypedExprKind::VariantQual {
            enum_name,
            variant,
            tag,
        } => {
            assert_eq!(enum_name, "Boolean");
            assert_eq!(variant, "True");
            assert_eq!(tag, &0);
        }
        other => panic!("True desqualificado deve produzir VariantQual, foi: {other:?}"),
    }
}
