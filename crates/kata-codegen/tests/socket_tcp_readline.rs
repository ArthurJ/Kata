//! Testes E2E de socket I/O — TCP readline: linha única, múltiplas linhas,
//! e EOF em linha parcial (sem \n).
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
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let line := readline!(conn)
            match line
              Ok text:
                let n := len text
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
        let _ := write!(sock, "hello\n")
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
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let line1 := readline!(conn)
            match line1
              Ok t1:
                let line2 := readline!(conn)
                match line2
                  Ok t2:
                    let n2 := len t2
                    close!(conn)
                    tx <! n2
                  Err msg:
                    close!(conn)
                    tx <! -1
              Err msg:
                close!(conn)
                tx <! -2
          Err msg: tx <! -3
      Err msg: tx <! -4

action cliente (addr::Text) => Int
    let result := open!(SocketKind::TCP(addr), SocketMode::Connected)
    match result
      Ok sock:
        let _ := write!(sock, "foo\nbar\n")
        close!(sock)
        1
      Err _: -5

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
      Ok listener:
        let client := listen!(listener)
        match client
          Ok conn:
            let line := readline!(conn)
            match line
              Ok text:
                let n := len text
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
        let _ := write!(sock, "partial")
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
        "readline! com EOF deve retornar linha parcial \"partial\" (7 chars)"
    );
}
