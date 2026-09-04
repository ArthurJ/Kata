//! Testes E2E de socket I/O — TCP básico: open/listen/connect roundtrip,
//! echo server, e close automático no epílogo.
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
// TCP — open + listen + write + read roundtrip
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP: abre listener, aceita conexão (listen!), lê dados,
/// fecha conexão. Cliente: conecta, escreve "hello", fecha.
///
/// O servidor e cliente rodam em fibers separados via fork!. O entry
/// point faz fork! do servidor, conecta como cliente, e retorna o
/// tamanho dos bytes lidos pelo servidor.
///
/// Estratégia: o entry point é o cliente. Ele faz fork! do servidor
/// (que vai bloquear em listen!), conecta, escreve, fecha. O servidor
/// lê, conta bytes, e envia o resultado via canal IPC. O cliente
/// recebe o resultado e retorna.
#[serial]
#[test]
fn socket_tcp_listen_connect_roundtrip() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action servidor (addr::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let dados := read!(conn, 100)
            match dados
              Ok bytes:
                let n := len bytes
                close!(conn)
                tx <! n
              Err msg:
                close!(conn)
                tx <! -1
          Err msg: tx <! -2
      Err msg: tx <! -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "hello")
        close!(sock)
        1
      Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    match (cliente!("{addr}"))
      n: rx !> result
            result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(
        val, 5,
        "servidor deve receber 5 bytes (\\\"hello\\\") do cliente TCP"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — echo server com fork! por conexão
// ═══════════════════════════════════════════════════════════════════

/// Servidor echo: aceita conexão, lê dados, escreve de volta, fecha.
/// Cliente: conecta, escreve "ping", lê echo, verifica tamanho, fecha.
#[serial]
#[test]
fn socket_tcp_echo_server() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action servidor (addr::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let dados := read!(conn, 100)
            match dados
              Ok bytes:
                let _ := write!(conn, bytes)
                close!(conn)
                tx <! 1
              Err msg:
                close!(conn)
                tx <! -1
          Err msg: tx <! -2
      Err msg: tx <! -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "ping")
        let dados := read!(sock, 100)
        match dados
          Ok bytes:
            let n := len bytes
            close!(sock)
            n
          Err msg:
            close!(sock)
            -5
      Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let n := cliente!("{addr}")
    rx !> _
    n
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(
        val, 4,
        "cliente deve receber 4 bytes de echo (\\\"ping\\\")"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — close automático no epílogo
// ═══════════════════════════════════════════════════════════════════

/// Abre um socket Connected sem chamar close! explicitamente.
/// O epílogo da action deve fechar automaticamente.
/// Verificamos que não crasha (o close idempotente no epílogo é no-op
/// se o programador já fechou, mas aqui não fechamos).
#[serial]
#[test]
fn socket_close_epilogo() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action servidor (addr::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            # Não chama close!(conn) — epílogo deve fechar
            let dados := read!(conn, 100)
            match dados
              Ok bytes:
                let n := len bytes
                tx <! n
              Err _: tx <! -1
          Err _: tx <! -2
      Err _: tx <! -3

action cliente (addr::Text) => Int
    # Não chama close!(sock) — epílogo deve fechar
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "epilogue")
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
        val, 8,
        "servidor deve receber 8 bytes (\\\"epilogue\\\") — epílogo fecha sockets"
    );
}
