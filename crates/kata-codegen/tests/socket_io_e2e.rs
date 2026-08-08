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
        directive_registry: kata_resolution::DirectiveRegistry::new(),
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
///
/// O main abre um listener, fork um cliente que conecta e fecha, o main
/// aceita a conexão (produzindo um socket Connected) e tenta `listen!(conn)`
/// — deve retornar Err imediatamente. Sem canal: o main retorna o resultado
/// diretamente, evitando o deadlock do `channel!()` (rendezvous) onde
/// `tx !> -1` bloqueia se o main ainda não chegou em `rx <! result`.
#[serial]
#[test]
fn socket_connected_listen_fails() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "hi")
        close!(sock)
        1
      Result::Err _: -4

action main => Int
    let result := open!(SocketKind::TCP("{addr}"), SocketMode::Listener)
    match result
      Result::Ok listener:
        fork!(cliente, ("{addr}"))
        let client := listen!(listener)
        match client
          Result::Ok conn:
            let bad := listen!(conn)
            match bad
              Result::Ok _:
                close!(conn)
                0
              Result::Err _:
                close!(conn)
                -1
          Result::Err _: -2
      Result::Err _: -3
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
    Result::Ok bytes: len bytes
    Result::Err _: -1

action fazer_select (conn::Socket, tx::Sender::Int) => Unit
  select
    read!(conn, 100) <! dados: tx !> extrair_n!(dados)

action servidor (addr::Text, tx::Sender::Int) => Unit
  let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
  match result
    Result::Ok listener:
      let client := listen!(listener)
      match client
        Result::Ok conn: fazer_select!(conn, tx)
        Result::Err _: tx !> -2
    Result::Err _: tx !> -3

action cliente (addr::Text) => Int
  let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
  match result
    Result::Ok sock:
      let _ := write!(sock, "select!")
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
    Result::Ok bytes: len bytes
    Result::Err _: -1

action fazer_select (conn::Socket, rx::Receiver::Int, tx_result::Sender::Int) => Unit
  select
    rx <! msg: tx_result !> msg
    read!(conn, 100) <! dados: tx_result !> extrair_n!(dados)

action prod (tx::Sender::Int) => Unit
  tx !> 42
  ()

action servidor (addr::Text, rx::Receiver::Int, tx_result::Sender::Int) => Unit
  let result := open!(SocketKind::TCP(addr), SocketMode::Listener)
  match result
    Result::Ok listener:
      let client := listen!(listener)
      match client
        Result::Ok conn: fazer_select!(conn, rx, tx_result)
        Result::Err _: tx_result !> -2
    Result::Err _: tx_result !> -3

action cliente (addr::Text) => Int
  let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
  match result
    Result::Ok sock:
      let _ := write!(sock, "data")
      close!(sock)
      1
    Result::Err _: -4

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
  rx2 <! result
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

// ═══════════════════════════════════════════════════════════════════
// TCP — readline: linha única
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP: aceita conexão, usa `readline!` para ler uma linha,
/// envia o tamanho da linha de volta via canal IPC. Cliente: conecta,
/// escreve "hello\n", fecha.
///
/// Testa o caso base: uma linha completa enviada em um único write.
#[serial]
#[test]
fn socket_readline_single_line() {
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
            let line := readline!(conn)
            match line
              Result::Ok text:
                let n := len text
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
        let _ := write!(sock, "hello\n")
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
    assert_eq!(
        val, 5,
        "readline! deve retornar \"hello\" (5 chars) sem o \\n"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — readline: múltiplas linhas
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP: aceita conexão, lê duas linhas com `readline!`,
/// envia o tamanho da segunda linha via canal. Cliente: conecta,
/// escreve "foo\nbar\n", fecha.
///
/// Testa que o buffer parcial persiste entre chamadas de readline!
/// — a segunda chamada não relê do FD se os bytes já estão no buffer.
#[serial]
#[test]
fn socket_readline_multiple_lines() {
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
            let line1 := readline!(conn)
            match line1
              Result::Ok t1:
                let line2 := readline!(conn)
                match line2
                  Result::Ok t2:
                    let n2 := len t2
                    close!(conn)
                    tx !> n2
                  Result::Err msg:
                    close!(conn)
                    tx !> -1
              Result::Err msg:
                close!(conn)
                tx !> -2
          Result::Err msg: tx !> -3
      Result::Err msg: tx !> -4

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Result::Ok sock:
        let _ := write!(sock, "foo\nbar\n")
        close!(sock)
        1
      Result::Err _: -5

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
        val, 3,
        "segunda readline! deve retornar \"bar\" (3 chars) — buffer parcial persiste"
    );
}

// ═══════════════════════════════════════════════════════════════════
// TCP — readline: EOF em linha parcial (sem \n)
// ═══════════════════════════════════════════════════════════════════

/// Servidor TCP: aceita conexão, faz readline! — o cliente envia
/// "partial" sem \n e fecha. O servidor deve receber a linha parcial
/// como Ok("partial") (EOF com dados no buffer).
#[serial]
#[test]
fn socket_readline_eof_partial() {
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
            let line := readline!(conn)
            match line
              Result::Ok text:
                let n := len text
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
        let _ := write!(sock, "partial")
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
    assert_eq!(
        val, 7,
        "readline! com EOF deve retornar linha parcial \"partial\" (7 chars)"
    );
}
