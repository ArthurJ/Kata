//! Testes E2E — Fase 3: acesso de functions e actions a constants.
//!
//! PRD-constant.md Fase 3: `Ident` dentro de action/função/lambda resolve
//! binding de módulo (constant). O comptime pass substitui `Ident(name)`
//! pelo literal/snapshot nos corpos de functions e actions após o fixpoint.
//!
//! DoD: `constant scale := 2` + `dobro :: Int => Int` + `lambda x: * x scale`
//! + `echo!(dobro 21)` imprime `42`.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn run_kata_run(source: &str) -> (String, String, i32) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_constant_fase3_e2e_{id}_{pid}.kata",
        pid = std::process::id()
    ));
    std::fs::write(&path, source).expect("escrever arquivo temporário");
    let output = Command::new(kata_bin())
        .args(["run", &path.to_string_lossy()])
        .output()
        .expect("executar kata run");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ── DoD Fase 3: constant + function acesso ─────────────────────────

/// `constant scale := 2` + `dobro :: Int => Int` + `lambda x: * x scale`
/// deve imprimir `42` quando chamado com `dobro 21`.
#[test]
fn constant_acessada_por_function() {
    let source =
        "constant scale := 2\ndobro :: Int => Int\nlambda x: * x scale\n\necho!(dobro 21)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "42",
        "dobro 21 com scale=2 deve imprimir 42 — stdout: {stdout}"
    );
}

/// Constant Int usada em função com múltiplas clauses.
#[test]
fn constant_em_function_multi_clauses() {
    let source = "constant base := 10\nsoma :: Int Int => Int\nlambda 0 y: + base y\nlambda x y: + x y\n\necho!(soma 0 5)\necho!(soma 3 5)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines[0], "15",
        "soma 0 5 = base + 5 = 15 — stdout: {stdout}"
    );
    assert_eq!(lines[1], "8", "soma 3 5 = 3 + 5 = 8 — stdout: {stdout}");
}

/// Constant Text usada em function (string_concat para Text).
#[test]
fn constant_text_em_function() {
    let source = "constant prefix := \"Hello: \"\ngreet :: Text => Text\nlambda name: string_concat prefix name\n\necho!(greet \"World\")\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "Hello: World",
        "greet World com prefix — stdout: {stdout}"
    );
}

/// Constant Float usada em function.
#[test]
fn constant_float_em_function() {
    let source = "constant pi := 3.14\ncircunferencia :: Float => Float\nlambda r: * pi r\n\necho!(circunferencia 2.0)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "6.28",
        "circunferencia 2.0 com pi=3.14 = 6.28 — stdout: {stdout}"
    );
}

/// Shadowing: parâmetro com mesmo nome de constant não deve ser substituído.
#[test]
fn constant_shadowed_por_parametro() {
    let source = "constant scale := 2\nid :: Int => Int\nlambda scale: scale\n\necho!(id 42)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "42",
        "id 42 deve retornar 42 (parâmetro shadowa constant) — stdout: {stdout}"
    );
}

/// Múltiplas constants referenciadas na mesma function.
#[test]
fn multiplas_constants_em_function() {
    let source = "constant a := 10\nconstant b := 20\nsoma_consts :: Int => Int\nlambda x: + (+ a b) x\n\necho!(soma_consts 5)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "35",
        "soma_consts 5 = 10 + 20 + 5 = 35 — stdout: {stdout}"
    );
}

/// Constant referenciada em guard de function.
#[test]
fn constant_em_guard() {
    let source = "constant limite := 10\nverifica :: Int => Text\nlambda n:\n    > n limite: \"acima\"\n    otherwise: \"abaixo\"\n\necho!(verifica 5)\necho!(verifica 15)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "abaixo", "verifica 5 = abaixo — stdout: {stdout}");
    assert_eq!(lines[1], "acima", "verifica 15 = acima — stdout: {stdout}");
}

/// Constant referenciada em with_binding de function.
#[test]
fn constant_em_with_binding() {
    let source = "constant offset := 5\nsoma_offset :: Int => Int\nlambda x:\n    total\n    with\n        total := + x offset\n\necho!(soma_offset 10)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "15",
        "soma_offset 10 com offset=5 = 15 — stdout: {stdout}"
    );
}

/// Constant List (HeapSnapshot) acessada por function.
/// TODO: HeapSnapshot em function body requer que o codegen da função
/// tenha acesso ao snapshot_id. O fold substitui Ident por HeapSnapshot,
/// mas o codegen da função precisa de kata_rt_get_snapshot disponível.
#[test]
#[ignore = "HeapSnapshot em function body — Fase 3b"]
fn constant_list_em_function() {
    let source = "constant base := [1 2 3]\nprepend :: Int => List::Int\nlambda x: + [x] base\n\necho!(prepend 0)\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "[0, 1, 2, 3]",
        "prepend 0 em [1 2 3] = [0, 1, 2, 3] — stdout: {stdout}"
    );
}

/// Constant acessada dentro de action.
#[test]
fn constant_em_action() {
    let source = "constant msg := \"hello\"\naction imprimir => Unit\n    echo!(msg)\n\naction main => Int\n    fork!(imprimir, ())\n    sleep!(10)\n    0\n\nmain!()\n";
    let (stdout, stderr, code) = run_kata_run(source);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(
        first, "hello",
        "action imprimir deve imprimir constant msg — stdout: {stdout}"
    );
}

// ── Bug 2: DuplicateConstant em arquivo ───────────────────────────

/// `constant x := 10` + `constant x := 20` no mesmo arquivo deve
/// falhar com `type.duplicate_constant`. Constants são imutáveis —
/// redefinir o mesmo nome é erro de compilação.
#[test]
fn constant_redefinida_erro() {
    let source = "constant x := 10\nconstant x := 20\necho!(x)\n";
    let (_stdout, stderr, code) = run_kata_run(source);
    assert_ne!(code, 0, "kata run deve falhar — stderr: {stderr}");
    assert!(
        stderr.contains("duplicate_constant"),
        "esperava erro duplicate_constant — stderr: {stderr}"
    );
}

// ── Bug 3: ConstantNameCollision — constant vs function/action ────

/// `constant f := 10` quando `f` já é função nomeada deve falhar com
/// `type.constant_name_collision`. O nome colide com uma entidade
/// existente no módulo.
#[test]
fn constant_colide_com_function() {
    let source = "f :: Int => Int\nlambda x: + x 1\nconstant f := 10\necho!(f 5)\n";
    let (_stdout, stderr, code) = run_kata_run(source);
    assert_ne!(code, 0, "kata run deve falhar — stderr: {stderr}");
    assert!(
        stderr.contains("constant_name_collision"),
        "esperava erro constant_name_collision — stderr: {stderr}"
    );
}

/// Ordem inversa: constant antes da function também deve detectar colisão.
#[test]
fn constant_colide_com_function_ordem_inversa() {
    let source = "constant f := 10\nf :: Int => Int\nlambda x: + x 1\necho!(f 5)\n";
    let (_stdout, stderr, code) = run_kata_run(source);
    assert_ne!(code, 0, "kata run deve falhar — stderr: {stderr}");
    assert!(
        stderr.contains("constant_name_collision"),
        "esperava erro constant_name_collision — stderr: {stderr}"
    );
}

/// `constant foo := 10` quando `foo` já é action deve falhar com
/// `type.constant_name_collision`.
#[test]
fn constant_colide_com_action() {
    let source = "action foo (x::Int) => Unit\n  echo!(x)\n\nconstant foo := 10\necho!(foo)\n";
    let (_stdout, stderr, code) = run_kata_run(source);
    assert_ne!(code, 0, "kata run deve falhar — stderr: {stderr}");
    assert!(
        stderr.contains("constant_name_collision"),
        "esperava erro constant_name_collision — stderr: {stderr}"
    );
}
