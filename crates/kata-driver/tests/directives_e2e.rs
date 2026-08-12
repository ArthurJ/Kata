//! Testes E2E — Diretivas customizadas: inlining de Enter, Exit, ShortCircuit, Transform.
//!
//! Valida que o desugaring de diretivas customizadas funciona end-to-end:
//! `directive nome{when: ..., on: ...}` + `@nome` aplicada em action/function
//! produz código que compila e executa corretamente.

use std::fs;
use std::process::Command;

/// Localiza o binário `kata` compilado (target/debug/kata).
fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Cria um arquivo `.kata` temporário e retorna o path.
fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-directives-e2e");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

/// Executa `kata run <path>` e retorna (stdout, stderr, exit_code).
fn run_kata(path: &str) -> (String, String, i32) {
    let output = Command::new(kata_bin())
        .args(["run", path])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── Test 1: Enter em action — imprime nome ao entrar ────────────────

#[test]
fn e2e_enter_action_prints_name() {
    let src = r#"directive trace_enter{when: Hook::Enter, on: Target::Action}
    echo!(_name)

@trace_enter
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_enter_action", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("greet"),
        "deve imprimir 'greet' (nome da action) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("hello"),
        "deve imprimir 'hello' (body original) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 2: Enter com _args — acessa argumentos via tupla ────────────

#[test]
fn e2e_enter_action_args() {
    let src = r#"directive trace_args{when: Hook::Enter, on: Target::Action}
    echo!(_args.0)

@trace_args
action add(a :: Int, b :: Int) => Int
    + a b

echo!(add!(3, 4))"#;
    let path = write_temp_kata("e2e_enter_args", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // _args.0 = 3 (primeiro arg), depois o resultado 7
    assert!(
        stdout.contains("3"),
        "deve imprimir 3 (primeiro arg via _args.0) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("7"),
        "deve imprimir 7 (resultado de add) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 3: Exit em action — observa resultado ───────────────────────

#[test]
fn e2e_exit_action_observes_result() {
    let src = r#"directive trace_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

@trace_exit
action double(x :: Int) => Int
    * x 2

echo!(double!(21))"#;
    let path = write_temp_kata("e2e_exit_action", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Exit observa _return = 42, depois o caller imprime 42
    // stdout deve conter 42 duas vezes (uma do echo! da diretiva, uma do echo! do caller)
    assert!(
        stdout.contains("42"),
        "deve imprimir 42 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 4: ShortCircuit — prossegue quando Optional::None ──────────

#[test]
fn e2e_shortcircuit_proceeds() {
    let src = r#"directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Optional::None

@gate
action process(x :: Int) => Int
    + x 1

echo!(process!(10))"#;
    let path = write_temp_kata("e2e_shortcircuit_proceeds", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // ShortCircuit retorna None → body executa → 10 + 1 = 11
    assert!(
        stdout.contains("11"),
        "deve imprimir 11 (body executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 5: ShortCircuit — short-circuita com Optional::Some ────────

#[test]
fn e2e_shortcircuit_blocks() {
    let src = r#"directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Optional::Some(999)

@gate
action process(x :: Int) => Int
    + x 1

echo!(process!(10))"#;
    let path = write_temp_kata("e2e_shortcircuit_blocks", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // ShortCircuit retorna Some(999) → body não executa → resultado = 999
    assert!(
        stdout.contains("999"),
        "deve imprimir 999 (short-circuit) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        !stdout.contains("11"),
        "não deve imprimir 11 (body não executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 6: Transform — modifica o resultado ────────────────────────

#[test]
fn e2e_transform_modifies_result() {
    let src = r#"directive redact{when: Hook::Transform, on: Target::Action}
    + _return 1

@redact
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_transform", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Transform: _return = 10 + 1 = 11, depois Transform soma 1 → 12
    assert!(
        stdout.contains("12"),
        "deve imprimir 12 (transform: 11 + 1) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 7: Stacking — Enter + Exit na mesma action ─────────────────

#[test]
fn e2e_stacking_enter_exit() {
    let src = r#"directive trace_enter{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive trace_exit{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

@trace_enter
@trace_exit
action compute(x :: Int) => Int
    + x 2

echo!(compute!(5))"#;
    let path = write_temp_kata("e2e_stacking", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Enter imprime "ENTER", body executa (5+2=7), Exit imprime "EXIT", caller imprime 7
    assert!(
        stdout.contains("ENTER"),
        "deve imprimir ENTER — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("EXIT"),
        "deve imprimir EXIT — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("7"),
        "deve imprimir 7 (resultado) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 8: Reflection vars — _name, _arity, _is_action ─────────────

#[test]
fn e2e_reflection_vars() {
    let src = r#"directive trace_meta{when: Hook::Enter, on: Target::Action}
    echo!(_name)
    echo!(_arity)
    echo!(_is_action)

@trace_meta
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_reflection_vars", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("greet"),
        "deve imprimir 'greet' (_name) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("1"),
        "deve imprimir 1 (_arity) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("True"),
        "deve imprimir True (_is_action) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 9: Enter em função pura ─────────────────────────────────────

#[test]
fn e2e_enter_function() {
    let src = r#"directive trace_fn{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace_fn
double :: Int => Int
lambda x: * x 2

echo!(double 10)"#;
    let path = write_temp_kata("e2e_enter_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // A diretiva inlina `let _ := _name` (binding de reflexão) antes do body.
    // A função executa corretamente: double(10) = 20.
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 (resultado) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 10: Target mismatch — on: Action aplicada em função → erro ─

#[test]
fn e2e_target_mismatch_action_on_function() {
    let src = r#"directive trace_act{when: Hook::Enter, on: Target::Action}
    echo!(_name)

@trace_act
double :: Int => Int
lambda x: * x 2

echo!(double 10)"#;
    let path = write_temp_kata("e2e_target_mismatch_act_on_fn", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (Target::Action em função) — stderr: {stderr}"
    );
    assert!(
        stderr.contains("não pode decorar")
            || stderr.contains("DirectiveTargetMismatch")
            || stderr.contains("target"),
        "deve reportar erro de target mismatch — stderr: {stderr}"
    );
}

// ── Test 12: Stacking 3 diretivas — Enter + ShortCircuit + Exit ─────

#[test]
fn e2e_stacking_three_directives() {
    let src = r#"directive log_enter{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Optional::None

@log_enter
@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_stacking_three", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    let enter_pos = stdout.find("ENTER");
    let result_pos = stdout.find("11");
    let exit_pos = stdout.find("EXIT");
    assert!(
        enter_pos.is_some(),
        "deve imprimir ENTER — stdout: {stdout}"
    );
    assert!(exit_pos.is_some(), "deve imprimir EXIT — stdout: {stdout}");
    assert!(result_pos.is_some(), "deve imprimir 11 — stdout: {stdout}");
    assert!(
        enter_pos < exit_pos,
        "ENTER deve vir antes de EXIT — stdout: {stdout}"
    );
}

// ── Test 13: ShortCircuit interna + Exit externa — short-circuit propaga ─

#[test]
fn e2e_shortcircuit_inner_exit_outer() {
    let src = r#"directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Optional::Some(999)

@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_sc_inner_exit_outer", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("999"),
        "deve imprimir 999 (short-circuit + exit observa) — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        !stdout.contains("11"),
        "não deve imprimir 11 (body não executou) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 14: ShortCircuit interna + Exit externa — prossegue ─────────

#[test]
fn e2e_shortcircuit_inner_exit_outer_proceeds() {
    let src = r#"directive log_exit{when: Hook::Exit, on: Target::Any}
    echo!(_return)

directive gate{when: Hook::ShortCircuit, on: Target::Action}
    Optional::None

@log_exit
@gate
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_sc_inner_exit_outer_proceeds", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    assert!(
        stdout.contains("11"),
        "deve imprimir 11 (body executou, exit observa) — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 15: Stacking ordem Enter — primeira = mais externa ──────────

#[test]
fn e2e_stacking_enter_order() {
    let src = r#"directive outer{when: Hook::Enter, on: Target::Any}
    echo!("OUTER")

directive inner{when: Hook::Enter, on: Target::Any}
    echo!("INNER")

@outer
@inner
action compute(x :: Int) => Int
    + x 1

echo!(compute!(10))"#;
    let path = write_temp_kata("e2e_stacking_enter_order", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    let outer_pos = stdout.find("OUTER");
    let inner_pos = stdout.find("INNER");
    assert!(
        outer_pos.is_some(),
        "deve imprimir OUTER — stdout: {stdout}"
    );
    assert!(
        inner_pos.is_some(),
        "deve imprimir INNER — stdout: {stdout}"
    );
    assert!(
        outer_pos < inner_pos,
        "OUTER deve vir antes de INNER (primeira = mais externa) — stdout: {stdout}"
    );
}

#[test]
fn e2e_target_mismatch_function_on_action() {
    let src = r#"directive trace_fn{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace_fn
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#;
    let path = write_temp_kata("e2e_target_mismatch_fn_on_act", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (Target::Function em action) — stderr: {stderr}"
    );
    assert!(
        stderr.contains("não pode decorar")
            || stderr.contains("DirectiveTargetMismatch")
            || stderr.contains("target"),
        "deve reportar erro de target mismatch — stderr: {stderr}"
    );
}

// ── Test 16: Overloading — mesmo nome, (Enter, Action) e (Enter, Function) ─

#[test]
fn e2e_overloading_enter_action_and_function() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Action}
    echo!("action-enter")

directive trace{when: Hook::Enter, on: Target::Function}
    let _ := _name

@trace
action act(x :: Int) => Int
    + x 1

@trace
double :: Int => Int
lambda x: * x 2

echo!(act!(5))
echo!(double 10)"#;
    let path = write_temp_kata("e2e_overloading_enter", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Action dispara Enter::Action (imprime "action-enter"), Function dispara Enter::Function (let _ := _name, puro)
    assert!(
        stdout.contains("action-enter"),
        "deve imprimir action-enter — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("6"),
        "deve imprimir 6 (act 5+1) — stdout: {stdout}"
    );
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 (double 10*2) — stdout: {stdout}"
    );
}

// ── Test 17: Overloading — Target::Any casa com action e função ──────

#[test]
fn e2e_overloading_any_matches_both() {
    let src = r#"directive trace{when: Hook::Exit, on: Target::Any}
    let _ := _return

@trace
action act(x :: Int) => Int
    + x 1

@trace
double :: Int => Int
lambda x: * x 2

echo!(act!(5))
echo!(double 10)"#;
    let path = write_temp_kata("e2e_overloading_any", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 — stderr: {stderr}");
    // Exit::Any dispara em ambos. O body da diretiva é `let _ := _return` (puro).
    // Resultados: act(5) = 6, double(10) = 20
    assert!(
        stdout.contains("6"),
        "deve imprimir 6 — stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("20"),
        "deve imprimir 20 — stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 18: Overloading — (nome, when, on) duplicado → erro ──────────

#[test]
fn e2e_overloading_duplicate_error() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Action}
    echo!("first")

directive trace{when: Hook::Enter, on: Target::Action}
    echo!("second")

@trace
action act(x :: Int) => Int
    + x 1

echo!(act!(5))"#;
    let path = write_temp_kata("e2e_overloading_dup_error", src);
    let (_stdout, stderr, code) = run_kata(&path);
    assert_ne!(
        code, 0,
        "deve falhar (diretiva duplicada) - stderr: {stderr}"
    );
    assert!(
        stderr.contains("duplicada") || stderr.contains("DuplicateDirective"),
        "deve reportar erro de diretiva duplicada - stderr: {stderr}"
    );
}

// ── Test 19: Overloading — mesmo nome com hooks diferentes coexiste ─

#[test]
fn e2e_overloading_different_hooks_coexist() {
    let src = r#"directive trace{when: Hook::Enter, on: Target::Any}
    echo!("ENTER")

directive trace{when: Hook::Exit, on: Target::Any}
    echo!("EXIT")

@trace
action act(x :: Int) => Int
    + x 1

echo!(act!(5))"#;
    let path = write_temp_kata("e2e_overloading_diff_hooks", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("ENTER"),
        "deve imprimir ENTER - stdout: {stdout}"
    );
    assert!(
        stdout.contains("EXIT"),
        "deve imprimir EXIT - stdout: {stdout}"
    );
    let enter_pos = stdout.find("ENTER");
    let exit_pos = stdout.find("EXIT");
    assert!(
        enter_pos < exit_pos,
        "ENTER deve vir antes de EXIT - stdout: {stdout}"
    );
}

// ── Test 20: Importação — diretiva exportada de outro módulo ─────────
//
// Cria dois arquivos: tracing_mod.kata (exportador) e main_import.kata
// (importador). O importador usa `import tracing_mod.trace_enter` e aplica
// `@trace_enter` numa action. O driver resolve o import, carrega o módulo
// exportador, mescla o DirectiveRegistry, e o desugaring inlinea a diretiva.

#[test]
fn e2e_import_directive() {
    let dir = std::env::temp_dir().join("kata-driver-directives-e2e-import");
    fs::create_dir_all(&dir).expect("criar temp dir");

    // Módulo exportador: define e exporta a diretiva
    let mod_path = dir.join("tracing_mod.kata");
    fs::write(
        &mod_path,
        r#"directive trace_enter{when: Hook::Enter, on: Target::Action}
    echo!("imported-enter")

export trace_enter"#,
    )
    .expect("escrever modulo exportador");

    // Módulo importador: importa e aplica a diretiva
    let main_path = dir.join("main_import.kata");
    fs::write(
        &main_path,
        r#"import tracing_mod.(trace_enter)

@trace_enter
action greet(name :: Text) => Unit
    echo!("hello")

greet!("world")"#,
    )
    .expect("escrever modulo importador");

    let output = Command::new(kata_bin())
        .args(["run", main_path.to_str().unwrap()])
        .output()
        .expect("executar kata run");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("imported-enter"),
        "deve imprimir imported-enter (diretiva importada) - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("hello"),
        "deve imprimir hello (body original) - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Fase 1: Args no site de aplicação + _args em funções ────────────

// ── Test 21: @trace{msg: Text, when: "enter"} em função pura com _args ─

#[test]
fn e2e_trace_args_function_pure() {
    // Função pura com diretiva Enter: log!() é FFI direta (não bloqueia).
    // Não podemos verificar o log (precisa de consumidor log_recv!()),
    // mas verificamos que a função compila e executa corretamente.
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Function, msg: Text}
    log!(LogLevel::Info, format _msg (_name,))

@trace_test{msg: "entering {}", when: "enter"}
dobra :: Int => Int
lambda n: * n 2

echo!(dobra 21)"#;
    let path = write_temp_kata("e2e_trace_args_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 22: @trace{msg: Text, when: "enter"} em action com _args ────

#[test]
fn e2e_trace_args_action() {
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text}
    echo!(format _msg (_name,))

@trace_test{msg: "action {}", when: "enter"}
action processar(x :: Int) => Int
    + x 1

echo!(processar!(5))"#;
    let path = write_temp_kata("e2e_trace_args_act", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("action processar"),
        "deve imprimir msg formatada com _name - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("6"),
        "deve imprimir resultado 6 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 23: Despacho por arg_keys — msg vs msg+topic ───────────────

#[test]
fn e2e_trace_dispatch_by_arg_keys() {
    let src = r#"directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text}
    echo!(format _msg (_name,))

directive trace_test{when: Hook::Enter, on: Target::Action, msg: Text, topic: Text}
    echo!(format _msg (_name,))

@trace_test{msg: "simple {}", when: "enter"}
action sem_topic(x :: Int) => Int
    + x 1

@trace_test{msg: "with topic {}", when: "enter", topic: "audit"}
action com_topic(x :: Int) => Int
    + x 2

echo!(sem_topic!(10))
echo!(com_topic!(20))"#;
    let path = write_temp_kata("e2e_trace_dispatch", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("simple sem_topic"),
        "deve despachar para overload sem topic - stdout: {stdout} | stderr: {stderr}"
    );
    assert!(
        stdout.contains("with topic com_topic"),
        "deve despachar para overload com topic - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 24: Exit hook com args do site ─────────────────────────────

#[test]
fn e2e_trace_exit_args_function() {
    // Exit em função pura: log!() com _return. Verifica que compila e executa.
    let src = r#"directive trace_test{when: Hook::Exit, on: Target::Function, msg: Text}
    log!(LogLevel::Info, format _msg (_name, _return))

@trace_test{msg: "exit {} -> {}", when: "exit"}
inc :: Int => Int
lambda n: + n 1

echo!(inc 41)"#;
    let path = write_temp_kata("e2e_trace_exit_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 25: @trace do stdlib sem declaration local (Fase 2 DoD) ────

#[test]
fn e2e_trace_stdlib_function() {
    // @trace do stdlib (core.kata) sem declaration local.
    // log!() vai para CSP (não stdout) — verificamos que compila e executa.
    let src = r#"@trace{msg: "entering {}", when: "enter"}
dobra :: Int => Int
lambda n: * n 2

echo!(dobra 21)"#;
    let path = write_temp_kata("e2e_trace_stdlib_fn", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 26: @trace do stdlib com exit hook (Fase 2 DoD) ────────────

#[test]
fn e2e_trace_stdlib_exit() {
    let src = r#"@trace{msg: "exit {} -> {}", when: "exit"}
inc :: Int => Int
lambda n: + n 1

echo!(inc 41)"#;
    let path = write_temp_kata("e2e_trace_stdlib_exit", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("42"),
        "deve imprimir resultado 42 - stdout: {stdout} | stderr: {stderr}"
    );
}

// ── Test 27: @trace do stdlib com topic+policy em action (Fase 2 DoD) ─

#[test]
fn e2e_trace_stdlib_action_topic_policy() {
    // Action com topic+policy: log!() publica em CSP com policy "drop".
    let src = r#"@trace{msg: "action {}", when: "enter", topic: "audit", policy: "drop"}
action processar(x :: Int) => Int
    + x 1

echo!(processar!(5))"#;
    let path = write_temp_kata("e2e_trace_stdlib_act", src);
    let (stdout, stderr, code) = run_kata(&path);
    assert_eq!(code, 0, "exit 0 - stderr: {stderr}");
    assert!(
        stdout.contains("6"),
        "deve imprimir resultado 6 - stdout: {stdout} | stderr: {stderr}"
    );
}
