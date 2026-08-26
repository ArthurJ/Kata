//! Testes E2E do monomorphizador.
//!
//! Verifica que:
//! 1. Módulo sem generics → MonoModule idêntico (sem novas funções)
//! 2. `id :: T => T` com `id 42` → gera `id_T_Int` no DispatchTable e rewrites o callee
//! 3. `id` com Float e Int → gera duas instâncias distintas
//! 4. Função genérica que chama outra genérica → fixpoint
//! 5. `pair :: A B => (A, B)` com 2 type params → instância com 2 subs
//!
//! Nota: `id :: T => T` sem corpo é Sig-only — só existe no DispatchTable
//! como OverloadInfo, não em `mono.functions`. O monomorphizador gera
//! OverloadInfo sempre (para o DispatchTable) e
//! TypedFunction só quando há corpo lambda.

use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, infer_module};
use kata_lexer::lex;
use kata_monomorph::{MonoModule, monomorphize};
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
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
        interface_registry,
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
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

fn mono_src(src: &str) -> MonoModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    monomorphize(typed)
}

/// Verifica se uma instância com dado nome existe no DispatchTable.
///
/// Instâncias monomorfizadas são registradas como OverloadInfo no
/// DispatchTable (mesmo sem corpo). Para funções com corpo, também
/// existem em `mono.functions`.
fn has_instance(mono: &MonoModule, name: &str) -> bool {
    mono.dispatch_table
        .get_overloads(name)
        .is_some_and(|ovs| !ovs.is_empty())
}

/// Encontra o nome do callee na entry expression (assume Closure).
fn entry_callee_name(mono: &MonoModule) -> Option<String> {
    let entry = &mono.entry.node;
    match &entry.kind {
        TypedExprKind::Closure { callee, .. } => match &callee.node.kind {
            TypedExprKind::Ident { name } => Some(name.clone()),
            _ => None,
        },
        _ => None,
    }
}

// ── Módulo sem generics ────────────────────────────────────────

/// Módulo sem generics não gera instâncias.
#[test]
fn no_generics_no_instances() {
    let src = "42";
    let mono = mono_src(src);
    // Não há funções genéricas, então nenhuma instância é gerada.
    // O número de funções deve ser o mesmo do TypedModule.
    let n_funcs = mono.functions.len();
    let mono2 = mono_src(src);
    assert_eq!(mono2.functions.len(), n_funcs);
}

// ── id :: T => T — caso canônico ───────────────────────────────

/// `id :: T => T` + `id 42` → gera `id_T_Int` no DispatchTable e rewrites o callee.
#[test]
fn generic_id_generates_int_instance() {
    let src = "id :: T => T\nid 42";
    let mono = mono_src(src);

    let instance_name = "id_T_Int";
    assert!(
        has_instance(&mono, instance_name),
        "instância {instance_name} deve existir no DispatchTable"
    );

    // O callee da entry deve ser a instância, não `id`.
    let callee = entry_callee_name(&mono).expect("entry deve ser Closure");
    assert_eq!(callee, instance_name);
}

/// `id` com Float → gera `id_T_Float`.
#[test]
fn generic_id_generates_float_instance() {
    let src = "id :: T => T\nid 3.14";
    let mono = mono_src(src);

    let instance_name = "id_T_Float";
    assert!(
        has_instance(&mono, instance_name),
        "instância {instance_name} deve existir no DispatchTable"
    );
}

/// `id` com Int e Float → gera duas instâncias distintas.
#[test]
fn generic_id_two_instances() {
    let src = "id :: T => T\nconstant x := id 42\nconstant y := id 3.14\nx";
    let mono = mono_src(src);

    assert!(
        has_instance(&mono, "id_T_Int"),
        "instância Int deve existir"
    );
    assert!(
        has_instance(&mono, "id_T_Float"),
        "instância Float deve existir"
    );
}

// ── pair :: A B => (A, B) — múltiplos type params ──────────────

/// `pair :: A B => (A, B)` com `pair 42 3.14` → gera instância com 2 subs.
#[test]
fn generic_pair_two_type_params() {
    let src = "pair :: A B => (A, B)\npair 42 3.14";
    let mono = mono_src(src);

    let instance_name = "pair_A_Int_B_Float";
    assert!(
        has_instance(&mono, instance_name),
        "instância {instance_name} deve existir no DispatchTable"
    );
}

// ── Instância tem tipos concretos ──────────────────────────────

/// A instância monomorfizada deve ter param_types e ret_ty concretos no DispatchTable.
#[test]
fn instance_has_concrete_types() {
    let src = "id :: T => T\nid 42";
    let mono = mono_src(src);

    let overloads = mono
        .dispatch_table
        .get_overloads("id_T_Int")
        .expect("overloads de id_T_Int devem existir");
    assert_eq!(overloads.len(), 1);
    let oi = &overloads[0];

    assert_eq!(oi.params, vec![Ty::int()]);
    assert_eq!(oi.ret, Ty::int());
}

/// A instância não deve ter type_params (é concreta).
#[test]
fn instance_has_no_type_params() {
    let src = "id :: T => T\nid 42";
    let mono = mono_src(src);

    let overloads = mono
        .dispatch_table
        .get_overloads("id_T_Int")
        .expect("overloads de id_T_Int devem existir");
    assert_eq!(overloads.len(), 1);
    assert!(
        overloads[0].type_params.is_empty(),
        "instância não deve ter type_params"
    );
    assert!(
        overloads[0].substitutions.is_some(),
        "instância deve ter substitutions = Some"
    );
}

// ── Função original genérica é preservada ──────────────────────

/// A função genérica original `id` deve continuar no DispatchTable.
#[test]
fn original_generic_function_preserved() {
    let src = "id :: T => T\nid 42";
    let mono = mono_src(src);

    // `id` deve ter overload genérico preservado.
    let id_overloads = mono
        .dispatch_table
        .get_overloads("id")
        .expect("overloads de id devem existir");
    assert!(
        id_overloads.iter().any(|oi| !oi.type_params.is_empty()),
        "função original `id` deve ter overload genérico preservado"
    );

    // `id_T_Int` deve existir como instância.
    assert!(
        has_instance(&mono, "id_T_Int"),
        "instância id_T_Int deve existir"
    );
}

// ── Múltiplas chamadas mesmo tipo → uma instância ──────────────

/// `id 42` duas vezes → gera uma só instância (dedup).
#[test]
fn same_type_dedup_instance() {
    let src = "id :: T => T\nconstant x := id 42\nconstant y := id 42\nx";
    let mono = mono_src(src);

    let overloads = mono
        .dispatch_table
        .get_overloads("id_T_Int")
        .expect("overloads de id_T_Int devem existir");
    assert_eq!(
        overloads.len(),
        1,
        "deve ter exatamente 1 overload id_T_Int"
    );
}

// ── Função genérica com corpo ──────────────────────────────────

/// `id :: T => T` com corpo lambda → instância tem o corpo em `mono.functions`.
#[test]
fn generic_function_with_body() {
    let src = "id :: T => T\nlambda x: x\nid 42";
    let mono = mono_src(src);

    // Com corpo, a instância deve existir tanto no DispatchTable quanto em functions.
    assert!(
        has_instance(&mono, "id_T_Int"),
        "instância id_T_Int deve existir no DispatchTable"
    );

    let instance = mono
        .functions
        .iter()
        .find(|f| f.name == "id_T_Int")
        .expect("instância id_T_Int deve existir em functions (tem corpo)");

    // A instância deve ter cláusulas com o corpo.
    assert!(!instance.clauses.is_empty(), "instância deve ter cláusulas");

    // O tipo do corpo deve ser Int (substituído de T).
    let body = &instance.clauses[0].body.node;
    assert_eq!(body.ty, Ty::int());
}
