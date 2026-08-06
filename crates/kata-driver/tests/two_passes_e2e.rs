//! Testes E2E do ciclo de dois passes (Fase 4 — arity-uniformization).
//!
//! Valida que `kata run` e `kata eval` executam o ciclo de dois passes:
//! Pass 1 (parse_decls_only → resolve → extract_arities) → Pass 2
//! (parse_with_arity → resolve → infer → codegen).
//!
//! Testa via subprocesso `kata run` e `kata eval` — o caminho real do driver.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-arity");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata_file(file: &str) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(["run", file])
        .output()
        .expect("executar kata run");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

fn eval_kata(expr: &str) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(["eval", expr])
        .output()
        .expect("executar kata eval");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

// ── Sub-aplicação via kata run (DoD Fase 4) ─────────────────────

/// `+ 5 * 2 2` deve retornar 9 via `kata run` — o ciclo de dois passes
/// extrai aridades do prelude e do módulo, depois parseia arity-aware.
#[test]
fn kata_run_sub_aplicacao() {
    let path = write_temp_kata("sub_aplicacao", "+ 5 * 2 2");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "9", "+ 5 * 2 2 deve imprimir 9");
}

/// `* 5 + 2 2` deve retornar 20 via `kata run`.
#[test]
fn kata_run_sub_aplicacao_aninhada() {
    let path = write_temp_kata("sub_aninhada", "* 5 + 2 2");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "20", "* 5 + 2 2 deve imprimir 20");
}

/// `+ 1 2 3` com aridade 2 deve dar erro de parser via `kata run`.
#[test]
fn kata_run_excesso_posicional() {
    let path = write_temp_kata("excesso", "+ 1 2 3");
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "kata run deve falhar com erro de parser");
    assert!(
        stderr.contains("aridade padrão 2") || stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 2 ou excesso, got: {stderr}"
    );
    let _ = stdout;
}

/// Função do usuário com aridade conhecida via `kata run`.
/// `soma :: Int Int => Int` + `soma 3 * 4 5` → 23
#[test]
fn kata_run_funcao_usuario_com_sub_aplicacao() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma 3 * 4 5";
    let path = write_temp_kata("fn_usuario", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "23", "soma 3 * 4 5 deve imprimir 23");
}

// ── Sub-aplicação via kata eval ──────────────────────────────────

/// `kata eval "+ 5 * 2 2"` deve retornar 9.
#[test]
fn kata_eval_sub_aplicacao() {
    let (stdout, stderr, code) = eval_kata("+ 5 * 2 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "9", "+ 5 * 2 2 deve imprimir 9");
}

/// `kata eval "+ 1 2 3"` deve dar erro de parser.
#[test]
fn kata_eval_excesso_posicional() {
    let (stdout, stderr, code) = eval_kata("+ 1 2 3");
    assert_ne!(code, 0, "kata eval deve falhar com erro de parser");
    assert!(
        stderr.contains("aridade padrão 2") || stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 2 ou excesso, got: {stderr}"
    );
    let _ = stdout;
}

/// `kata eval "+ 1 2"` deve retornar 3 (aridade simples).
#[test]
fn kata_eval_aridade_simples() {
    let (stdout, stderr, code) = eval_kata("+ 1 2");
    assert_eq!(code, 0, "kata eval deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "3", "+ 1 2 deve imprimir 3");
}

// ── Dict dispatch sem `!` (Fase 5) ───────────────────────────────

/// `soma{"a": 3, "b": 4}` deve funcionar como dict dispatch de função pura
/// via `kata run` — sem `!`, usando o ciclo de dois passes.
#[test]
fn kata_run_dict_dispatch_sem_bang() {
    let src = "soma :: (a::Int) (b::Int) => Int\nlambda a b: + a b\nsoma{\"a\": 3 \"b\": 4}";
    let path = write_temp_kata("dict_sem_bang", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "7", "soma com dict sem bang deve imprimir 7");
}

/// Ordem invertida das chaves também funciona via `kata run`.
#[test]
fn kata_run_dict_dispatch_ordem_invertida() {
    let src = "soma :: (a::Int) (b::Int) => Int\nlambda a b: + a b\nsoma{\"b\": 4 \"a\": 3}";
    let path = write_temp_kata("dict_invertido", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "7", "soma com dict ordem invertida deve imprimir 7");
}

/// Função pura com 1 param nomeado via `kata eval`.
/// `dobro :: (x::Int) => Int` + `dobro{"x": 21}` → 42
#[test]
fn kata_eval_dict_dispatch_um_param() {
    let src = "dobro :: (x::Int) => Int\nlambda x: * x 2\ndobro{\"x\": 21}";
    let path = write_temp_kata("dict_um_param", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "42", "dobro com dict deve imprimir 42");
}

/// Whitespace não distingue: `soma {\"a\": 1 \"b\": 2}` é o mesmo dict dispatch.
#[test]
fn kata_run_dict_dispatch_whitespace_nao_distingue() {
    let src = "soma :: (a::Int) (b::Int) => Int\nlambda a b: + a b\nsoma {\"a\": 1 \"b\": 2}";
    let path = write_temp_kata("dict_whitespace", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "3", "soma com dict e whitespace deve imprimir 3");
}

/// Função com 3 params nomeados e ordem embaralhada.
/// `sub :: (x::Int) (y::Int) (z::Int) => Int` + `sub{"z": 1 "x": 10 "y": 3}` → 6
#[test]
fn kata_run_dict_dispatch_tres_params_embaralhados() {
    let src = "sub :: (x::Int) (y::Int) (z::Int) => Int\nlambda x y z: - (- x y) z\nsub{\"z\": 1 \"x\": 10 \"y\": 3}";
    let path = write_temp_kata("dict_3params", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "6", "sub com 3 params embaralhados deve imprimir 6");
}

// ── Validação e mensagens de erro (Fase 6) ───────────────────────

/// `+ 1 2 3` deve dar erro claro mencionando aridade padrão 2 e excesso.
#[test]
fn erro_excesso_posicional_mensagem_clara() {
    let path = write_temp_kata("erro_excesso", "+ 1 2 3");
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar");
    assert!(
        stderr.contains("aridade padrão 2") && stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 2 e excesso, got: {stderr}"
    );
}

/// `+{"a": 1}` deve dar erro — `+` não tem params nomeados.
#[test]
fn erro_dict_sem_params_nomeados() {
    let src = "+{\"a\": 1 \"b\": 2}";
    let path = write_temp_kata("dict_sem_params", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — + não tem params nomeados");
    assert!(
        stderr.contains("não é parâmetro") || stderr.contains("parâmetro"),
        "erro deve mencionar parâmetro inexistente, got: {stderr}"
    );
}

/// Função sem params nomeados chamada com dict deve dar erro.
#[test]
fn erro_funcao_sem_params_nomeados_com_dict() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma{\"a\": 1 \"b\": 2}";
    let path = write_temp_kata("fn_sem_named", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — soma não tem params nomeados");
    assert!(
        stderr.contains("não é parâmetro") || stderr.contains("parâmetro"),
        "erro deve mencionar parâmetro inexistente, got: {stderr}"
    );
}

/// Chave inexistente no dict deve dar erro mencionando o param.
#[test]
fn erro_chave_inexistente_no_dict() {
    let src = "soma :: (a::Int) (b::Int) => Int\nlambda a b: + a b\nsoma{\"a\": 1 \"x\": 2}";
    let path = write_temp_kata("dict_chave_errada", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — chave 'x' não existe");
    assert!(
        stderr.contains("não existe") || stderr.contains("não é parâmetro") || stderr.contains("parâmetro"),
        "erro deve mencionar param inexistente, got: {stderr}"
    );
}

/// Param faltante no dict deve dar erro.
#[test]
fn erro_param_faltante_no_dict() {
    let src = "soma :: (a::Int) (b::Int) => Int\nlambda a b: + a b\nsoma{\"a\": 1}";
    let path = write_temp_kata("dict_param_faltante", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — param 'b' faltante");
    assert!(
        stderr.contains("não foi fornecido") || stderr.contains("faltando") || stderr.contains("parâmetro"),
        "erro deve mencionar param faltante, got: {stderr}"
    );
}