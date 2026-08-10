//! Testes E2E de file I/O.
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: open, read, readline, write (Text e Bytes), close, echo para file.
//! Usa arquivos temporários em /tmp.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
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
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
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
    let jit = jit_eval(&typed, &Default::default(), &[], leak_rt_ptr()).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Cria um arquivo temporário com conteúdo, retorna o path.
fn make_temp_file(content: &str) -> String {
    let path = format!(
        "/tmp/kata_test_{}_{}.txt",
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
// open + read — lê arquivo texto como Bytes
// ═══════════════════════════════════════════════════════════════════

/// Abre um arquivo existente em modo Read e lê todo o conteúdo como Bytes.
#[test]
#[serial]
fn file_open_read_bytes() {
    let path = make_temp_file("Hello, World!");
    let src = format!(
        r#"action read_bytes_len (h::File) => Int
  let content := read!(h)
  match content
    Result::Ok bytes: len bytes
    Result::Err _: -1

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok handle: read_bytes_len!(handle)
    Result::Err _: -2
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(untag_smi(raw), 13, "deve ter 13 bytes");
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// open + readline — lê primeira linha como Text
// ═══════════════════════════════════════════════════════════════════

/// Abre um arquivo com múltiplas linhas e lê a primeira linha.
#[test]
#[serial]
fn file_open_readline_text() {
    let path = make_temp_file("first line\nsecond line\nthird line");
    let src = format!(
        r#"action readline_len (h::File) => Int
  let line := readline!(h)
  match line
    Result::Ok text: len text
    Result::Err _: -1

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok handle: readline_len!(handle)
    Result::Err _: -2
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        10,
        "primeira linha deve ter 10 chars (\"first line\")"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// write + read round-trip
// ═══════════════════════════════════════════════════════════════════

/// Escreve texto num arquivo e lê de volta como Bytes.
#[test]
#[serial]
fn file_write_read_roundtrip() {
    let path = format!(
        "/tmp/kata_test_wr_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let src = format!(
        r#"action read_back (p::Text) => Int
  let f2 := open!(p, FileMode::Read)
  match f2
    Result::Ok h2: match (read!(h2))
      Result::Ok bytes: len bytes
      Result::Err _: -1
    Result::Err _: -2

action do_write (h::File, p::Text) => Int
  let _ := write!(h, "test content")
  close!(h)
  read_back!(p)

action main => Int
  let f := open!("{path}", FileMode::Write)
  match f
    Result::Ok handle: do_write!(handle, "{path}")
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(untag_smi(raw), 12, "deve ter 12 bytes (\"test content\")");
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// FileMode::Create — falha se arquivo existe
// ═══════════════════════════════════════════════════════════════════

/// Create falha se o arquivo já existe (retorna Err).
#[test]
#[serial]
fn file_create_falha_se_existe() {
    let path = make_temp_file("already exists");
    let src = format!(
        r#"action main => Int
  let f := open!("{path}", FileMode::Create)
  match f
    Result::Ok _: 0
    Result::Err _: -1
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -1,
        "Create deve falhar quando arquivo existe"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// FileMode::Create — sucesso se arquivo não existe
// ═══════════════════════════════════════════════════════════════════

/// Create sucede se o arquivo não existe.
#[test]
#[serial]
fn file_create_sucesso_se_nao_existe() {
    let path = format!(
        "/tmp/kata_test_create_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let src = format!(
        r#"action close_and_one (h::File) => Int
  close!(h)
  1

action main => Int
  let f := open!("{path}", FileMode::Create)
  match f
    Result::Ok handle: close_and_one!(handle)
    Result::Err _: -1
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        1,
        "Create deve suceder quando arquivo não existe"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// echo para file — escreve show(msg) + newline
// ═══════════════════════════════════════════════════════════════════

/// echo escreve show(42) + newline num arquivo e lê de volta.
#[test]
#[serial]
fn file_echo_writes_show_plus_newline() {
    let path = format!(
        "/tmp/kata_test_echo_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let src = format!(
        r#"action read_back (p::Text) => Int
  let f2 := open!(p, FileMode::Read)
  match f2
    Result::Ok h2: match (read!(h2))
      Result::Ok bytes: len bytes
      Result::Err _: -1
    Result::Err _: -2

action do_echo (h::File, p::Text) => Int
  echo!(42, h)
  close!(h)
  read_back!(p)

action main => Int
  let f := open!("{path}", FileMode::Write)
  match f
    Result::Ok handle: do_echo!(handle, "{path}")
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    // show(42) = "42" (2 chars) + "\n" (1 char) = 3 bytes
    assert_eq!(untag_smi(raw), 3, "echo deve escrever \"42\\n\" (3 bytes)");
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// open arquivo inexistente — retorna Err
// ═══════════════════════════════════════════════════════════════════

/// Abrir arquivo inexistente em modo Read retorna Err.
#[test]
#[serial]
fn file_open_inexistente_retorna_err() {
    let src = r#"action main => Int
  let f := open!("/tmp/kata_nao_existe_99999.txt", FileMode::Read)
  match f
    Result::Ok _: 0
    Result::Err _: -1
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -1,
        "abrir arquivo inexistente deve retornar Err"
    );
}

// ═══════════════════════════════════════════════════════════════════
// read_chunk — streaming de arquivo em chunks
// ═══════════════════════════════════════════════════════════════════

/// Lê um arquivo de 10000 bytes em chunks de 4096.
/// Espera-se: chunk1=4096, chunk2=4096, chunk3=1808, chunk4=EOF.
/// Retorna a soma dos tamanhos (10000) para validar.
#[test]
#[serial]
fn file_read_chunk_streaming() {
    // 10000 bytes de conteúdo
    let content = "A".repeat(10000);
    let path = make_temp_file(&content);
    let src = format!(
        r#"action sum3 (a::Int, b::Int, c::Int) => Int
  + (+ a b) c

action read_all_chunks (h::File) => Int
  let c1 := read!(h, 4096)
  match c1
    Result::Ok b1: match (read!(h, 4096))
      Result::Ok b2: match (read!(h, 4096))
        Result::Ok b3: match (read!(h, 4096))
          Result::Ok _: -10
          Result::Err _: sum3!(len b1, len b2, len b3)
        Result::Err _: -9
      Result::Err _: -8
    Result::Err _: -7

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok handle: read_all_chunks!(handle)
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        10000,
        "soma dos chunks deve ser 10000 (4096+4096+1808)"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// read_chunk + readline intercalados — BufReader persistente
// ═══════════════════════════════════════════════════════════════════

/// Intercala read_chunk e readline no mesmo arquivo para verificar
/// que o BufReader persistente preserva o estado de leitura.
///
/// Arquivo: "AAAAABBBBB\nCCCCC" (12 bytes)
/// 1. read_chunk(5) → "AAAAA" (5 bytes)
/// 2. readline() → "BBBBB" (5 chars, \n consumido)
/// 3. read_chunk(5) → "CCCCC" (5 bytes)
/// 4. read_chunk(1) → EOF
///
/// Se o BufReader não fosse persistente, o readline recriaria o buffer
/// e perderia os bytes após "AAAAABBBBB\n" que já foram bufferizados
/// pelo read_chunk anterior. Ou o read_chunk ignoraria bytes que o
/// readline bufferizou.
#[test]
#[serial]
fn file_read_chunk_readline_intercalados() {
    let path = make_temp_file("AAAAABBBBB\nCCCCC");
    let src = format!(
        r#"action sum3 (a::Int, b::Int, c::Int) => Int
  + (+ a b) c

action interleave (h::File) => Int
  let c1 := read!(h, 5)
  match c1
    Result::Ok b1: match (readline!(h))
      Result::Ok l1: match (read!(h, 5))
        Result::Ok b2: match (read!(h, 1))
          Result::Ok _: -10
          Result::Err _: sum3!(len b1, len l1, len b2)
        Result::Err _: -9
      Result::Err _: -8
    Result::Err _: -7

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok handle: interleave!(handle)
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    // 5 (chunk "AAAAA") + 5 (readline "BBBBB") + 5 (chunk "CCCCC") = 15
    assert_eq!(
        untag_smi(raw),
        15,
        "intercalação read_chunk+readline deve preservar estado do BufReader"
    );
    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════
// read_chunk EOF imediato — arquivo vazio
// ═══════════════════════════════════════════════════════════════════

/// read_chunk num arquivo vazio retorna EOF imediatamente.
#[test]
#[serial]
fn file_read_chunk_eof_imediato() {
    let path = make_temp_file("");
    let src = format!(
        r#"action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok handle: match (read!(handle, 100))
      Result::Ok _: 0
      Result::Err _: -1
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::int(), "deve retornar Int");
    assert_eq!(
        untag_smi(raw),
        -1,
        "read_chunk em arquivo vazio deve retornar EOF"
    );
    let _ = std::fs::remove_file(&path);
}
