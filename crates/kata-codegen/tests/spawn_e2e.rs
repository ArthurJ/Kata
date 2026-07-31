//! Testes E2E de spawn! — multiprocess (fork+IPC).
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste verifica o valor retornado pelo JIT executando spawn! em processo OS separado.
//!
//! Caveats conhecidos (do handoff):
//! - fork() + threads: timer thread não sobrevive no child. Actions sem loops
//!   com yield_check devem funcionar.
//! - kata_rt_yield no parent: parent faz yield após fork. Se não há scheduler
//!   init, yield pode crashar. Verificar empiricamente.
//! - from_bytes e lifetime do blob: from_bytes deve copiar dados para a arena.

use std::collections::HashMap;

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{load_prelude, resolve, ResolvedModule};
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

// ── Teste 1: spawn! básico — Int ──
//
// Action simples que retorna x + 1. spawn! executa em processo separado,
// resultado volta via pipe IPC.

#[test]
fn spawn_basico_int() {
    let src = r#"action tarefa (x::Int) => Int
    + x 1
spawn!(tarefa, (41))"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "spawn!(tarefa, (41)) deve retornar 42");
}

// ── Teste 2: spawn! com tupla (Int, Int) → Int ──
//
// Action que recebe uma tupla, faz match, soma os elementos.
// A type table precisa ter TypeShape::Tuple([Prim, Prim]) no type_id
// correto para que to_bytes/from_bytes serializem a tupla corretamente.

#[test]
fn spawn_tupla_soma() {
    let src = r#"action soma_tupla (t::(Int, Int)) => Int
    match t
        (a, b): + a b
        otherwise: 0
spawn!(soma_tupla, ((15, 27)))"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "spawn!(soma_tupla, ((15, 27))) deve retornar 42");
}

// ── Teste 3: spawn! sem args (Unit) ──
//
// Action sem params, spawn! sem args.

#[test]
fn spawn_sem_args() {
    let src = r#"action resposta => Int
    42
spawn!(resposta, ())"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "spawn!(resposta, ()) deve retornar 42");
}