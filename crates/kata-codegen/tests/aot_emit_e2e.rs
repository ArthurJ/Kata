//! Testes E2E de emissão AOT — `aot_emit` produz object file válido.
//!
//! Pipeline completo até codegen: lex → parse → resolve → infer →
//! monomorphize → optimize → aot_emit. Verifica que os bytes retornados
//! são um object file válido no formato nativo do host (ELF no Linux,
//! Mach-O no macOS), com magic number correto e tamanho não-trivial.
//!
//! Não executa o binário — o linking é Fase 4. Aqui só validamos que o
//! Cranelift emite um object file parseável.

use kata_codegen::aot_emit;
use kata_core::InterfaceRegistry;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

/// Merge prelude + user resolved modules — mesmo helper dos testes JIT.
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
        interface_registry: InterfaceRegistry::new(),
        functions: user.functions,
        actions: user.actions,
    }
}

/// Roda o pipeline até `aot_emit` e retorna os bytes do object file.
fn emit_src(src: &str) -> Vec<u8> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    aot_emit(&typed).expect("aot_emit deve succeed")
}

/// Detecta o magic number esperado para o host.
///
/// - Linux/ELF: `0x7f E L F` (bytes `7f 45 4c 46`).
/// - macOS/Mach-O: `0xfeedface` (32-bit) ou `0xfeedfacf` (64-bit).
/// - Windows/COFF: não suportado pelo host de CI (Linux), mas documentado.
fn expected_magic() -> &'static [u8] {
    let target = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match (os, target) {
        ("linux", _) => &[0x7f, 0x45, 0x4c, 0x46], // ELF
        ("macos", "x86_64") | ("macos", "aarch64") => &[0xcf, 0xfa, 0xed, 0xfe], // Mach-O 64 (LE)
        _ => &[0x7f, 0x45, 0x4c, 0x46],            // fallback assume ELF
    }
}

/// `aot_emit` de uma expressão simples produz bytes não vazios com
/// magic number correto para o formato nativo do host.
#[test]
fn aot_emit_expressão_simples_produz_object_file() {
    let bytes = emit_src("+ 1 2");
    assert!(!bytes.is_empty(), "object file não pode ser vazio");

    // Tamanho razoável para um object file com prelude + 1 função.
    // Mesmo um programa trivial gera várias funções (FFI imports + entry).
    assert!(
        bytes.len() > 100,
        "object file muito pequeno ({} bytes) — suspeito",
        bytes.len()
    );

    // Verifica magic number do formato nativo.
    let magic = expected_magic();
    assert!(
        bytes.starts_with(magic),
        "magic number não corresponde ao esperado para {}: \
         bytes iniciam com {:02x?}, esperado {:02x?}",
        std::env::consts::OS,
        &bytes[..magic.len().min(bytes.len())],
        magic
    );
}

/// `aot_emit` de um programa com função nomeada + entry point também
/// emite bytes válidos — verifica que o lowering de funções Kata
/// (declare_kata_function + define_kata_function) funciona no AOT.
/// Usa sintaxe de `examples/fatorial.kata` (recursão com TCO).
#[test]
fn aot_emit_com_função_nomeada_emite_bytes() {
    let src = "\
fat :: Int Int => Int
    lambda 0 acc: acc
    lambda n acc: fat (- n 1) (* n acc)

fat 5 1";
    let bytes = emit_src(src);
    assert!(!bytes.is_empty());
    assert!(bytes.len() > 200, "object file pequeno demais");

    let magic = expected_magic();
    assert!(bytes.starts_with(magic), "magic number incorreto");
}

/// `aot_emit` de um programa com Action também emite bytes válidos —
/// verifica que o lowering de Actions funciona no AOT (Fase 4
/// linkará contra libkata_rt que tem o scheduler).
/// Usa sintaxe de `examples/hello_action.kata`.
#[test]
fn aot_emit_com_action_emite_bytes() {
    let src = "\
action greet
    echo!(\"hello\")
    echo!(\"world\")

greet!()";
    let bytes = emit_src(src);
    assert!(!bytes.is_empty());

    let magic = expected_magic();
    assert!(bytes.starts_with(magic), "magic number incorreto");
}

/// `aot_emit` é determinístico para a mesma entrada — duas chamadas
/// produzem bytes idênticos (mesma configuração de flags/isa).
/// Isso valida que não há estado compartilhado entre chamadas.
#[test]
fn aot_emit_é_determinístico() {
    let src = "+ 1 2";
    let bytes_a = emit_src(src);
    let bytes_b = emit_src(src);
    assert_eq!(
        bytes_a, bytes_b,
        "aot_emit deve ser determinístico para a mesma entrada"
    );
}

/// `aot_emit` de Text literal funciona — exercita o caminho de
/// declare_data + define_data para strings literais no AOT.
#[test]
fn aot_emit_com_text_literal_emite_bytes() {
    let src = "\"hello world\"";
    let bytes = emit_src(src);
    assert!(!bytes.is_empty());

    let magic = expected_magic();
    assert!(bytes.starts_with(magic), "magic number incorreto");
}
