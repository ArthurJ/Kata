//! Testes E2E — `type!()`: introspecção compile-time.
//!
//! PRD-introspection.md: `type!(expr)` retorna o tipo nominal de `expr`
//! como `Text`, resolvido em compile-time. O argumento é avaliado
//! (side-effects preservados), mas o valor é descartado.

use std::fs;
use std::process::Command;
use std::sync::Once;

static SETUP: Once = Once::new();

fn ensure_kata_rt_built() {
    SETUP.call_once(|| {
        let build_root = env!("KATA_BUILD_ROOT");
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let target_dir = std::path::Path::new(build_root)
            .join("target")
            .join(profile);
        let static_lib = target_dir.join("libkata_rt.a");
        let dynamic_lib = target_dir.join("libkata_rt.so");

        if static_lib.exists() && dynamic_lib.exists() {
            return;
        }

        let mut cmd = Command::new("cargo");
        cmd.current_dir(build_root)
            .arg("build")
            .arg("-p")
            .arg("kata-rt");

        if profile == "release" {
            cmd.arg("--release");
        }

        let status = cmd
            .status()
            .expect("setup: não foi possível executar `cargo build -p kata-rt`");

        assert!(
            status.success(),
            "setup: `cargo build -p kata-rt` falhou — \
             libkata_rt.a é necessária para AOT"
        );
    });
}

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-typeof");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata_build(input: &str, output: &str) -> (String, i32) {
    ensure_kata_rt_built();
    let output_path = std::env::temp_dir()
        .join("kata-driver-e2e-typeof")
        .join(output);
    let output_str = output_path.to_string_lossy().to_string();

    let result = Command::new(kata_bin())
        .args(["build", input, "-o", &output_str])
        .output()
        .expect("executar kata build");

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}{stderr}")
    };
    (combined, result.status.code().unwrap_or(-1))
}

fn run_built_binary(name: &str) -> (String, i32) {
    let bin_path = std::env::temp_dir()
        .join("kata-driver-e2e-typeof")
        .join(name);
    let result = Command::new(&bin_path)
        .output()
        .expect("executar binário AOT");
    let stdout = String::from_utf8_lossy(&result.stdout).to_string();
    (stdout, result.status.code().unwrap_or(-1))
}

/// Helper: build + run + retornar primeira linha do stdout.
/// O `main!()` retorna Unit que é impresso como "()" — a primeira
/// linha é o output do echo!, a última é "()".
fn build_and_get_first_line(name: &str, src: &str) -> String {
    let path = write_temp_kata(name, src);
    let (build_out, build_code) = run_kata_build(&path, &format!("{name}_bin"));
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary(&format!("{name}_bin"));
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");

    stdout.lines().next().unwrap_or("").to_string()
}

// ── DoD 1/5: type!(42) retorna "Int" ──────────────────────────────

#[test]
fn type_of_int_literal() {
    let first = build_and_get_first_line(
        "type_of_int_literal",
        "action main => Unit\n    echo!(type!(42))\nmain!()",
    );
    assert_eq!(first, "Int", "type!(42) deve imprimir \"Int\"");
}

// ── DoD 6: type!("hello") retorna "Text" ──────────────────────────

#[test]
fn type_of_text_literal() {
    let first = build_and_get_first_line(
        "type_of_text_literal",
        "action main => Unit\n    echo!(type!(\"hello\"))\nmain!()",
    );
    assert_eq!(first, "Text", "type!(\"hello\") deve imprimir \"Text\"");
}

// ── DoD 7: type!(()) retorna "Unit" ───────────────────────────────

#[test]
fn type_of_unit() {
    let first = build_and_get_first_line(
        "type_of_unit",
        "action main => Unit\n    echo!(type!(()))\nmain!()",
    );
    assert_eq!(first, "Unit", "type!(()) deve imprimir \"Unit\"");
}

// ── DoD 8: type!(Boolean::True) retorna "Boolean" ─────────────────

#[test]
fn type_of_boolean() {
    let first = build_and_get_first_line(
        "type_of_boolean",
        "action main => Unit\n    echo!(type!(Boolean::True))\nmain!()",
    );
    assert_eq!(
        first, "Boolean",
        "type!(Boolean::True) deve imprimir \"Boolean\""
    );
}

// ── DoD 9: type!(Optional::Some 42) retorna "Optional::Int" ────────

#[test]
fn type_of_generic_one_param() {
    let first = build_and_get_first_line(
        "type_of_generic_one",
        "action main => Unit\n    echo!(type!(Optional::Some 42))\nmain!()",
    );
    assert_eq!(
        first, "Optional::Int",
        "type!(Optional::Some 42) deve imprimir \"Optional::Int\""
    );
}

// ── DoD 10: type!(Result::Ok 42) retorna "Result::(Int, Text)" ────

/// `type!(r)` onde `r := Result::Ok 42` → "Result::(Int, Text)".
/// O default `Err(E=Text)` do prelude preenche E=Text automaticamente.
#[test]
fn type_of_generic_two_params() {
    let src = "action main => Unit\n    let r := Result::Ok 42\n    echo!(type!(r))\nmain!()";
    let first = build_and_get_first_line("type_of_generic_two", src);
    assert_eq!(
        first, "Result::(Int, Text)",
        "type!(r) deve imprimir \"Result::(Int, Text)\" — E=Text via default"
    );
}

// ── DoD 11: type!([1 2 3]) retorna "[Int]" ─────────────────────────

#[test]
fn type_of_list() {
    let first = build_and_get_first_line(
        "type_of_list",
        "action main => Unit\n    echo!(type!([1 2 3]))\nmain!()",
    );
    assert_eq!(first, "[Int]", "type!([1 2 3]) deve imprimir \"[Int]\"");
}

// ── DoD 12: type!(soma) retorna "(Int Int -> Int)" ─────────────────

/// `type!(fat)` na entry expression (não dentro de Action) →
/// "(Int Int -> Int)". Funções puras são acessíveis na entry expression.
#[test]
fn type_of_function() {
    let src = "fat :: Int Int => Int\nlambda 0 acc: acc\nlambda n acc: fat (- n 1) (* n acc)\n\necho!(type!(fat))";
    let path = write_temp_kata("type_of_function", src);
    let (build_out, build_code) = run_kata_build(&path, "type_of_function_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("type_of_function_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    let first = stdout.lines().next().unwrap_or("").to_string();
    assert_eq!(
        first, "(Int Int -> Int)",
        "type!(fat) deve imprimir \"(Int Int -> Int)\""
    );
}

// ── DoD 13: type!(worker) retorna "Action(Int) => Unit" ────────────

#[test]
fn type_of_action_ref() {
    let src = "\
action worker (n :: Int) => Unit
    echo!(n)

action main => Unit
    echo!(type!(worker))
main!()";
    let first = build_and_get_first_line("type_of_action_ref", src);
    assert_eq!(
        first, "Action(Int) => Unit",
        "type!(worker) deve imprimir \"Action(Int) => Unit\""
    );
}

// ── DoD 14: type!(f!()) executa f!() — side-effects acontecem ─────

/// `type!(echo!("side effect"))` imprime "side effect" e depois "Unit".
#[test]
fn type_of_side_effect() {
    let src = "action main => Unit\n    echo!(type!(echo!(\"side effect\")))\nmain!()";
    let path = write_temp_kata("type_of_side_effect", src);
    let (build_out, build_code) = run_kata_build(&path, "type_of_side_effect_bin");
    assert_eq!(
        build_code, 0,
        "kata build deve ter exit 0 — output: {build_out}"
    );

    let (stdout, code) = run_built_binary("type_of_side_effect_bin");
    assert_eq!(code, 0, "binário AOT deve exit 0 — stdout: {stdout}");
    // echo!("side effect") imprime "side effect",
    // echo!(type!(...)) imprime "Unit",
    // main!() retorna Unit → imprime "()"
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "deve imprimir 3 linhas — stdout: {stdout}");
    assert_eq!(
        lines[0], "side effect",
        "primeira linha deve ser o side-effect"
    );
    assert_eq!(lines[1], "Unit", "segunda linha deve ser \"Unit\"");
}

// ── DoD extra: tipo criado pelo usuário (data) ────────────────────

/// `type!(p)` onde `p := Pessoa "Alice" 30` → "Pessoa".
/// Testa que `type!()` retorna o nome nominal de um `data` definido
/// pelo usuário, não o tipo base ou layout interno.
#[test]
fn type_of_user_data() {
    let src = "data Pessoa (nome::Text idade::Int)\n\naction main => Unit\n    let p := Pessoa \"Alice\" 30\n    echo!(type!(p))\nmain!()";
    let first = build_and_get_first_line("type_of_user_data", src);
    assert_eq!(first, "Pessoa", "type!(p) deve imprimir \"Pessoa\"");
}

// ── DoD extra: tipo criado pelo usuário (enum) ────────────────────

/// `type!(c)` onde `c := Cor::Verde` → "Cor".
/// Testa que `type!()` retorna o nome nominal de um `enum` definido
/// pelo usuário.
#[test]
fn type_of_user_enum() {
    let src = "enum Cor\n    Vermelho\n    Verde\n    Azul\n\naction main => Unit\n    let c := Cor::Verde\n    echo!(type!(c))\nmain!()";
    let first = build_and_get_first_line("type_of_user_enum", src);
    assert_eq!(first, "Cor", "type!(c) deve imprimir \"Cor\"");
}
