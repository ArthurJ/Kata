//! Testes E2E de socket I/O — TCP erros: modo incorreto (read em listener,
//! listen em connected), endereço inválido, connect refused.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.

use kata_codegen::type_table::build_and_register_type_table;
use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::{PrimTy, Ty};
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
      Ok listener:
        let dados := read!(listener)
        match dados
          Ok _: 0
          Err _: -1
      Err _: -2
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
/// `tx <! -1` bloqueia se o main ainda não chegou em `rx !> result`.
#[serial]
#[test]
fn socket_connected_listen_fails() {
    let port = random_port();
    let addr = format!("127.0.0.1:{port}");

    let src = format!(
        r#"action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "hi")
        close!(sock)
        1
      Err _: -4

action main => Int
    let result := open!(SocketKind::TCP("{addr}"), SocketMode::Listener)
    match result
      Ok listener:
        fork!(cliente, ("{addr}"))
        let client := listen!(listener)
        match client
          Ok conn:
            let bad := listen!(conn)
            match bad
              Ok _:
                close!(conn)
                0
              Err _:
                close!(conn)
                -1
          Err _: -2
      Err _: -3
main!()"#
    );

    let (raw, _ty) = eval_src(&src);
    assert_ne!(raw, DEADLOCK_SENTINEL, "não deve deadlockar");
    let val = untag_smi(raw);
    assert_eq!(val, -1, "listen em socket conectado deve retornar Err");
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
      Ok _: 0
      Err _: -1
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
      Ok _: 0
      Err _: -1
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
