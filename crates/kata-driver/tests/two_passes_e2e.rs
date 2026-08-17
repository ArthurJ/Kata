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

// ── Dict dispatch em actions com `!{}` (Fase 5) ──────────────────

/// `action soma (a::Int, b::Int) => Int` + `soma!{"a": 3 "b": 4}` → 7.
/// Via `kata run` — testa dict nomeado em action com o ciclo de dois passes.
#[test]
fn kata_run_dict_dispatch_sem_bang() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\naction main => Unit\n    echo!(soma!{\"a\": 3 \"b\": 4})\nmain!()";
    let path = write_temp_kata("dict_sem_bang", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "7", "soma com dict nomeado deve imprimir 7");
}

/// Ordem invertida das chaves também funciona via `kata run`.
#[test]
fn kata_run_dict_dispatch_ordem_invertida() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\naction main => Unit\n    echo!(soma!{\"b\": 4 \"a\": 3})\nmain!()";
    let path = write_temp_kata("dict_invertido", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        "7",
        "soma com dict ordem invertida deve imprimir 7"
    );
}

/// Action com 1 param nomeado via `kata run`.
/// `action dobro (x::Int) => Int` + `dobro!{"x": 21}` → 42
#[test]
fn kata_eval_dict_dispatch_um_param() {
    let src = "action dobro (x::Int) => Int\n    * x 2\naction main => Unit\n    echo!(dobro!{\"x\": 21})\nmain!()";
    let path = write_temp_kata("dict_um_param", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "42", "dobro com dict deve imprimir 42");
}

/// Whitespace entre `!` e `{` não distingue: `soma! {"a": 1 "b": 2}` é o mesmo.
#[test]
fn kata_run_dict_dispatch_whitespace_nao_distingue() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\naction main => Unit\n    echo!(soma! {\"a\": 1 \"b\": 2})\nmain!()";
    let path = write_temp_kata("dict_whitespace", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        "3",
        "soma com dict e whitespace deve imprimir 3"
    );
}

/// Action com 3 params nomeados e ordem embaralhada.
/// `action sub (x::Int, y::Int, z::Int) => Int` + `sub!{"z": 1 "x": 10 "y": 3}` → 6
#[test]
fn kata_run_dict_dispatch_tres_params_embaralhados() {
    let src = "action sub (x::Int, y::Int, z::Int) => Int\n    - (- x y) z\naction main => Unit\n    echo!(sub!{\"z\": 1 \"x\": 10 \"y\": 3})\nmain!()";
    let path = write_temp_kata("dict_3params", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(
        stdout.trim(),
        "6",
        "sub com 3 params embaralhados deve imprimir 6"
    );
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

/// `+{"a": 1 "b": 2}` deve dar erro — `+` tem aridade 2, o parser
/// arity-aware coleta o DictLit como 1º arg, depois espera o 2º arg
/// e encontra EOF. Erro de parser (aridade), não de tipo.
#[test]
fn erro_dict_sem_params_nomeados() {
    let src = "+{\"a\": 1 \"b\": 2}";
    let path = write_temp_kata("dict_sem_params", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — + não aceita Dict como argumento");
    assert!(
        stderr.contains("aridade")
            || stderr.contains("argumento")
            || stderr.contains("token inesperado")
            || stderr.contains("NoOverload")
            || stderr.contains("parâmetro"),
        "erro deve mencionar aridade, token inesperado ou parâmetro, got: {stderr}"
    );
}

/// Função posicional chamada com dict como valor deve dar erro.
/// `soma :: Int Int => Int` + `soma{"a": 1 "b": 2}` → parser arity-aware
/// coleta DictLit como 1º arg, espera 2º arg, encontra EOF.
#[test]
fn erro_funcao_sem_params_nomeados_com_dict() {
    let src = "soma :: Int Int => Int\nlambda a b: + a b\nsoma{\"a\": 1 \"b\": 2}";
    let path = write_temp_kata("fn_sem_named", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — soma espera Int Int, recebe Dict");
    assert!(
        stderr.contains("aridade")
            || stderr.contains("argumento")
            || stderr.contains("token inesperado")
            || stderr.contains("NoOverload")
            || stderr.contains("parâmetro"),
        "erro deve mencionar aridade, token inesperado ou parâmetro, got: {stderr}"
    );
}

/// Chave inexistente no dict nomeado de action deve dar erro.
/// `action soma (a::Int, b::Int) => Int` + `soma!{"a": 1 "x": 2}`
/// → `x` não é param de soma → erro no reorder.
#[test]
fn erro_chave_inexistente_no_dict() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\naction main => Unit\n    echo!(soma!{\"a\": 1 \"x\": 2})\nmain!()";
    let path = write_temp_kata("dict_chave_errada", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — chave 'x' não é param de soma");
    assert!(
        stderr.contains("não existe")
            || stderr.contains("não é parâmetro")
            || stderr.contains("parâmetro"),
        "erro deve mencionar param inexistente, got: {stderr}"
    );
}

/// Param faltante no dict nomeado de action deve dar erro.
/// `action soma (a::Int, b::Int) => Int` + `soma!{"a": 1}` — `b` faltante.
#[test]
fn erro_param_faltante_no_dict() {
    let src = "action soma (a::Int, b::Int) => Int\n    + a b\naction main => Unit\n    echo!(soma!{\"a\": 1})\nmain!()";
    let path = write_temp_kata("dict_param_faltante", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — param 'b' faltante");
    assert!(
        stderr.contains("não foi fornecido")
            || stderr.contains("faltando")
            || stderr.contains("parâmetro"),
        "erro deve mencionar param faltante, got: {stderr}"
    );
}

// ── scan_lambdas — arity-aware para let-bound lambdas ───────────

/// `let f := lambda x: + x 1` + `f (* 2 2)` → f(4) = 5
/// Com aridade 1, f coleta 1 arg. O grouping `(* 2 2)` é o argumento.
#[test]
fn let_lambda_sub_aplicacao_com_grouping() {
    let src = "f :: Int => Int\nlambda x: + x 1\nf (* 2 2)";
    let path = write_temp_kata("let_lambda_sub_group", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "5", "f (* 2 2) deve imprimir 5 (f(4))");
}

/// `let f := lambda x: + x 1` + `f 5` → f(5) = 6
/// Aridade 1, chamada simples.
#[test]
fn let_lambda_chamada_simples() {
    let src = "f :: Int => Int\nlambda x: + x 1\nf 5";
    let path = write_temp_kata("let_lambda_simples", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "6", "f 5 deve imprimir 6");
}

/// `let f := lambda x: + x 1` + `f 1 2 3` → erro de excesso posicional
/// (f tem aridade 1, recebeu 3)
#[test]
fn let_lambda_excesso_posicional() {
    let src = "f :: Int => Int\nlambda x: + x 1\nf 1 2 3";
    let path = write_temp_kata("let_lambda_excesso", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(code, 0, "deve falhar — f tem aridade 1, recebeu 3");
    assert!(
        stderr.contains("aridade padrão 1") || stderr.contains("excesso"),
        "erro deve mencionar aridade padrão 1 ou excesso, got: {stderr}"
    );
}

/// `let f := lambda x: + x 1` seguido de `+ 5 * 2 2` — função prelude
/// ainda funciona (não só lambdas).
#[test]
fn let_lambda_e_prelude_ambos_funcionam() {
    let src = "f :: Int => Int\nlambda x: + x 1\n+ 5 * 2 2";
    let path = write_temp_kata("let_lambda_e_prelude", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "9", "+ 5 * 2 2 deve imprimir 9");
}

/// `let n := 42` (não-lambda) não deve quebrar — scan_lambdas skipa.
#[test]
fn let_nao_lambda_nao_quebra() {
    let src = "constant n := 42\nn";
    let path = write_temp_kata("let_nao_lambda", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "42", "n deve imprimir 42");
}

/// Lambda com 1 param: `let g := lambda x: - x 1`
/// A aridade 1 é extraída pelo scan. `- x 1` constrain x to Int.
/// `g (* 4 5)` → g(20) = 19
#[test]
fn let_lambda_1param_aridade_extraida() {
    let src = "g :: Int => Int\nlambda x: - x 1\ng (* 4 5)";
    let path = write_temp_kata("let_lambda_1param", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "19", "g (* 4 5) = g(20) = 20 - 1 = 19");
}

/// Função pura não aceita dict dispatch — `f{\"x\": 5}` sem `!` é dict
/// como valor posicional. `f :: Int => Int` recebe `Dict` → type error.
/// Antes da Fase 1, `f :: (x::Int) => Int` + `f{\"x\": 5}` despachava via
/// `param_names` no TypeEnv. Agora funções são exclusivamente posicionais.
#[test]
fn let_lambda_dict_dispatch_via_typeenv() {
    let src = "f :: Int => Int\nlambda x: - x 1\nf{\"x\": 5}";
    let path = write_temp_kata("let_lambda_dict", src);
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert_ne!(
        code, 0,
        "deve falhar — função pura não aceita dict dispatch"
    );
    assert!(
        stderr.contains("NoOverload")
            || stderr.contains("TypeMismatch")
            || stderr.contains("nenhuma sobrecarga")
            || stderr.contains("sobrecarga")
            || stderr.contains("não é parâmetro")
            || stderr.contains("parâmetro"),
        "erro deve mencionar type mismatch (função recebe Dict, espera Int), got: {stderr}"
    );
}

// ── Default args via kata run ─────────────────────────────────

/// Default args via chamada nomeada omitindo default — `kata run`.
/// `action act{msg::Text: _, dft::Int: 5}` + `act!{"msg": "hi"}` → dft=5.
#[test]
fn kata_run_default_args_nomeada_omitindo_default() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\naction main => Unit\n    echo!(act!{\"msg\": \"hi\"})\nmain!()";
    let path = write_temp_kata("default_omit", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "7", "5 + len(\"hi\") = 7");
}

/// Default args via chamada posicional omitindo default — `kata run`.
/// `act!("hi")` → msg="hi", dft=5 (default).
#[test]
fn kata_run_default_args_posicional_omitindo_default() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\naction main => Unit\n    echo!(act!(\"hi\"))\nmain!()";
    let path = write_temp_kata("default_pos_omit", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "7", "5 + len(\"hi\") = 7");
}

/// Default args sobrescrevendo default na chamada nomeada — `kata run`.
/// `act!{"msg": "hi" "dft": 10}` → dft=10.
#[test]
fn kata_run_default_args_sobrescrevendo_default() {
    let src = "action act{msg::Text: _, dft::Int: 5} => Int\n    + dft (len msg)\naction main => Unit\n    echo!(act!{\"msg\": \"hi\" \"dft\": 10})\nmain!()";
    let path = write_temp_kata("default_override", src);
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "kata run deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout.trim(), "12", "10 + len(\"hi\") = 12");
}
