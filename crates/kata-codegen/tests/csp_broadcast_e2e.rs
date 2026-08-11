//! Testes E2E de codegen CSP — broadcast pub-sub e receiver factory.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com `broadcast!()`, `rxf!()` e múltiplos
//! receivers, verificando a semântica latest-only / future-only (Decisão F do
//! PRD-Fio11).
//!
//! Os testes cobrem:
//! 1. Pub-sub com múltiplos receivers via `rxf!()` — ambos recebem o último
//!    valor enviado (latest only).
//! 2. Receiver factory cria receivers independentes — cada um mantém seu
//!    próprio `last_seen_version`; late subscribers não recebem histórico.
//! 3. Múltiplos sends — receivers veem apenas o mais recente ao desbloquear.
//!
//! Destructuring `let (tx, rxf) := ...` é suportado (desugar para FieldAccess).
//!
//! Limitação do parser (pitfall #45): valores recebidos têm tipo `Var("T0")`
//! — evitamos operações aritméticas sobre o valor recebido; retornamo-lo direto.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;
use serial_test::serial;

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

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

// ── Teste 1: broadcast!() pub-sub — 2 receivers via rxf!() ──

/// Main cria broadcast, cria 2 receivers via `rxf!()`, envia 42 e ambos
/// recebem (latest only). Como o typeck não tem overload de aritmética para
/// `Var("T0")` (valor recebido), retornamos apenas `b` — mas `b` só é
/// recebido se `rx2 <!` desbloqueou, o que só acontece se `rx1 <! a` também
/// desbloqueou antes (mesma fiber, sequencial). Se `rx1 <! a` travasse, o
/// scheduler detectaria deadlock antes de chegar em `rx2 <! b`.
///
/// Topologia: `tx !> 42` é fire-and-forget (não bloqueia). Ambos receivers
/// compartilham o mesmo BroadcastInner via ponteiro. Cada um tem seu próprio
/// `last_seen_version` (inicializado = version atual = 0). Quando `tx !> 42`
/// incrementa version para 1, ambos veem `version > last_seen` e desbloqueiam.
///
/// DoD: "Pub-sub via broadcast! com múltiplos receivers (latest only)".
#[serial]
#[test]
fn broadcast_pubsub_multiplos_receivers() {
    let src = r#"action main => Int
  let (tx, rxf) := broadcast!()
  let rx1 := rxf!()
  let rx2 := rxf!()
  tx !> 42
  rx1 <! a
  rx2 <! b
  b
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "broadcast pub-sub: ambos receivers devem receber 42 (latest only)"
    );
}

// ── Teste 2: receiver factory cria receivers independentes (latest only) ──

/// Late subscriber não recebe histórico. Main cria broadcast, envia 99
/// **antes** de criar o receiver. Como o receiver nasce com
/// `last_seen_version = current_version`, ele **não** vê a mensagem 99
/// (future-only). Ao tentar `<!`, bloqueia (sem nova mensagem). Como não há
/// outros fibers, o scheduler detecta deadlock.
///
/// DoD: "Receiver factory: múltiplos receivers independentes" +
/// "Late subscribers não recebem histórico" (Decisão F).

#[test]
fn receiver_factory_late_subscriber_nao_recebe_historico() {
    let src = r#"action main => Int
  let (tx, rxf) := broadcast!()
  tx !> 99
  let rx := rxf!()
  rx <! v
  v
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_eq!(
        raw, DEADLOCK_SENTINEL,
        "late subscriber deve bloquear (future-only) — deadlock detectado"
    );
}

// ── Teste 3: múltiplos sends — receiver vê apenas o mais recente ──

/// Main cria broadcast, cria receiver, envia 10, 20, 30 em sequência
/// (sem yield entre sends — tudo na mesma fiber antes de recv). O receiver
/// ao fazer `<!` vê apenas o último (30), pois `version` incrementou 3x
/// e o receiver só lê o valor atual.
///
/// DoD: "Se o receiver é lento e perde mensagens intermediárias,
/// vê **a última** quando desbloqueia" (Decisão F).

#[test]
fn broadcast_multiplos_sends_latest_only() {
    let src = r#"action main => Int
  let (tx, rxf) := broadcast!()
  let rx := rxf!()
  tx !> 10
  tx !> 20
  tx !> 30
  rx <! v
  v
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        30,
        "broadcast latest-only: receiver deve ver apenas 30 (último)"
    );
}

// ── Teste 4: rxf!() retorna Receiver::T (typecheck) ──

/// Smoke test do typeck: `rxf!()` deve produzir `Receiver::T0` que pode fazer
/// `<!`. Se o typeck estivesse errado (produzindo ChannelCreate::Broadcast),
/// `<!` falharia com "esperado Receiver, encontrado Sender/ReceiverFactory".
///
/// Este teste é subsumido pelos anteriores, mas isola a verificação de tipos
/// para facilitar diagnóstico de regressões.

#[test]
fn rxf_retorna_receiver_que_pode_receber() {
    let src = r#"action main => Int
  let (tx, rxf) := broadcast!()
  let rx := rxf!()
  tx !> 42
  rx <! v
  v
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "rxf!() deve produzir Receiver que recebe 42"
    );
}
