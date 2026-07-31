//! Testes E2E do codegen de wrappers `__kata_test_*`.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen.
//! Usa `jit_compile_tests` que compila sem executar e retorna
//! `(JITModule, Vec<TestWrapper>)`. Verifica que os wrappers são
//! gerados com a identidade semântica correta (action_name, test_index)
//! e que negativos CompileError recebem func_id placeholder.

use cranelift_module::FuncId;
use kata_codegen::{TestWrapper, jit_compile_tests};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake_preserve_tests;

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
    }
}

/// Pipeline completo até `jit_compile_tests`.
fn compile_tests(src: &str) -> (cranelift_jit::JITModule, Vec<TestWrapper>) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake_preserve_tests(typed.inner));
    jit_compile_tests(&typed, &Default::default()).expect("codegen+JIT deve succeed")
}

/// Encontra um wrapper por action_name e test_index.
fn find_wrapper<'a>(wrappers: &'a [TestWrapper], name: &str, idx: usize) -> &'a TestWrapper {
    wrappers
        .iter()
        .find(|w| w.action_name == name && w.test_index == idx)
        .unwrap_or_else(|| panic!("wrapper não encontrado: {name}[{idx}]"))
}

// ── Teste 1: @test("desc") sem args gera wrapper com desc correta ──

/// Action sem params com `@test("desc")` — wrapper gerado, args = None.
#[test]
fn test_wrapper_desc_sem_args() {
    let src = r#"@test("resposta da vida")
action resposta => Int
    42
resposta!()"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1, "deve ter 1 wrapper");

    let w = find_wrapper(&wrappers, "resposta", 0);
    assert_eq!(w.action_name, "resposta");
    assert_eq!(w.test_index, 0);
    assert_eq!(
        w.spec.desc.as_deref(),
        Some("resposta da vida"),
        "desc deve casar"
    );
    assert!(w.spec.args.is_none(), "args deve ser None");
    assert!(w.spec.expects.is_none(), "expects deve ser None");
}

// ── Teste 2: @test{args: (1, 2)} gera wrapper com args tipados ──

/// Action com 2 params Int, `@test{args: (3, 4)}` — wrapper com args tipados.
#[test]
fn test_wrapper_com_args_tupla() {
    let src = r#"@test{desc: "soma 3+4", args: (3, 4)}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1);

    let w = find_wrapper(&wrappers, "soma", 0);
    assert_eq!(w.action_name, "soma");
    assert_eq!(w.spec.desc.as_deref(), Some("soma 3+4"));
    assert!(w.spec.args.is_some(), "args deve ser Some");
}

// ── Teste 3: múltiplos @test na mesma action geram múltiplos wrappers ──

/// Action com 2 casos `@test` — 2 wrappers com test_index 0 e 1.
#[test]
fn test_wrapper_multiplos_tests_mesma_action() {
    let src = r#"@test("caso 1")
@test("caso 2")
action res => Int
    7
res!()"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 2, "deve ter 2 wrappers");

    let w0 = find_wrapper(&wrappers, "res", 0);
    assert_eq!(w0.spec.desc.as_deref(), Some("caso 1"));

    let w1 = find_wrapper(&wrappers, "res", 1);
    assert_eq!(w1.spec.desc.as_deref(), Some("caso 2"));
}

// ── Teste 4: @test com expects CompileError tem func_id placeholder ──

/// Negativo `expects: "CompileError: ..."` não gera wrapper — func_id é
/// `FuncId::from_u32(0)` (placeholder). O driver compila sub-módulo isolado.
/// O módulo em si compila normalmente — a action é válida, o `expects`
/// indica que um sub-módulo do teste deve falhar.
#[test]
fn test_wrapper_negativo_compileerror_placeholder() {
    let src = r#"@test{desc: "tipo errado", expects: "CompileError: TypeMismatch"}
action valida => Int
    42
valida!()"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1);

    let w = find_wrapper(&wrappers, "valida", 0);
    assert_eq!(
        w.spec.expects.as_deref(),
        Some("CompileError: TypeMismatch"),
        "expects deve ter prefixo CompileError"
    );
    // func_id placeholder — não deve ser chamado pelo driver.
    assert_eq!(
        w.func_id,
        FuncId::from_u32(0),
        "negativo CompileError deve ter func_id placeholder"
    );
}

// ── Teste 5: @test com timeout — spec carrega timeout ──

/// `@test{timeout: 5000}` — spec tem timeout definido.
#[test]
fn test_wrapper_com_timeout() {
    let src = r#"@test{desc: "com timeout", timeout: 5000}
action rapida => Int
    1
rapida!()"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1);

    let w = find_wrapper(&wrappers, "rapida", 0);
    assert_eq!(w.spec.timeout, Some(5000), "timeout deve ser 5000");
}

// ── Teste 6: wrappers positivos têm func_id válido (não placeholder) ──

/// Wrappers positivos (sem CompileError) devem ter func_id != placeholder.
/// `FuncId::from_u32(0)` é reservado para negativos.
#[test]
fn test_wrapper_positivo_tem_func_id_valido() {
    let src = r#"@test("positivo")
action ok => Int
    1
ok!()"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1);

    let w = find_wrapper(&wrappers, "ok", 0);
    assert_ne!(
        w.func_id,
        FuncId::from_u32(0),
        "wrapper positivo deve ter func_id válido (não placeholder)"
    );
}

// ── Teste 7: @test{args: ({"a": 1, "b": 2})} — Dict como arg de diretiva ──

/// `@test{args: (...)}` com DictLit dentro da tupla de args. Verifica que
/// o parser de diretivas aceita DictLit, o typeck infere Dict::(Text, Int),
/// e o codegen gera o wrapper sem erros.
#[test]
fn test_wrapper_com_dict_como_arg() {
    let src = r#"@test{desc: "dict como arg", args: ({"a": 1 "b": 2})}
action conta_d (d :: Dict::(Text, Int)) => Int
    len d
conta_d!({"a": 1 "b": 2})"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1, "deve ter 1 wrapper");

    let w = find_wrapper(&wrappers, "conta_d", 0);
    assert_eq!(w.action_name, "conta_d");
    assert_eq!(w.spec.desc.as_deref(), Some("dict como arg"));
    assert!(w.spec.args.is_some(), "args deve ser Some (Dict)");
    assert_ne!(
        w.func_id,
        FuncId::from_u32(0),
        "wrapper positivo deve ter func_id válido"
    );
}

// ── Teste 8: @test{args: ({|1 2 3|})} — Set como arg de diretiva ──

/// `@test{args: (...)}` com SetLit dentro da tupla de args. Verifica que
/// o parser de diretivas aceita SetLit, o typeck infere Set::Int,
/// e o codegen gera o wrapper sem erros.
#[test]
fn test_wrapper_com_set_como_arg() {
    let src = r#"@test{desc: "set como arg", args: ({|1 2 3|})}
action conta_s (s :: Set::Int) => Int
    len s
conta_s!({|1 2 3|})"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1, "deve ter 1 wrapper");

    let w = find_wrapper(&wrappers, "conta_s", 0);
    assert_eq!(w.action_name, "conta_s");
    assert_eq!(w.spec.desc.as_deref(), Some("set como arg"));
    assert!(w.spec.args.is_some(), "args deve ser Some (Set)");
    assert_ne!(
        w.func_id,
        FuncId::from_u32(0),
        "wrapper positivo deve ter func_id válido"
    );
}

// ── Teste 9: @test{args: {"b": 4 "a": 3}} — Dict nomeado como args de @test ──

/// `@test{args: {...}}` com DictLit onde chaves são nomes de params.
/// O typeck mapeia chaves→params e reordena para Tuple posicional.
#[test]
fn test_wrapper_com_dict_nomeado_args() {
    let src = r#"@test{desc: "args nomeados", args: {"b": 4 "a": 3}}
action soma (a::Int, b::Int) => Int
    + a b
soma!(1, 2)"#;
    let (_module, wrappers) = compile_tests(src);
    assert_eq!(wrappers.len(), 1, "deve ter 1 wrapper");

    let w = find_wrapper(&wrappers, "soma", 0);
    assert_eq!(w.action_name, "soma");
    assert_eq!(w.spec.desc.as_deref(), Some("args nomeados"));
    assert!(w.spec.args.is_some(), "args deve ser Some (Dict→Tuple)");
    assert_ne!(
        w.func_id,
        FuncId::from_u32(0),
        "wrapper positivo deve ter func_id válido"
    );
}
