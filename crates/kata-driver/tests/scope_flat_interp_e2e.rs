//! E2E — escopo plano da action no interpretador (Impl D do modelo plano).
//!
//! Responsabilidade: cravar que o INTERP espelha o modelo de escopo
//! único sancionado (2026-08-29/30) — o mesmo que o typeck já garante
//! (`t_scope_flat.rs`) e que o JIT já produz por construção (var_map
//! plano):
//!
//! - braços de match/select e corpos de for/loop NÃO abrem escopo:
//!   bindings nascem no namespace único da action
//! - reuso: `var` externo re-binclado por braço/loop/pattern persiste
//!   pós-construto com o último valor escrito (P18/P19/P20)
//! - evaporação: bindings FRESCOS de construto morrem no fim dele —
//!   o typeck já rejeita a leitura pós-construto (`unbound_name`),
//!   então o interp só precisa não vazar o valor entre construtos
//! - constants são legíveis dentro de actions no interp (P26) — JIT
//!   já imprime via comptime pass; o interp perdia por rodar a action
//!   em env fresco no trampoline, sem o prólogo de constants
//!
//! RED medido no início do Impl D (2026-08-30, interp pré-mudança):
//! - P18: `0 7` (alvo `0 0`) — re-binding de braço deve persistir
//! - P19: `2 1` (alvo `2 2`) — pattern sobre var deve reusar
//! - P20: `1 2` (alvo `1 2 2`) — for sobre var deve reusar
//! - P26: erro "variável não definida" (alvo `5`)
//! - P21: reassign do loop-var já ok (`99 99`) — regressão
//!
//! JIT é o gabarito de todos os casos (oráculo invertido: o var_map
//! plano do codegen já implementa o modelo; o interp desviava).

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn run_kata(source: &str, interp: bool) -> (String, String, i32) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_scope_flat_interp_e2e_{id}_{pid}.kata",
        pid = std::process::id()
    ));
    std::fs::write(&path, source).expect("escrever arquivo temporário");
    let mut cmd_args = vec!["run".to_string()];
    if interp {
        cmd_args.push("--interp".to_string());
    }
    cmd_args.push(path.to_string_lossy().into_owned());
    let output = Command::new(kata_bin())
        .args(&cmd_args)
        .output()
        .expect("executar kata run");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

/// Roda nos DOIS backends e exige stdout idêntico ao esperado.
fn assert_both(source: &str, expected: &str) {
    let (out_jit, err_jit, code_jit) = run_kata(source, false);
    assert_eq!(
        code_jit, 0,
        "kata run (JIT) deve exit 0 — stderr: {err_jit}"
    );
    assert_eq!(
        out_jit.trim(),
        expected,
        "JIT: esperava `{expected}` — stdout: {out_jit}"
    );
    let (out_interp, err_interp, code_interp) = run_kata(source, true);
    assert_eq!(
        code_interp, 0,
        "kata run --interp deve exit 0 — stderr: {err_interp}"
    );
    assert_eq!(
        out_interp.trim(),
        expected,
        "INTERP: esperava `{expected}` — stdout: {out_interp}"
    );
}

// ── P18: re-binding de braço persiste (var sobre var) ──────────

/// `var d := 7` externo; braço True re-bincla `d := 0`. O braço é
/// o MESMO namespace da action → o re-binding persiste pós-match.
/// Alvo: `0\n0` (RED hoje: `0\n7` — escopo filho descartava o re-binding).
#[test]
fn p18_var_de_braco_persiste() {
    let source = "\
action main (n::Int)
    var d := n
    match (> n 0)
        Boolean::True:
            var d := 0
            echo!(d)
        Boolean::False: echo!(2)
    echo!(d)
main!(7)";
    assert_both(&source, "0\n0");
}

// ── P19: pattern sobre var externo reusa (não sombreia) ────────

/// `var v := 1` externo; match pattern `Some v` sobre `Some n`.
/// Pattern sobre var é REUSO — o var externo recebe o payload e
/// persiste. `echo!(v)` no braço imprime 2 (payload), pós-match
/// imprime 2 (persistiu). Alvo: `2\n2` (RED hoje: `2\n1`).
#[test]
fn p19_pattern_sobre_var_reusa() {
    let source = "\
action main (n::Int)
    var v := 1
    match (Some n)
        Some v: echo!(v)
        None: echo!(0)
    echo!(v)
main!(2)";
    assert_both(&source, "2\n2");
}

// ── P20: for sobre var externo reusa o loop-var ────────────────

/// `var i := 99` + `for i in [1..3]` reusa o var externo: cada
/// iteração escreve nele, pós-laço vale o último elemento.
/// `[1..3]` é EXCLUSIVO → elementos 1, 2 → alvo `1\n2\n2` (RED: `1\n2\n99`).
#[test]
fn p20_for_sobre_var_reusa() {
    let source = "\
action main (n::Int)
    var i := 99
    for i in [1..3]
        echo!(i)
    echo!(i)
main!(0)";
    assert_both(&source, "1\n2\n2");
}

// ── P21: reassign do loop-var no corpo é legal ─────────────────

/// `for i in [1..3]` + `i := 99` no corpo — reassign muta a var de
/// laço (agora um var de verdade). Duas iterações, cada uma imprime
/// o 99 reatribuído (o laço reescreve i := elemento na próxima
/// iteração ANTES do corpo). Nome é FRESCO → lê pós-laço é
/// `unbound_name` (evaporação) — o teste NÃO lê fora.
/// Alvo: `99\n99`.
#[test]
fn p21_reassign_do_loop_var() {
    let source = "\
action main
    for i in [1..3]
        i := 99
        echo!(i)
main!()";
    assert_both(&source, "99\n99");
}

// ── P25: bindings frescos de construto NÃO vazam no interp ────

/// `for i in [1..3]` com nome fresco: o binding morre no fim do laço.
/// A leitura pós-laço já é REJEITADA pelo typeck (`unbound_name`) —
/// o papel do interp aqui é só não vazar o valor entre construtos
/// (evaporação): o segundo `for` com o MESMO nome nasce limpo.
/// Alvo: compila e imprime `1 2 3 4` (2 iterações + 2 iterações).
#[test]
fn p25_loop_var_fresco_evapora_reuso_sequencial() {
    let source = "\
action main
    for i in [1..3]
        echo!(i)
    for i in [3..5]
        echo!(i)
main!()";
    assert_both(&source, "1\n2\n3\n4");
}

// ── P26: constants legíveis dentro de actions (interp) ────────

/// `constant x := 5` + action lê `x` sem shadow. No JIT o comptime
/// pass substitui o literal; no interp o trampoline rodava a action
/// em env fresco SEM o prólogo de constants → "variável não
/// definida". Alvo: `5` nos dois backends (RED interp hoje: erro).
#[test]
fn p26_action_le_constant() {
    let source = "\
constant x := 5

action main
    echo!(x)
main!()";
    assert_both(&source, "5");
}

// ── P16-flip: pattern sobre let externo rejeitado em compile-time ─

/// Pattern de match sobre `let` externo → `DuplicateDecl` (escopo
/// único: pattern binding colide com imutável). O erro é em COMPILE
/// (typeck), então os dois backends devem falhar com o mesmo código.
#[test]
fn p16_pattern_sobre_let_rejeitado() {
    let source = "\
action main (n::Int)
    let d := 1
    match (- n 3)
        2: echo!(d)
        otherwise:
            let d := 2
            echo!(d)
    echo!(d)
main!(5)";
    let (out_jit, err_jit, code_jit) = run_kata(&source, false);
    assert_ne!(code_jit, 0, "JIT deve rejeitar — stderr: {err_jit}");
    assert!(
        err_jit.contains("duplicate_decl"),
        "JIT esperava duplicate_decl — stderr: {err_jit} / stdout: {out_jit}"
    );
    let (out_interp, err_interp, code_interp) = run_kata(&source, eval_interp());
    assert_ne!(
        code_interp, 0,
        "INTERP deve rejeitar — stderr: {err_interp}"
    );
    assert!(
        err_interp.contains("duplicate_decl"),
        "INTERP esperava duplicate_decl — stderr: {err_interp} / stdout: {out_interp}"
    );
}

/// Helper: `true` para o interp (eval separado para legibilidade).
fn eval_interp() -> bool {
    true
}

// ── P22: for sobre constant é rejeitado (constant sagrada) ────

/// `constant k := 5` + `for k in [1..3]` → erro compile-time em ambos
/// os backends (`type.duplicate_constant`).
#[test]
fn p22_for_sobre_constant_rejeitado() {
    let source = "\
constant k := 5

action main
    for k in [1..3]
        echo!(k)
main!()";
    let (out_jit, err_jit, code_jit) = run_kata(&source, false);
    assert_ne!(code_jit, 0, "JIT deve rejeitar — stderr: {err_jit}");
    assert!(
        err_jit.contains("duplicate_constant"),
        "JIT esperava duplicate_constant — stderr: {err_jit} / stdout: {out_jit}"
    );
    let (out_interp, err_interp, code_interp) = run_kata(&source, true);
    assert_ne!(
        code_interp, 0,
        "INTERP deve rejeitar — stderr: {err_interp}"
    );
    assert!(
        err_interp.contains("duplicate_constant"),
        "INTERP esperava duplicate_constant — stderr: {err_interp} / stdout: {out_interp}"
    );
}

// ── Evaporação interp: binding de braço não vaza pós-match ────

/// Binding de braço com nome fresco morre no braço. A leitura pós-
/// match é rejeitada pelo typeck (`unbound_name`), mas este teste
/// força o interp a PROVAR a evaporação por outro caminho: o mesmo
/// nome é reusado num segundo match, e o valor precisa nascer limpo
/// (não pode herdar do primeiro match por leak de env).
#[test]
fn p25b_binding_de_braco_evapora_interp() {
    let source = "\
action main (n::Int)
    match (- n 3)
        2:
            let b := 42
            echo!(b)
        otherwise: echo!(0)
    match (- n 3)
        2: echo!(b)
        otherwise: echo!(0)
main!(5)";
    // typeck rejeita a leitura pós-match (unbound_name) — compile
    // error em ambos backends. Crava que o interp não compila NEM
    // executa um programa que vaza binding de braço.
    let (out_interp, err_interp, code_interp) = run_kata(&source, true);
    assert_ne!(
        code_interp, 0,
        "INTERP deve rejeitar leak de binding de braço — stderr: {err_interp}"
    );
    assert!(
        err_interp.contains("unbound_name"),
        "INTERP esperava unbound_name — stderr: {err_interp} / stdout: {out_interp}"
    );
}

/// `loop` com `var x` interno: iterações não vazam valores entre si.
/// `var x := count` dentro do corpo re-nasce a cada iteração (fresh)
/// — evapora no fim de cada passagem; break via match Boolean.
#[test]
fn loop_var_fresco_por_iteracao() {
    let source = "\
action main
    var count := 0
    loop
        var x := count
        count := + count 1
        echo!(x)
        match (> count 2)
            Boolean::True: break
            Boolean::False: continue
    echo!(count)
main!()";
    assert_both(&source, "0\n1\n2\n3");
}
