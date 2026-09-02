//! Testes E2E: Limite de Profundidade de Recursão.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Verifica DoD do PRD-recursion-limit.

use kata_codegen::{jit_eval, leak_rt_ptr};
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, resolve, ResolvedModule};
use kata_tree_shaking::tree_shake;

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
            let mut f = prelude.functions;
            f.extend(user.functions);
            f
        },
        actions: {
            let mut a = prelude.actions;
            a.extend(user.actions);
            a
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

fn eval_src(src: &str) -> Result<(i64, Ty), String> {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));
    match jit_eval(&typed, &Default::default(), &[], leak_rt_ptr(), false) {
        Ok(jit) => Ok((jit.raw, jit.ty)),
        Err(e) => Err(format!("{e}")),
    }
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// Recursão não-de-cauda via JIT com profundidade > limite
/// → `CodegenError::Runtime`, não SIGSEGV.
/// Usa `count` — múltiplas cláusulas, só a base faz adição.
/// Não-TCO: a chamada recursiva não está na posição de cauda.
/// Não-TRMA: `+` com 1 (constante) não é acumulador pattern.
#[test]
fn recursion_limit_codegen() {
    let src = r#"
count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

count 1200
"#;
    let result = eval_src(src);
    assert!(result.is_err(), "count 1200 com limite 1000 deve falhar");
    let err = result.unwrap_err();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );
}

/// DoD 3: TCO preservado — `fat_tail 100000 1` (tail-recursiva) via JIT
/// executa sem atingir o limite.
#[test]
fn tco_not_limited_codegen() {
    let src = r#"
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail (- n 1) (* n acc)

fat_tail 100000 1
"#;
    let (raw, _) = eval_src(src).expect("TCO deve executar sem erro");
    // 100000! é um número enorme — só verificar que não crasha.
    let _ = raw;
}

/// DoD 3 (TRMA): `soma_acc 1000000` (TRMA rewrite) via JIT executa com sucesso.
#[test]
fn trma_not_limited_codegen() {
    let src = r#"
soma_acc :: Int Int => Int
lambda 0 acc: acc
lambda n acc: soma_acc (- n 1) (+ n acc)

soma_acc 1000000 0
"#;
    let (raw, _) = eval_src(src).expect("TRMA deve executar sem erro");
    let _ = raw;
}

/// DoD 4: Recursão mútua não-de-cauda com profundidade > limite falha graciosamente.
/// `ping`/`pong` — chamada não-tail (resultado envolvido em adição).
#[test]
fn recursion_limit_mutual_codegen() {
    let src = r#"
ping :: Int => Int
lambda 0: 0
lambda n: + (pong (- n 1)) 1

pong :: Int => Int
lambda 0: 0
lambda n: + (ping (- n 1)) 1

ping 2000
"#;
    let result = eval_src(src);
    assert!(result.is_err(), "recursão mútua não-tail 2000 deve falhar");
    let err = result.unwrap_err();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );
}

/// DoD 6: Reset entre execuções — após execução bem-sucedida,
/// depth volta a 0. Verificamos que `fat 10` executa corretamente.
#[test]
fn depth_resets_codegen() {
    let src = r#"
fat :: Int => Int
lambda 0: 1
lambda n: * n (fat (- n 1))

fat 10
"#;
    let (raw, _) = eval_src(src).expect("fat 10 deve succeed");
    assert_eq!(untag_smi(raw), 3628800); // 10! = 3628800
}

/// DoD 2: Recursão não-de-cauda com profundidade dentro do limite
/// funciona normalmente. `fat 100` = não-TCO, dentro do limite 1000.
#[test]
fn recursion_within_limit_codegen() {
    let src = r#"
fat :: Int => Int
lambda 0: 1
lambda n: * n (fat (- n 1))

fat 100
"#;
    let (raw, _) = eval_src(src).expect("fat 100 deve succeed");
    // 100! é BigInt — só verificar que não crasha e retorna algum valor.
    let _ = raw;
}

/// call_indirect (não-de-cauda) também é contado.
/// `apply` aplica uma função — call_indirect no JIT.
/// `count` é não-TCO/não-TRMA, então atinge o limite.
#[test]
fn recursion_limit_indirect_codegen() {
    let src = r#"
count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

apply :: (Int -> Int) Int => Int
lambda f x: f x

apply count 1200
"#;
    let result = eval_src(src);
    assert!(result.is_err(), "apply count 1200 deve falhar por depth");
    let err = result.unwrap_err();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );
}

/// `set_recursion_limit` em `constant` propaga o limite do comptime
/// Runtime para o Runtime da execução principal. Usa o pipeline completo
/// com comptime pass: cria comptime Runtime, executa `run_comptime_pass`,
/// lê `depth_limit`, cria Runtime principal com o limite propagado.
///
/// `count 1500` falha com limite default (1000) mas passa com limite 2000.
/// Usa `@ffi("kata_rt_depth_set_limit")` diretamente — sem `import config`
/// porque testes E2E em kata-codegen não carregam imports do filesystem.
#[test]
fn recursion_limit_configurable_codegen() {
    let src = r#"
@ffi("kata_rt_depth_set_limit")
set_recursion_limit :: Int => Unit

count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

constant _ := set_recursion_limit(2000)

count 1500
"#;
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = monomorphize(typed);
    let typed = optimize(typed);
    let typed = kata_monomorph::MonoModule::from(tree_shake(typed.inner));

    // Criar comptime Runtime e rodar comptime pass.
    let comptime_rt = Box::new(kata_rt::Runtime::new());
    let comptime_rt_ptr = Box::into_raw(comptime_rt) as i64;
    let typed =
        run_comptime_pass(typed.inner, &resolved.enum_registry, comptime_rt_ptr)
            .expect("comptime deve succeed");

    // Ler depth_limit do comptime Runtime.
    let depth_limit = unsafe { (*(comptime_rt_ptr as *mut kata_rt::Runtime)).depth_limit() };
    unsafe { drop(Box::from_raw(comptime_rt_ptr as *mut kata_rt::Runtime)) };

    // Deve ser 2000 (setado por set_recursion_limit em constant).
    assert_eq!(
        depth_limit, 2000,
        "depth_limit deve ser 2000 após set_recursion_limit"
    );

    // Criar Runtime principal com depth_limit propagado.
    let typed = kata_monomorph::MonoModule::from(typed);
    let rt = Box::new(kata_rt::Runtime::new());
    let rt_ptr = Box::into_raw(rt) as i64;
    unsafe { (*(rt_ptr as *mut kata_rt::Runtime)).depth_set_limit(depth_limit) };

    let result = jit_eval(&typed, &Default::default(), &[], rt_ptr, false);
    assert!(
        result.is_ok(),
        "count 1500 com limite 2000 deve passar: {:?}",
        result.err()
    );
}