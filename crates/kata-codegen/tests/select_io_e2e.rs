//! Testes E2E de select com I/O (files) e select misto (channels + files).
//!
//! Pipeline completo: lex → parse → resolve → infer → monomorphize → optimize → codegen → JIT.
//! Cobertura: select só-files, select misto (channel + file), select com EOF, select com timeout.

use kata_codegen::jit_eval;
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
    let jit = jit_eval(&typed, &Default::default()).expect("codegen+JIT deve succeed");
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

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Cria um arquivo temporário com conteúdo, retorna o path.
fn make_temp_file(content: &str) -> String {
    let path = format!(
        "/tmp/kata_test_sel_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::fs::write(&path, content).expect("deve escrever arquivo temp");
    path
}

// ── Teste 1: select só de files — dois arquivos, um braço dispara ──

/// Dois arquivos com conteúdo. O select faz read! em ambos.
/// Arquivos regulares sempre estão "prontos" para leitura (poll POLLIN).
/// O primeiro braço (índice 0) dispara e retorna o tamanho do chunk lido.
#[serial]
#[test]
fn select_files_only() {
    let path1 = make_temp_file("Hello");
    let path2 = make_temp_file("World!");
    let src = format!(
        r#"action open_and_select (p1::Text, p2::Text) => Int
  let f1 := open!(p1, FileMode::Read)
  let f2 := open!(p2, FileMode::Read)
  match f1
    Result::Ok h1:
      match f2
        Result::Ok h2:
          select
            read!(h1, 100) <! chunk: 5
            read!(h2, 100) <! chunk: 6
        Result::Err _: -4
    Result::Err _: -3

action main => Int
  open_and_select!("{path1}", "{path2}")
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    let val = untag_smi(raw);
    // Arquivos regulares: poll retorna POLLIN para ambos. O primeiro
    // braço na ordem dispara. "Hello" tem 5 bytes.
    assert!(
        val == 5 || val == 6,
        "select files deve retornar 5 (Hello) ou 6 (World!), got {val}"
    );
    let _ = std::fs::remove_file(&path1);
    let _ = std::fs::remove_file(&path2);
}

// ── Teste 2: select misto — channel + file ──

/// Um canal com produtor (fork) e um arquivo com conteúdo.
/// O select multiplexa entre receiver e file read.
/// Ambos estão prontos — o primeiro braço (channel) dispara.
#[serial]
#[test]
fn select_misto_channel_file() {
    let path = make_temp_file("file data");
    let src = format!(
        r#"action chunk_len (r::Result::(Bytes, Text)) => Int
  match r
    Result::Ok bytes: len bytes
    Result::Err _: -1

action prod (ch::Sender::Int) => Unit
  ch !> 42
  ()
action do_select (rx::Receiver::Int, h::File) => Int
  select
    rx <! msg: msg
    read!(h, 100) <! chunk: chunk_len!(chunk)
action main => Int
  let ch := channel!()
  let tx := ch.0
  let rx := ch.1
  fork!(prod, (tx))
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok h: do_select!(rx, h)
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    let val = untag_smi(raw);
    // O canal está pronto (produtor envia 42). O file também está pronto
    // (arquivo regular). A ordem de aparição é channel primeiro.
    assert!(
        val == 42 || val == 9,
        "select misto deve retornar 42 (channel) ou 9 (file \"file data\"), got {val}"
    );
    let _ = std::fs::remove_file(&path);
}

// ── Teste 3: select com EOF — arquivo no fim, braço dispara com Err ──

/// Arquivo vazio — read! retorna Err("EOF") (0 bytes lidos).
/// O braço de I/O dispara, e o body (chunk_len) retorna -99.
#[serial]
#[test]
fn select_file_eof() {
    let path = make_temp_file("");
    let src = format!(
        r#"action chunk_len (r::Result::(Bytes, Text)) => Int
  match r
    Result::Ok bytes: len bytes
    Result::Err _: -99

action do_select (h::File) => Int
  select
    read!(h, 100) <! chunk: chunk_len!(chunk)
    timeout 5000: 0

action main => Int
  let f := open!("{path}", FileMode::Read)
  match f
    Result::Ok h: do_select!(h)
    Result::Err _: -3
main!()"#
    );
    let (raw, ty) = eval_src(&src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // Arquivo vazio: read_chunk retorna Err("EOF") → chunk_len retorna -99.
    assert_eq!(
        untag_smi(raw),
        -99,
        "select com EOF deve retornar -99 (Err do read)"
    );
    let _ = std::fs::remove_file(&path);
}

// ── Teste 4: select com timeout — nenhum handle pronto ──

/// Canal sem produtor. O timeout dispara após 100ms.
#[serial]
#[test]
fn select_io_timeout() {
    let src = r#"action main => Int
  let ch := channel!()
  let rx := ch.1
  select
    rx <! msg: msg
    timeout 100: 999
main!()"#;
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // Canal sem produtor, timeout deve disparar após 100ms.
    assert_eq!(
        untag_smi(raw),
        999,
        "select io timeout deve retornar 999 (timeout)"
    );
}
