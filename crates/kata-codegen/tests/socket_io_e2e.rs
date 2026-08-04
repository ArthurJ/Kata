//! Testes E2E de socket I/O.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: TCP open/listen/read/write/close, Unix domain socket, erros de modo,
//! read_chunk streaming, close automático no epílogo.
//!
//! Sockets são non-blocking obrigatórios. O scheduler cooperativo gerencia o bloqueio
//! via poll(POLLIN) + suspensão de fiber. Testes de TCP usam `fork!` para rodar
//! servidor e cliente em fibers separados (listen! bloqueia cooperativamente).

use kata_codegen::jit_eval;
use kata_codegen::type_table::build_and_register_type_table;
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
    let type_id_map =
        build_and_register_type_table(&typed, &typed.struct_registry, &resolved.enum_registry);
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

/// Desfaz SMI tagging: (raw >> 1).
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Sentinel de deadlock retornado por `kata_rt_run`.
const DEADLOCK_SENTINEL: i64 = i64::MIN + 1;

/// Gera uma porta aleatória para evitar colisão entre testes paralelos.
fn random_port() -> u16 {
    // Faixa de portas efêmeras alta — baixa probabilidade de colisão.
    let pid = std::process::id() as u64;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let port = 30000 + ((pid ^ ts) % 30000) as u16;
    port
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
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let dados := read!(conn, 100)
            match dados
              Result::Ok bytes:
                let n := len bytes
                close!(conn)
                tx !> n
              Result::Err msg:
                close!(conn)
                tx !> -1
          Result::Err msg: tx !> -2
      Result::Err msg: tx !> -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "hello")
        close!(sock)
        1
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    match (cliente!("{addr}"))
      n: rx <! result
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
// TCP — read em listener falha
// ═══════════════════════════════════════════════════════════════════

/// `read!` em um socket Listener deve retornar Err (não suporta read).
#[serial]
#[test]
fn socket_listener_read_fails() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action main => Int
    let result := open!(SocketKind::TCP("{addr}"), SocketMode::Listener)
    match result
      Result::Ok listener:
        let dados := read!(listener)
        match dados
          Result::Ok _: 0
          Result::Err _: -1
      Result::Err _: -2
main!()"#
    );

    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    let val = untag_smi(raw);
    assert_eq!(
        val, -1,
        "read em listener deve retornar Err (não suporta read)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — listen em connected falha
// ═══════════════════════════════════════════════════════════════════

/// `listen!` em um socket Connected deve retornar Err (não aceita conexões).
/// TODO: este teste deadlocka — o servidor faz `listen!(conn)` após accept,
/// mas o `tx !> -1` (send rendezvous) pode bloquear se o main ainda não
/// chegou em `rx <! result`. Investigar race condition.
#[serial]
#[test]
#[ignore = "race condition: tx !> -1 bloqueia antes do main chegar em rx <! result"]
fn socket_connected_listen_fails() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action servidor (addr::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
    match result
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let bad := listen!(conn)
            match bad
              Result::Ok _:
                close!(conn)
                tx !> 0
              Result::Err _:
                close!(conn)
                tx !> -1
          Result::Err _: tx !> -2
      Result::Err _: tx !> -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "hi")
        close!(sock)
        1
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let _ := cliente!("{addr}")
    rx <! result
    result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(val, -1, "listen em socket conectado deve retornar Err");
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
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let c1 := read!(conn, 4)
            match c1
              Result::Ok b1: match (read!(conn, 4))
                Result::Ok b2: match (read!(conn, 4))
                  Result::Ok b3: match (read!(conn, 4))
                    Result::Ok _: tx !> -10
                    Result::Err _: tx !> sum3!(len b1, len b2, len b3)
                  Result::Err _: tx !> -9
                Result::Err _: tx !> -8
              Result::Err _: tx !> -7
          Result::Err msg: tx !> -2
      Result::Err msg: tx !> -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "AAAAAAAAAA")
        close!(sock)
        1
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let _ := cliente!("{addr}")
    rx <! result
    result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(val, 10, "soma dos chunks deve ser 10 (4+4+2)");
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
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let dados := read!(conn, 100)
            match dados
              Result::Ok bytes:
                let _ := write!(conn, bytes)
                close!(conn)
                tx !> 1
              Result::Err msg:
                close!(conn)
                tx !> -1
          Result::Err msg: tx !> -2
      Result::Err msg: tx !> -3

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "ping")
        let dados := read!(sock, 100)
        match dados
          Result::Ok bytes:
            let n := len bytes
            close!(sock)
            n
          Result::Err msg:
            close!(sock)
            -5
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let n := cliente!("{addr}")
    rx <! _
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
// TCP — open falha em endereço inválido
// ═══════════════════════════════════════════════════════════════════

/// Abrir um listener em endereço inválido deve retornar Err.
#[serial]
#[test]
fn socket_open_invalid_addr_fails() {
    let src = r#"action main => Int
    let result := open!(SocketKind::TCP("invalid:not_a_port"), SocketMode::Listener)
    match result
      Result::Ok _: 0
      Result::Err _: -1
main!()"#;

    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        -1,
        "open com endereço inválido deve retornar Err"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — connect falha sem servidor
// ═══════════════════════════════════════════════════════════════════

/// Conectar a uma porta sem servidor deve retornar Err (connection refused).
#[serial]
#[test]
fn socket_connect_refused_fails() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action main => Int
    let result := open!(SocketKind::TCP("{addr}"), SocketMode::Connected)
    match result
      Result::Ok _: 0
      Result::Err _: -1
main!()"#
    );

    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        -1,
        "connect sem servidor deve retornar Err (connection refused)"
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
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            # Não chama close!(conn) — epílogo deve fechar
            let dados := read!(conn, 100)
            match dados
              Result::Ok bytes:
                let n := len bytes
                tx !> n
              Result::Err _: tx !> -1
          Result::Err _: tx !> -2
      Result::Err _: tx !> -3

action cliente (addr::Text) => Int
    # Não chama close!(sock) — epílogo deve fechar
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "epilogue")
        1
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{addr}", tx))
    let _ := cliente!("{addr}")
    rx <! result
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

// ═══════════════════════════════════════════════════════════════════
// Unix domain socket — listen + connect + roundtrip
// ═══════════════════════════════════════════════════════════════════

/// Unix domain socket: servidor binda num path, cliente conecta,
/// escreve, servidor lê.
#[serial]
#[test]
fn socket_unix_listen_connect_roundtrip() {
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = format!("/tmp/kata_test_unix_{pid}_{ts}.sock");

    let src = format!(
        r#"action servidor (path::Text, tx::Sender::Int) => Unit
    let result := open!(SocketKind::Unix(path), SocketMode::Listener)
    match result
      Result::Ok listener:
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let dados := read!(conn, 100)
            match dados
              Result::Ok bytes:
                let n := len bytes
                close!(conn)
                tx !> n
              Result::Err msg:
                close!(conn)
                tx !> -1
          Result::Err msg: tx !> -2
      Result::Err msg: tx !> -3

action cliente (path::Text) => Int
    let result := open!(SocketKind::Unix(path), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "unix hi")
        close!(sock)
        1
      Result::Err _: -4

action main => Int
    let ch := channel!()
    let tx := ch.0
    let rx := ch.1
    fork!(servidor, ("{path}", tx))
    let _ := cliente!("{path}")
    rx <! result
    result
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(
        val, 7,
        "servidor unix deve receber 7 bytes (\\\"unix hi\\\")"
    );
    let _ = std::fs::remove_file(&path);
}
