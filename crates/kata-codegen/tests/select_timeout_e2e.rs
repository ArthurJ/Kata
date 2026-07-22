//! Testes E2E de codegen de select com timeout.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste compila um programa Kata com `select` e verifica o valor
//! retornado pelo JIT executando em fibers com scheduler cooperativo.
//!
//! Os testes usam `fork!` para criar produtores que enviam em canais
//! diferentes. O `select` no main multiplexa entre os receivers e retorna
//! o valor do braço que disparar primeiro.
//!
//! Nota: o parser não suporta destructuring `let (tx, rx) := ...`.
//! Usamos `ch.0` (sender) e `ch.1` (receiver) via DotAccess/FieldAccess.
//!
//! **Sintaxe do select:** o `timeout` deve estar indentado dentro do bloco
//! do select (mesma indentação dos braços), não na indentação do `select`.

use kata_codegen::jit_eval;
use kata_core::InterfaceRegistry;
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

/// Sentinel de deadlock retornado por `kata_rt_run`.
const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

// ── Teste 1: select com 2 receivers — produtor envia em ch1 ──

/// Dois canais, dois produtores (fork). `prod_a` envia 100 em ch1,
/// `prod_b` envia 200 em ch2. O select deve disparar um dos braços.
///
/// Como o scheduler é cooperativo e single-threaded, a ordem de
/// execução dos forks depende do round-robin. O select retorna o
/// valor do primeiro canal que tiver dado disponível.
#[serial]
#[test]
fn select_2_receivers_dispara() {
    let src = r#"action prod_a (ch::Sender::Int) => Unit
  ch !> 100
  ()
action prod_b (ch::Sender::Int) => Unit
  ch !> 200
  ()
action main => Int
  let ch1 := channel!()
  let tx1 := ch1.0
  let rx1 := ch1.1
  let ch2 := channel!()
  let tx2 := ch2.0
  let rx2 := ch2.1
  fork!(prod_a, (tx1))
  fork!(prod_b, (tx2))
  select
    rx1 <! msg: msg
    rx2 <! msg: msg
    timeout 5000: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    let val = untag_smi(raw);
    assert!(
        val == 100 || val == 200,
        "select deve retornar 100 ou 200, got {val}"
    );
}

// ── Teste 2: select com timeout — nenhum produtor, timeout dispara ──

/// Main cria dois canais sem produtores. O select espera com timeout
/// de 100ms. Como nenhum canal terá dado, o timeout_body dispara e
/// retorna 999.
#[serial]
#[test]
fn select_timeout_dispara() {
    let src = r#"action main => Int
  let ch1 := channel!()
  let rx1 := ch1.1
  let ch2 := channel!()
  let rx2 := ch2.1
  select
    rx1 <! msg: msg
    rx2 <! msg: msg
    timeout 100: 999
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        999,
        "select com timeout deve retornar 999 (timeout_body)"
    );
}

// ── Teste 3: select sem timeout — deadlock se nenhum canal tem dado ──

/// Main cria dois canais sem produtores. O select sem timeout espera
/// indefinidamente. O scheduler detecta deadlock (run_queue vazia +
/// blocked sem deadlines) e retorna DEADLOCK_SENTINEL.
#[serial]
#[test]
fn select_sem_timeout_deadlock() {
    let src = r#"action main => Int
  let ch1 := channel!()
  let rx1 := ch1.1
  let ch2 := channel!()
  let rx2 := ch2.1
  select
    rx1 <! msg: msg
    rx2 <! msg: msg
main!()"#;
    let (raw, _ty) = eval_src(src);
    assert_eq!(
        raw, DEADLOCK_SENTINEL,
        "select sem timeout e sem producers deve detectar deadlock"
    );
}

// ── Teste 4: select com 3 receivers — primeiro canal pronto ──

/// Três canais, mas só o primeiro tem produtor. O select deve disparar
/// o braço do canal 1 (índice 0) e retornar 42.
#[serial]
#[test]
fn select_3_receivers_primeiro_pronto() {
    let src = r#"action prod (ch::Sender::Int) => Unit
  ch !> 42
  ()
action main => Int
  let ch1 := channel!()
  let tx1 := ch1.0
  let rx1 := ch1.1
  let ch2 := channel!()
  let rx2 := ch2.1
  let ch3 := channel!()
  let rx3 := ch3.1
  fork!(prod, (tx1))
  select
    rx1 <! msg: msg
    rx2 <! msg: msg
    rx3 <! msg: msg
    timeout 5000: 0
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        42,
        "select deve retornar 42 (único canal com dado é ch1)"
    );
}

// ── Teste 5: select com timeout — produtor envia antes do timeout ──

/// Produtor envia 55 em ch1. O select tem timeout de 5000ms, mas o
/// dado chega antes. O braço do canal dispara e retorna 55 (não o
/// timeout_body).
#[serial]
#[test]
fn select_dado_chega_antes_timeout() {
    let src = r#"action prod (ch::Sender::Int) => Unit
  ch !> 55
  ()
action main => Int
  let ch := channel!()
  let tx := ch.0
  let rx := ch.1
  fork!(prod, (tx))
  select
    rx <! msg: msg
    timeout 5000: 999
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        55,
        "select deve retornar 55 (dado chega antes do timeout)"
    );
}
