//! Testes E2E de ReadResult — EOF tipado em leitura.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: read/readline em arquivo vazio (Eof), com dados (Ok), e match exaustivo.

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

/// Combina prelude + módulo do usuário (replica do driver com merge completo).
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

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);
    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);
    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
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
        embed_dependencies: Vec::new(),
    }
}

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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false)
        .expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Tenta inferir (espera erro). Retorna a mensagem de erro.
fn infer_err(src: &str) -> String {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    match infer_module(&module, &resolved) {
        Ok(_) => panic!("infer deve falhar para match não-exaustivo"),
        Err(e) => format!("{e:?}"),
    }
}

/// Cria um arquivo temporário com conteúdo, retorna o path.
fn make_temp_file(content: &str) -> String {
    let path = format!(
        "/tmp/kata_test_rr_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::fs::write(&path, content).expect("deve escrever arquivo temp");
    path
}

/// Desfaz SMI tagging: (raw >> 1).
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ═══════════════════════════════════════════════════════════════════
// read! em arquivo vazio → Eof
// ═══════════════════════════════════════════════════════════════════

/// `read!(file)` em arquivo vazio retorna `Eof` (tag 2), não `Err`.
/// O match cobre os 3 casos: Ok, Error, Eof. Retorna:
/// - Ok: len bytes (positivo)
/// - Error: -1
/// - Eof: -2
#[test]
#[serial]
fn read_result_eof_file_read() {
    let path = make_temp_file("");
    let src = format!(
        r#"action read_len (h::File) => Int
  let content := read!(h)
  match content
    Data bytes: len bytes
    Error _: -1
    Eof: -2

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: read_len!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -2,
        "read em arquivo vazio deve retornar Eof (-2)"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// readline! em arquivo vazio → Eof
// ═══════════════════════════════════════════════════════════════════

/// `readline!(file)` em arquivo vazio retorna `Eof`.
#[test]
#[serial]
fn read_result_eof_file_readline() {
    let path = make_temp_file("");
    let src = format!(
        r#"action readline_check (h::File) => Int
  let line := readline!(h)
  match line
    Data text: len text
    Error _: -1
    Eof: -2

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: readline_check!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -2,
        "readline em arquivo vazio deve retornar Eof (-2)"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// read! em arquivo com dados → Ok
// ═══════════════════════════════════════════════════════════════════

/// `read!(file)` em arquivo com 13 bytes retorna `Ok(bytes)` com len=13.
#[test]
#[serial]
fn read_result_ok_data() {
    let path = make_temp_file("Hello, World!");
    let src = format!(
        r#"action read_len (h::File) => Int
  let content := read!(h)
  match content
    Data bytes: len bytes
    Error _: -1
    Eof: -2

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: read_len!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        13,
        "read em arquivo com 13 bytes deve retornar Ok com len=13"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// read! com n em arquivo vazio → Eof
// ═══════════════════════════════════════════════════════════════════

/// `read!(file, 100)` em arquivo vazio retorna `Eof`.
#[test]
#[serial]
fn read_result_eof_file_read_chunk() {
    let path = make_temp_file("");
    let src = format!(
        r#"action read_chunk_check (h::File) => Int
  let content := read!(h, 100)
  match content
    Data bytes: len bytes
    Error _: -1
    Eof: -2

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: read_chunk_check!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -2,
        "read_chunk em arquivo vazio deve retornar Eof (-2)"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// Match não-exaustivo (sem Eof) → erro de inferência
// ═══════════════════════════════════════════════════════════════════

/// Match em ReadResult sem cobrir `Eof` deve falhar na inferência
/// com erro de exaustividade.
#[test]
#[serial]
fn read_result_match_nao_exaustivo() {
    let path = make_temp_file("dados");
    let src = format!(
        r#"action read_len (h::File) => Int
  let content := read!(h)
  match content
    Data bytes: len bytes
    Error _: -1

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: read_len!(handle)
    Err _: -3
main!()"#
    );
    let err = infer_err(&src);
    assert!(
        err.contains("NonExhaustive") || err.contains("exaust") || err.contains("Eof"),
        "erro deve mencionar exaustividade ou Eof faltante, got: {err}"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// Loop de leitura até Eof — pattern comum de I/O
// ═══════════════════════════════════════════════════════════════════

/// Lê um arquivo linha por linha até Eof, conta o número de linhas.
/// Exercita o pattern comum: loop com match em ReadResult.
#[test]
#[serial]
fn read_result_loop_ate_eof() {
    let path = make_temp_file("linha1\nlinha2\nlinha3\n");
    let src = format!(
        r#"action count_lines (h::File) => Int
  var n := 0
  loop
    match (readline!(h))
      Data _: n := + n 1
      Error _: break
      Eof: break
  n

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: count_lines!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(untag_smi(raw), 3, "deve contar 3 linhas antes de Eof");
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// read! em arquivo com dados → Eof na segunda leitura
// ═══════════════════════════════════════════════════════════════════

/// Primeiro read retorna Ok, segundo read retorna Eof (stream acabou).
#[test]
#[serial]
fn read_result_eof_segunda_leitura() {
    let path = make_temp_file("dados");
    let src = format!(
        r#"action read_twice (h::File) => Int
  let first := read!(h)
  match first
    Data _:
      let second := read!(h)
      match second
        Data _: 1
        Error _: -1
        Eof: 0
    Error _: -1
    Eof: -2

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Ok handle: read_twice!(handle)
    Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        0,
        "segundo read após esgotar stream deve retornar Eof (0)"
    );
    let _ = std::fs::remove_file(&path);
}
