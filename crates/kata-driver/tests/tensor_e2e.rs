//! Testes E2E de Tensor — criação, display, operações, indexação N-D.
//!
//! Fases 1-6 do PRD-tensor: runtime, AST, parser, inference, codegen, display.
//! Fase 7: estes testes E2E.

use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn run_kata(src: &str) -> (i32, String, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let bin = kata_bin();
    let tmp = std::env::temp_dir().join(format!(
        "kata_tensor_e2e_{}_{}.kata",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, src).expect("escrever arquivo temporário");

    let output = Command::new(&bin)
        .arg("run")
        .arg(&tmp)
        .output()
        .expect("executar kata run");

    let _ = std::fs::remove_file(&tmp);

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

// ── Criação e display ──────────────────────────────────────────

#[test]
fn tensor_1d_show() {
    let (code, stdout, _) = run_kata("echo!(show [1 2 3;])");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1  2  3");
}

#[test]
fn tensor_2d_show() {
    let (code, stdout, _) = run_kata("echo!(show [1 2 3; 4 5 6])");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1  2  3\n4  5  6");
}

#[test]
fn tensor_column_vector() {
    let (code, stdout, _) = run_kata("echo!(show [1; 2; 3])");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1\n2\n3");
}

// ── scale e shift ──────────────────────────────────────────────

#[test]
fn tensor_scale() {
    let (code, stdout, _) = run_kata("echo!(show (scale [1 2; 3 4] 2))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2  4\n6  8");
}

#[test]
fn tensor_shift() {
    let (code, stdout, _) = run_kata("echo!(show (shift [1 2; 3 4] 10))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "11  12\n13  14");
}

// ── _+ e _* (variantes pânicas) ────────────────────────────────

#[test]
fn tensor_panic_add() {
    let (code, stdout, _) = run_kata("echo!(show (_+ [1 2; 3 4] [5 6; 7 8]))");
    assert_eq!(code, 0);
    // Display alinha à direita: " 6   8" (6 e 8 têm 1 dígito, 10 e 12 têm 2)
    assert_eq!(stdout.trim_end(), " 6   8\n10  12");
}

#[test]
fn tensor_panic_mul() {
    let (code, stdout, _) = run_kata("echo!(show (_* [1 2; 3 4] [5 6; 7 8]))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), " 5  12\n21  32");
}

// ── Indexação .N (flatten via INDEXABLE) ───────────────────────

#[test]
fn tensor_dot_n_index() {
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].0))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Ok(1)");
}

// ── Indexação N-D .() escalar ──────────────────────────────────

#[test]
fn tensor_nd_scalar_index() {
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(0 1)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Ok(2)");
}

#[test]
fn tensor_nd_scalar_index_last() {
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(1 2)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Ok(6)");
}

// ── Indexação N-D .() com Wildcard ─────────────────────────────

#[test]
fn tensor_nd_wildcard_all() {
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(_ _)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1  2  3\n4  5  6");
}

// ── Indexação N-D .() com Range ────────────────────────────────

#[test]
fn tensor_nd_range_submatrix() {
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(0..2 0..2)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1  2\n4  5");
}

// ── Transpose display (strides-aware) ──────────────────────────

#[test]
fn tensor_transpose_display() {
    // [1 2 3; 4 5 6] transposto deve mostrar [1 4; 2 5; 3 6]
    // Antes do fix de format_tensor, display ignorava strides e mostrava
    // os dados na ordem original (row-major), produzindo output errado.
    let (code, stdout, _) = run_kata("echo!(show (transpose [1 2 3; 4 5 6]))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1  4\n2  5\n3  6");
}

#[test]
fn tensor_transpose_square_display() {
    // Transposta de matriz quadrada
    let (code, stdout, _) = run_kata("echo!(show (transpose [1 2; 3 4]))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1  3\n2  4");
}

// ── Sub-tensor display ─────────────────────────────────────────

#[test]
fn tensor_sub_wildcard_column() {
    // m.(_ 1) de [1 2 3; 4 5 6] → 1-D tensor shape [2] (Wildcard keep, Int collapse)
    // Valores: elemento [0,1]=2 e [1,1]=5
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(_ 1)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2  5");
}

#[test]
fn tensor_sub_wildcard_all_columns() {
    // m.(_ _) de [1 2 3; 4 5 6] → 2-D tensor shape 2×3 (ambos Wildcard)
    let (code, stdout, _) = run_kata("echo!(show ([1 2 3; 4 5 6].(_ _)))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1  2  3\n4  5  6");
}

// ── Construtores: eye e zeros ──────────────────────────────────

#[test]
fn tensor_eye_3() {
    let (code, stdout, _) = run_kata("echo!(show (eye 3))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1  0  0\n0  1  0\n0  0  1");
}

#[test]
fn tensor_eye_1() {
    let (code, stdout, _) = run_kata("echo!(show (eye 1))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
}

#[test]
fn tensor_zeros_2d() {
    let (code, stdout, _) = run_kata("echo!(show (zeros {2 3}))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "0  0  0\n0  0  0");
}

#[test]
fn tensor_zeros_1d() {
    let (code, stdout, _) = run_kata("echo!(show (zeros {4}))");
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "0  0  0  0");
}

#[test]
fn tensor_eye_add_zeros() {
    // eye(3) + zeros({3 3}) = eye(3) — identidade é neutra para +
    // Usa _+ (panic variant) entre tensores Int
    let src = r#"action main
    let i := eye 3
    let z := zeros {3 3}
    echo!(show (_+ i z))
main!()"#;
    let (code, stdout, _) = run_kata(src);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1  0  0\n0  1  0\n0  0  1");
}

// ── Programa completo (operações que funcionam corretamente) ───

#[test]
fn tensor_full_program() {
    let src = r#"action main
    let m := [1 2; 3 4]
    echo!(show m)
    echo!(show (scale m 2))
    echo!(show (shift m 10))
    echo!(show (_+ m m))
    echo!(show (m.(0 1)))
main!()
"#;
    let (code, stdout, _) = run_kata(src);
    assert_eq!(code, 0);
    // m = [1 2; 3 4], scale*2 = [2 4; 6 8], shift+10 = [11 12; 13 14]
    // _+ m m = [2 4; 6 8] (todos 1 dígito, sem padding), m.(0 1) = Ok(2)
    let expected = "1  2\n3  4\n2  4\n6  8\n11  12\n13  14\n2  4\n6  8\nOk(2)";
    assert_eq!(stdout.trim_end(), expected);
}
