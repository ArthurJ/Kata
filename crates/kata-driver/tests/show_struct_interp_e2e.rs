//! E2E — show de structs com campos no interpretador.
//!
//! Responsabilidade: cravar que `echo!` de um struct com campos mostra
//! `Nome(v0, v1, ...)` (mesma regra do codegen em `build_struct_show_body`).
//! Antes do fix do A8, o interp sempre retornava `Nome()` (vazio) porque
//! `struct_field_count` retornava 0 hardcoded.
//!
//! Também verifica paridade interp↔JIT para todos os casos — Text dentro
//! de struct deve ser quoteado (`"João"`, não `João`), espelhando `repr_expr`
//! do codegen. List/Array dentro de struct usam `, ` como separador.

use std::process::Command;

/// Roda `kata run [--interp] <src>` e retorna (stdout, stderr, code).
fn run_kata(path: &str, interp: bool) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kata"));
    if interp {
        cmd.args(["run", "--interp", path]);
    } else {
        cmd.args(["run", path]);
    }
    let out = cmd.output().expect("kata run deve executar");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// Escreve source num .kata temporário de nome ÚNICO e retorna o path.
fn write_temp(name: &str, src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kata_show_struct_e2e_{name}_{id}_{}.kata",
        std::process::id()
    ));
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().to_string()
}

/// Ambos os backends devem ter exit 0 e o mesmo stdout.
fn assert_both(src: &str, expected: &str) {
    let path = write_temp("case", src);
    let (out_i, err_i, code_i) = run_kata(&path, true);
    assert_eq!(
        code_i, 0,
        "interp deve exit 0 — stderr: {err_i}\nstdout: {out_i}"
    );
    assert_eq!(out_i, expected, "interp: stdout divergente");
    let (out_j, err_j, code_j) = run_kata(&path, false);
    assert_eq!(
        code_j, 0,
        "JIT deve exit 0 — stderr: {err_j}\nstdout: {out_j}"
    );
    assert_eq!(out_j, expected, "JIT: stdout divergente");
}

/// Struct com Text + Int: `Pessoa("João", 30)` — Text é quoteado dentro de struct.
#[test]
fn show_struct_text_int() {
    let source = r#"data Pessoa (nome::Text idade::Int)

action main => Unit
    let p := Pessoa "João" 30
    echo!(p)
main!()"#;
    assert_both(source, "Pessoa(\"João\", 30)\n");
}

/// Struct com dois Ints: `Ponto(3, 4)`.
#[test]
fn show_struct_dois_ints() {
    let source = r#"data Ponto (x::Int y::Int)

action main => Unit
    let p := Ponto 3 4
    echo!(p)
main!()"#;
    assert_both(source, "Ponto(3, 4)\n");
}

/// Struct com Float: `Medida(1.5, 2.5)`.
#[test]
fn show_struct_float() {
    let source = r#"data Medida (a::Float b::Float)

action main => Unit
    let m := Medida 1.5 2.5
    echo!(m)
main!()"#;
    assert_both(source, "Medida(1.5, 2.5)\n");
}

/// Struct com Boolean: `Flag(True, 42)`.
#[test]
fn show_struct_boolean() {
    let source = r#"data Flag (ativo::Boolean id::Int)

action main => Unit
    let f := Flag True 42
    echo!(f)
main!()"#;
    assert_both(source, "Flag(True, 42)\n");
}

/// Struct aninhado: `Pessoa("João", Ponto(3, 4))`.
#[test]
fn show_struct_aninhado() {
    let source = r#"data Ponto (x::Int y::Int)
data Pessoa (nome::Text loc::Ponto)

action main => Unit
    let p := Pessoa "João" (Ponto 3 4)
    echo!(p)
main!()"#;
    assert_both(source, "Pessoa(\"João\", Ponto(3, 4))\n");
}

/// `show` despacha por tipo — dois structs diferentes, mesmo nome "show".
#[test]
fn show_despacha_por_tipo() {
    let source = r#"data Pessoa (nome::Text idade::Int)
data Ponto (x::Int y::Int)

action main => Unit
    let p := Pessoa "João" 30
    let pt := Ponto 3 4
    echo!(show p)
    echo!(show pt)
main!()"#;
    assert_both(source, "Pessoa(\"João\", 30)\nPonto(3, 4)\n");
}

/// Struct com List como campo: `Caixa([1, 2, 3])`.
#[test]
fn show_struct_com_list() {
    let source = r#"data Caixa (items::List::Int)

action main => Unit
    let c := Caixa [1 2 3]
    echo!(c)
main!()"#;
    assert_both(source, "Caixa([1, 2, 3])\n");
}

/// Struct com List de Text: `Caixa(["a", "b"])` — Text dentro de List é quoteado.
#[test]
fn show_struct_com_list_text() {
    let source = r#"data Caixa (items::List::Text)

action main => Unit
    let c := Caixa ["a" "b"]
    echo!(c)
main!()"#;
    assert_both(source, "Caixa([\"a\", \"b\"])\n");
}

/// Struct com Sum/Enum como campo: `Box(Some(42))`.
#[test]
fn show_struct_com_sum() {
    let source = r#"enum Optional
    Some(Int)
    None

data Box (val::Optional)

action main => Unit
    let b := Box (Some 42)
    echo!(b)
main!()"#;
    assert_both(source, "Box(Some(42))\n");
}

/// Struct com Rational: `Rat(1/2)`.
#[test]
fn show_struct_rational() {
    let source = r#"data Rat (val::Rational)

action main => Unit
    let r := Rat (rational 1)
    echo!(r)
main!()"#;
    assert_both(source, "Rat(1)\n");
}

/// Struct com Tuple como campo: `Par((1, 2))`.
#[test]
fn show_struct_com_tuple() {
    let source = r#"data Par (t::(Int, Int))

action main => Unit
    let p := Par ((1, 2))
    echo!(p)
main!()"#;
    assert_both(source, "Par((1, 2))\n");
}

/// Struct com Tuple de Text: `Par(("x", 2))` — Text dentro de Tuple é quoteado.
#[test]
fn show_struct_com_tuple_text() {
    let source = r#"data Par (t::(Text, Int))

action main => Unit
    let p := Par ("x", 2)
    echo!(p)
main!()"#;
    assert_both(source, "Par((\"x\", 2))\n");
}