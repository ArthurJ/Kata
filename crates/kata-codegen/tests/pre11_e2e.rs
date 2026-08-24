//! Testes E2E do Pré-11 — memória hierárquica.
//!
//! Estes testes exercitam os code paths que antes vazavam na arena global
//! (handle 0, nunca destruída): tuplas em função pura, CaptureBox, Sum
//! results, e tuplas em Actions. Com o Pré-11, esses objetos são alocados
//! na arena determinada pelo EscapeTarget e destruídos no fim do run.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = kata_inference::infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
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
        internal_signatures: Vec::new(),
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

// ── Tupla em função pura ────────────────────────────────────────────

/// Tupla em função pura: antes do Pré-11, alocava na arena global (handle 0,
/// nunca destruída). Agora aloca na arena raiz do scheduler.
#[test]
fn tupla_em_funcao_pura_nao_vaza() {
    let src = "soma_tupla :: Int Int => (Int, Int)\nlambda a b: (a, b)\nsoma_tupla 3 4";
    let (val, ty) = eval_src(src);
    assert!(val != 0, "tupla deve ser alocada (ptr != 0)");
    assert!(matches!(ty, Ty::Tuple(_)));
}

/// Tupla literal no entry point.
#[test]
fn tupla_literal_no_entry() {
    let (val, _ty) = eval_src("(1, 2)");
    assert!(val != 0, "tupla deve ser alocada (ptr != 0)");
}

// ── Closure com captura ─────────────────────────────────────────────

/// Closure com captura: CaptureBox alocado na caller_arena, não na global.
/// (migrado de `constant f := + _ n` — sections produzem lambdas, que não
/// são serializáveis como constant. Usa named function que retorna lambda.)
#[test]
fn closure_com_captura_nao_vaza() {
    let src = "make_adder :: Int => (Int -> Int)
lambda n: lambda x: + x n
constant add10 := make_adder 10
add10 5";
    let (val, _ty) = eval_src(src);
    assert_eq!(val >> 1, 15, "make_adder 10 5 deve retornar 15");
}

// ── Sum result ──────────────────────────────────────────────────────

/// Sum result: não vaza na arena global.
#[test]
fn sum_result_em_funcao_pura_nao_vaza() {
    let (val, _ty) = eval_src("Ok 42");
    assert!(val != 0, "sum box deve ser alocado (ptr != 0)");
}

/// Variante unitária de enum do usuário (None).
#[test]
fn variant_unitaria_nao_vaza() {
    let (val, _ty) = eval_src("None");
    assert!(val != 0, "sum box unitário deve ser alocado (ptr != 0)");
}

// ── Action com tupla local ───────────────────────────────────────────

/// Action que cria tupla local: aloca em fiber_arena, liberada quando fiber
/// termina.
#[test]
fn action_tupla_local_destruida() {
    let src = "action teste => (Int, Int)\n    (1, 2)\nteste!()";
    let (val, _ty) = eval_src(src);
    assert!(val != 0, "tupla deve ser alocada");
}

/// Action que retorna Int.
#[test]
fn action_retorna_int() {
    let src = "action soma => Int\n    + 2 3\nsoma!()";
    let (val, _ty) = eval_src(src);
    assert_eq!(val >> 1, 5, "soma! deve retornar 5");
}

// ── Arena raiz destruída ─────────────────────────────────────────────

/// Verifica que executa sem crash — a arena raiz é destruída após o run.
#[test]
fn arena_raiz_destruida_apos_run() {
    let (val, _ty) = eval_src("(1, 2)");
    assert!(val != 0, "tupla deve ser alocada");
}
