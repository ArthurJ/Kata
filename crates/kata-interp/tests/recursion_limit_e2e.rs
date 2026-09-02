//! Testes E2E: Limite de Profundidade de Recursão (interpretador).
//!
//! Pipeline: lex → parse → resolve → infer → monomorph → optimize → interpret.
//! Verifica DoD do PRD-recursion-limit para o backend interpretador.

use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_interp::{InterpError, interpret_with_registry};
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};
use kata_rt::Runtime;

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

fn interp_src(src: &str) -> Result<(i64, Ty), InterpError> {
    // O interpretador tree-walking tem frames Rust grandes em debug mode.
    // Rodar em thread com stack expandida para evitar stack overflow do
    // próprio Rust antes do depth tracker do Kata pegar.
    let src = src.to_string();
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // 32MB
        .spawn(move || {
            let tokens = lex(&src).expect("lex deve succeed");
            let module = parse(tokens).expect("parse deve succeed");
            let prelude = load_stdlib_for_tests().expect("prelude deve carregar");
            let user = resolve(&module).expect("resolve deve succeed");
            let resolved = merge_resolved(prelude, user);
            let typed = infer_module(&module, &resolved).expect("infer deve succeed");
            let enum_registry = resolved.enum_registry.clone();
            let typed = monomorphize(typed);
            let typed = optimize(typed);
            let typed = typed.inner;
            let rt = Box::new(Runtime::new());
            let rt_ptr = Box::into_raw(rt) as i64;
            let result = interpret_with_registry(typed, rt_ptr, enum_registry);
            // Leak do Runtime — valores retornados podem ser ponteiros para a arena.
            std::mem::forget(unsafe { Box::from_raw(rt_ptr as *mut Runtime) });
            result.map(|r| (r.raw, r.ty))
        })
        .unwrap();
    handle.join().unwrap()
}

fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

/// DoD 1: Recursão não-de-cauda com profundidade > limite
/// → `InterpError::Runtime`, não SIGSEGV.
/// Usa `count` — não-TCO (chamada recursiva não está em posição de cauda).
/// Configura limite 100 para evitar stack overflow do próprio Rust
/// (interpretador tree-walking: cada frame Rust é ~2KB, limite 1000
/// pode estourar a stack de teste de 2MB antes do depth tracker pegar).
#[test]
fn recursion_limit_interp() {
    let src = r#"
@ffi("kata_rt_depth_set_limit")
set_recursion_limit :: Int => Unit

constant _ := set_recursion_limit(100)

count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

count 200
"#;
    let result = interp_src(src);
    assert!(result.is_err(), "count 200 com limite 100 deve falhar");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );
}

/// DoD 4: Recursão mútua is_even/is_odd com profundidade > limite falha graciosamente.
/// Usa recursão não-de-cauda (match envolve a chamada) para garantir que
/// o trampoline não faça TCO.
#[test]
fn recursion_limit_mutual() {
    let src = r#"
@ffi("kata_rt_depth_set_limit")
set_recursion_limit :: Int => Unit

constant _ := set_recursion_limit(100)

is_even :: Int => Boolean
lambda 0: True
lambda n:
    match (is_odd (- n 1))
        Boolean::True: True
        Boolean::False: False

is_odd :: Int => Boolean
lambda 0: False
lambda n:
    match (is_even (- n 1))
        Boolean::True: True
        Boolean::False: False

is_even 200
"#;
    let result = interp_src(src);
    assert!(result.is_err(), "recursão mútua 200 deve falhar");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );
}

/// DoD 3: TCO preservado — `fat_tail 100000 1` (tail-recursiva) executa
/// com sucesso (não atinge limite). O trampoline não incrementa depth.
#[test]
fn tco_not_limited() {
    let src = r#"
fat_tail :: Int Int => Int
lambda 0 acc: acc
lambda n acc: fat_tail (- n 1) (* n acc)

fat_tail 100000 1
"#;
    let (raw, _) = interp_src(src).expect("TCO deve executar sem erro");
    // 100000! é BigInt — só verificar que não crasha.
    let _ = raw;
}

/// DoD 6: Reset entre execuções — após execução bem-sucedida,
/// depth volta a 0. Verificamos que uma execução curta funciona
/// (o reset acontece no início de eval_entry_with_env).
#[test]
fn depth_resets() {
    let src = r#"
fat :: Int => Int
lambda 0: 1
lambda n: * n (fat (- n 1))

fat 10
"#;
    let (raw, _) = interp_src(src).expect("fat 10 deve succeed");
    assert_eq!(untag_smi(raw), 3628800); // 10! = 3628800
}

/// DoD 5: Configuração via constant — `set_recursion_limit(10)` +
/// recursão 20 falha; `set_recursion_limit(100)` + recursão 50 passa.
#[test]
fn configurable_limit() {
    // Com limite 10, recursão 20 deve falhar.
    let src_fail = r#"
@ffi("kata_rt_depth_set_limit")
set_recursion_limit :: Int => Unit

constant _ := set_recursion_limit(10)

count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

count 20
"#;
    let result = interp_src(src_fail);
    assert!(
        result.is_err(),
        "count 20 com limite 10 deve falhar: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("recursion depth exceeded"),
        "erro deve mencionar recursion depth: {err}"
    );

    // Com limite 100, recursão 50 deve passar.
    let src_pass = r#"
@ffi("kata_rt_depth_set_limit")
set_recursion_limit :: Int => Unit

constant _ := set_recursion_limit(100)

count :: Int => Int
lambda 0: 0
lambda 1: 1
lambda n: + (count (- n 1)) 1

count 50
"#;
    let result = interp_src(src_pass).expect("count 50 com limite 100 deve passar");
    // count 50: cada chamada retorna + (count (n-1)) 1.
    // count 0 = 0, count 1 = 1, count 2 = + 1 1 = 2, ... count n = n.
    assert_eq!(untag_smi(result.0), 50);
}
