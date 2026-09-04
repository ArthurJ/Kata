//! Testes E2E de socket I/O — TCP streaming: read_chunk em múltiplos chunks,
//! e select (multiplexação socket + channel).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.

use kata_codegen::type_table::build_and_register_type_table;
use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_tree_shaking::tree_shake;
use serial_test::serial;

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    let (type_id_map, type_shapes) =
        build_and_register_type_table(&typed, &typed.struct_registry, &resolved.enum_registry);
    let jit = jit_eval(&typed, &type_id_map, &type_shapes, leak_rt_ptr(), false)
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

/// Desfaz SMI tagging: (raw >> 1).
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
use kata_rt::DEADLOCK_SENTINEL;

/// Gera uma porta aleatória para evitar colisão entre testes paralelos.
fn random_port() -> u16 {
    // Faixa de portas efêmeras alta — baixa probabilidade de colisão.
    let pid = std::process::id() as u64;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    30000 + ((pid ^ ts) % 30000) as u16
}

// ═══════════════════════════════════════════════════════════════════
// TCP — read_chunk streaming
// ═══════════════════════════════════════════════════════════════════

/// Cliente escreve 10 bytes. Servidor lê em chunk de 4 bytes.
/// Espera-se: chunk1=4, chunk2=4, chunk3=2, chunk4=EOF.
/// Retorna a soma (10) para validar.
#[serial]
#[test]
fn socket_read_chunk_streaming() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action sum3 (a::Int, b::Int, c::Int) => Int
  + (+ a b) c

action servidor (addr::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let c1 := read!(conn, 4)
            match c1
              Ok b1: match (read!(conn, 4))
                Ok b2: match (read!(conn, 4))
                  Ok b3: match (read!(conn, 4))
                    Ok _: tx <! -10
                    Err _: tx <! sum3!(len b1, len b2, len b3)
                  Err _: tx <! -9
                Err _: tx <! -8
              Err _: tx <! -7
          Err msg: tx <! -2
      Err msg: tx <! -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "AAAAAAAAAA")
        close!(sock)
        1
      Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let _ := cliente!("{addr}")
    rx !> result
    result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(val, 10, "soma dos chunks deve ser 10 (4+4+2)");
}

// ═══════════════════════════════════════════════════════════════════
// TCP — select com socket (multiplexação socket + channel)
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP abre listener, aceita conexão, e faz `select` entre:
///   - read!(conn, 100) do socket
///   - rx do canal IPC (nunca recebe)
///
/// O cliente conecta e escreve "select!" no socket. O braço do socket
/// dispara. O servidor lê os bytes, conta o tamanho, envia via canal.
///
/// O body de cada braço do select deve estar na mesma linha do `:`
/// (limitação do parser). Match aninhado vai em action auxiliar.
#[serial]
#[test]
fn socket_select_with_socket() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action extrair_n (r::Result::(Bytes, Text)) => Int
  match r
    Ok bytes: len bytes
    Err _: -1

action fazer_select (conn::Socket, tx::Sender::Int) => Unit
  select
    read!(conn, 100) !> dados: tx <! extrair_n!(dados)

action servidor (addr::Text, tx::Sender::Int) => Unit
  let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
  match result
    Ok listener:
      let client := listen!(listener)
      match client
        Ok conn: fazer_select!(conn, tx)
        Err _: tx <! -2
    Err _: tx <! -3

action cliente (addr::Text) => Int
  let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
  match result
    Ok sock:
      let _ := write!(sock, "select!")
      close!(sock)
      1
    Err _: -4

action main => Int
  let ch := channel!()
  let tx := ch.0
  let rx := ch.1
  fork!(servidor, ("{addr}", tx))
  let _ := cliente!("{addr}")
  rx !> result
  result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(
        val, 7,
        "select com socket deve disparar braço de read e receber 7 bytes (\"select!\")"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — select misto (channel + socket)
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP aceita conexão e faz `select` entre:
///   - rx do canal IPC (produtor envia 42)
///   - read!(conn, 100) do socket
///
/// Ambos estão prontos. O braço do canal aparece primeiro na ordem
/// e deve disparar (channel arms são testados antes de socket arms
/// no runtime). O servidor envia 42 via tx_result para o main.
///
/// O body de cada braço do select deve estar na mesma linha do `:`
/// (limitação do parser).
#[serial]
#[test]
fn socket_select_misto_channel_socket() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action extrair_n (r::Result::(Bytes, Text)) => Int
  match r
    Ok bytes: len bytes
    Err _: -1

action fazer_select (conn::Socket, rx::Receiver::Int, tx_result::Sender::Int) => Unit
  select
    rx !> msg: tx_result <! msg
    read!(conn, 100) !> dados: tx_result <! extrair_n!(dados)

action prod (tx::Sender::Int) => Unit
  tx <! 42
  ()

action servidor (addr::Text, rx::Receiver::Int, tx_result::Sender::Int) => Unit
  let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
  match result
    Ok listener:
      let client := listen!(listener)
      match client
        Ok conn: fazer_select!(conn, rx, tx_result)
        Err _: tx_result <! -2
    Err _: tx_result <! -3

action cliente (addr::Text) => Int
  let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
  match result
    Ok sock:
      let _ := write!(sock, "data")
      close!(sock)
      1
    Err _: -4

action main => Int
  let ch := channel!()
  let tx := ch.0
  let rx := ch.1
  let ch2 := channel!()
  let tx2 := ch2.0
  let rx2 := ch2.1
  fork!(servidor, ("{addr}", rx, tx2))
  fork!(prod, (tx))
  let _ := cliente!("{addr}")
  rx2 !> result
  result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(
        val, 42,
        "select misto channel+socket deve disparar braço do channel (42)"
    );
}
