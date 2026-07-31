//! Testes E2E de spawn! — multiprocess (fork).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! spawn! é fire-and-forget — fork + exec da Action em processo OS separado.
//! Não há retorno de valor. A comunicação entre parent e child é por canais.
//!
//! Estes testes verificam que o fork+exec acontece sem crashar o parent.
//! O child executa a Action e termina. O parent continua executando.

use std::collections::HashMap;

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_rt::{TypeShape, register_type_table};
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

    // type_id_map vazio — para Int, o fallback type_id=0 mapeia para
    // TypeShape::Prim. Precisamos registrar a type table no runtime TLS
    // para que to_bytes/from_bytes funcionem. O type_id 0 = Prim (Int).
    let type_id_map: HashMap<Ty, i64> = HashMap::new();
    register_type_table(vec![TypeShape::Prim]); // type_id 0 = Prim

    let jit = jit_eval(&typed, &type_id_map).expect("codegen+JIT deve succeed");
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

// ── Teste 1: spawn! básico — fire-and-forget ──
//
// Action simples que retorna x + 1. spawn! executa em processo separado.
// O parent não espera o child e não recebe retorno.
// O entry point retorna 42 (SMI) diretamente — o spawn! é side-effect only.

#[test]
fn spawn_basico_fire_and_forget() {
    let src = r#"action tarefa (x::Int) => Int
    + x 1
spawn!(tarefa, (41))
42"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "entry point deve retornar 42");
}

// ── Teste 2: spawn! sem args (Unit) ──
//
// Action sem params, spawn! sem args. Fire-and-forget.

#[test]
fn spawn_sem_args() {
    let src = r#"action resposta => Int
    42
spawn!(resposta, ())
42"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "entry point deve retornar 42");
}

// ── Teste 3: spawn! dentro de Action ──
//
// Action externa recebe x, faz spawn! da action interna.
// Exercita lower_spawn com caller_arena = arena do chamador da
// Action externa (não root_arena como no entry point).

#[test]
fn spawn_dentro_de_action() {
    let src = r#"action interna (x::Int) => Int
    + x 1
action externa (x::Int) => Int
    spawn!(interna, (x))
    0
spawn!(externa, (41))
42"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "entry point deve retornar 42");
}

// ── Teste 4: spawn! com tupla dentro de Action ──
//
// Igual ao teste 3, mas passa uma tupla (x, y) como arg. A tupla é
// alocada na arena (não SMI), então o args_ptr é um ponteiro real.

#[test]
fn spawn_dentro_de_action_tupla() {
    let src = r#"action interna (t::(Int, Int)) => Int
    match t
        (a, b): + a b
        otherwise: 0
action externa (x::Int) => Int
    spawn!(interna, ((x, 1)))
    0
spawn!(externa, (41))
42"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "entry point deve retornar 42");
}
