//! Testes E2E de codegen de fibers, scheduler, ABI uniforme.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste verifica o valor retornado pelo JIT executando em fibers.

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

// ── Teste 1: Action simples executa em fiber — resultado correto ──

/// Action sem args que retorna um literal Int.
/// `action simples => Int` retorna 42. Deve executar em 1 fiber.
#[test]
fn action_simples_fiber() {
    let src = "action simples => Int\n    42\nsimples!()";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "simples!() deve retornar 42");
}

// ── Teste 2: Action aninhada (A→B→C) — calls diretos no mesmo fiber ──

/// A chama B, B chama C, C retorna 7. Tudo no mesmo fiber.
#[test]
fn action_aninhada_a_b_c() {
    let src = r#"action c => Int
    7
action b => Int
    c!()
action a => Int
    b!()
a!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7, "a!() → b!() → c!() deve retornar 7");
}

// ── Teste 3: Action com args (tupla) — args_ptr desempacotado ──

/// Action com 2 args Int que os soma.
#[test]
fn action_com_args_tupla() {
    let src = r#"action soma (a::Int, b::Int) => Int
    + a b
soma!(3, 4)"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 7, "soma!(3, 4) deve retornar 7");
}

// ── Teste 4: Action com 1 arg (Grouping normalizado) ──

/// `action!(x)` produz Grouping no parser, normalizado para Tuple de 1.
#[test]
fn action_com_1_arg_grouping() {
    let src = r#"action dobra (x::Int) => Int
    * x 2
dobra!(21)"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "dobra!(21) deve retornar 42");
}

// ── Teste 5: Action sem args (Unit) — args_ptr = 0, sem loads ──

/// `action!()` com Unit — args_ptr deve ser 0, sem loads.
#[test]
fn action_sem_args_unit() {
    let src = r#"action resposta => Int
    42
resposta!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "resposta!() deve retornar 42");
}

// ── Teste 6: Action com arena local — alocação na arena do fiber ──

/// Action que aloca uma tupla na arena do fiber e a acessa.
#[test]
fn action_aloca_na_arena_do_fiber() {
    let src = r#"action aloca => Int
    let t := (10, 20)
    match t
        (a, b): + a b
        otherwise: 0
aloca!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 30, "aloca!() deve retornar 30 (10+20)");
}

// ── Teste 7: Caller's arena — valor retornado sobrevive à destruição do fiber ──

/// Action aloca tupla em tail_pos (caller's arena) e retorna.
/// O caller acessa o valor — deve ser válido após o fiber (e sua arena) ser destruído.
#[test]
fn action_valor_sobrevive_fiber() {
    let src = r#"action cria_tupla => (Int, Int)
    (1, 2)
match cria_tupla!()
    (a, b): + a b
    otherwise: 0"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 3, "cria_tupla!() → (1,2) → 1+2 = 3");
}

// ── Teste 8: Action que retorna Float — bitcast na borda ──

/// Action retorna Float. O fiber retorna i64 (bitcast F64→I64 no epílogo),
/// e o caller faz bitcast I64→F64 para usar o valor.
#[allow(clippy::approx_constant)]
#[test]
fn action_retorna_float() {
    let src = r#"action pi => Float
    3.14
pi!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Float));
    // raw é i64 bitcast de f64 3.14
    let f = f64::from_bits(raw as u64);
    assert!((f - 3.14).abs() < 1e-6, "pi!() deve retornar 3.14, got {f}");
}
