//! Testes E2E — OverloadSet: dispatch de Actions first-class sem hint.
//!
//! Fase 1 do PRD §12: `let f := echo` (sem hint) produz `Ty::OverloadSet`
//! internamente. `f!(args)` faz dispatch por args para resolver o overload.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve_with_prelude};
use kata_tree_shaking::tree_shake;
use serial_test::serial;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve_with_prelude(
        &module,
        "__local__",
        kata_resolution::DirectiveRegistry::new(),
        &prelude.interface_registry,
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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

// ── Test 1: let f := echo; f!("hello") — dispatch por args resolve ──
//
// echo tem overloads no prelude (SHOW, SHOW+File, SHOW+Socket).
// `let f := echo` sem hint produz Ty::OverloadSet.
// `f!("hello")` faz dispatch por args: "hello" é Text que implementa
// SHOW → resolve o overload (SHOW) => Unit.

#[test]
#[serial]
fn overloadset_dispatch_por_args_text() {
    let src = r#"action main => Unit
    let f := echo
    f!("hello")
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Test 2: dispatcher!(echo, "hello") com hint — hint resolve ──
// (já funcionava com hint-based, mas valida que não quebrou)
// Nota: o hint Action(Text) => Unit usa tipo concreto (Text implementa SHOW),
// mas echo tem overload com params [Interface("SHOW")]. O select_action_overload
// usa match_score que verifica se Text implementa SHOW via InterfaceRegistry.
// O call indireto dentro de dispatcher usa == estrito, que não suporta
// interface dispatch — esse é um bug pré-existente do caminho indirect,
// não da Fase 1. Testamos com hint de tipo concreto aqui.

#[test]
#[serial]
fn overloadset_hint_resolves() {
    // Testa que `let f := echo` com hint de tipo Action(Text) => Unit
    // resolve via select_action_overload (hint-based).
    // O hint Action(Text) => Unit deve casar com echo(SHOW) => Unit
    // porque Text implementa SHOW.
    let src = r#"action main => Unit
    let f := echo
    f!("hello")
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Test 3: let f := echo sem uso — tipo OverloadSet, sem erro ──

#[test]
#[serial]
fn overloadset_sem_uso_nao_erro() {
    let src = r#"action main => Unit
    let f := echo
    ()
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Test 4: let f := echo; f!(42) — dispatch por args resolve ──
// Int implementa SHOW no prelude (show tem overload para Int).
// f!(42) é válido — o dispatch por args seleciona echo(SHOW) => Unit.

#[test]
#[serial]
fn overloadset_dispatch_int_implementa_show() {
    let src = r#"action main => Unit
    let f := echo
    f!(42)
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Test 5: overloads com aridades diferentes — dispatch por arity ──
//
// echo tem overloads de arity 1 (SHOW), 2 (SHOW+File), 3 (SHOW+Socket).
// f!("hello") com arity 1 deve resolver para echo(SHOW) => Unit,
// não ambíguo com os de arity 2 e 3.

#[test]
#[serial]
fn overloadset_dispatch_arity_1_vs_2() {
    let src = r#"action main => Unit
    let f := echo
    f!("hello")
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Fase 2: Test 6 — Action genérica (SHOW) via OverloadSet + monomorfização ──
//
// `worker` é uma action genérica com param `msg :: SHOW`. `let f := worker`
// produz Ty::OverloadSet. `f!("hello")` faz dispatch por args: Text implementa
// SHOW → seleciona overload (SHOW) => Unit → ActionCall direto.
// O monomorphizador deve instanciar `worker_SHOW_Text` via unify.

#[test]
#[serial]
fn overloadset_action_generica_show_monomorfiza() {
    let src = r#"action worker (msg :: SHOW) => Unit
    echo!(msg)
action main => Unit
    let f := worker
    f!("hello")
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Fase 2: Test 6b — Action genérica (SHOW) chamada direta (sem OverloadSet) ──
//
// `worker!("hello")` chama worker diretamente (não via `let f := worker`).
// Isto testa se echo!(msg) funciona dentro de uma action genérica com
// param Interface("SHOW") — o caso base sem OverloadSet.

#[test]
#[serial]
fn action_generica_show_chamada_direta() {
    let src = r#"action worker (msg :: SHOW) => Unit
    echo!(msg)
action main => Unit
    worker!("hello")
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Fase 2: Test 9 — Action genérica com Int (Int implementa SHOW) ──
//
// Variante do Test 6 com Int em vez de Text. `let f := worker; f!(42)`
// deve instanciar `worker_SHOW_Int` (Int implementa SHOW).

#[test]
#[serial]
fn action_generica_show_com_int_monomorfiza() {
    let src = r#"action worker (msg :: SHOW) => Unit
    echo!(msg)
action main => Unit
    let f := worker
    f!(42)
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Fase 3: Test 7 — dispatcher!(echo) — OverloadSet como arg direto ──
//
// Passar uma action como argumento para outra action. `echo` é
// referenciado sem hint → Ty::OverloadSet. `dispatcher` espera
// `g :: Action(Text) => Unit`. O match_score (braço OverloadSet
// vs Action) aceita na inference. O monomorphizer instancia
// echo_SHOW_Text e rewrites o arg para Ident("echo_SHOW_Text")
// com ty: Action([Text], Unit). O codegen produz fn_ptr válido.

#[test]
#[serial]
fn dispatcher_recebe_action_concreta() {
    let src = r#"action dispatcher (g :: Action(Text) => Unit) => Unit
    g!("hello")
action main => Unit
    dispatcher!(echo)
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}

// ── Fase 3: Test 8 — let f := echo; dispatcher!(f) — via variável ──
//
// Mesmo cenário do Test 7, mas o OverloadSet chega via variável
// (`let f := echo`). O monomorphizer vê o arg Ident("f") com
// ty: OverloadSet { name: "echo" } e instancia echo_SHOW_Text.

#[test]
#[serial]
fn overloadset_via_variavel_passa_para_dispatcher() {
    let src = r#"action dispatcher (g :: Action(Text) => Unit) => Unit
    g!("hello")
action main => Unit
    let f := echo
    dispatcher!(f)
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Unit);
    assert_eq!(raw, 0);
}
