//! Testes E2E de exaustividade aninhada — PRD-exaustividade-aninhada.
//!
//! Oráculos copiados mecanicamente de `tests/probe-nested/`. Cada teste
//! executa `kata run` (JIT) e verifica exit code + output/diagnóstico.
//!
//! Fase 1 (oráculos RED): probeE e probeK_arity_tuple devem dar
//! `ArityMismatch` gracioso (hoje panic 101). Os demais oráculos F2+
//! entram `#[ignore]` até a fase-alvo.
//!
//! Controles verdes (probeC, probeD, probeF2, probeG, probeJ2, probeK_deep,
//! probeK_grid, probeK_deep_paren, RatUmOuDois_wildcard) permanecem verdes.

use std::fs;
use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

fn write_temp_kata(name: &str, content: &str) -> String {
    let dir = std::env::temp_dir().join("kata-driver-e2e-nested");
    fs::create_dir_all(&dir).expect("criar temp dir");
    let path = dir.join(format!("{name}.kata"));
    fs::write(&path, content).expect("escrever .kata temporário");
    path.to_string_lossy().to_string()
}

fn run_kata(args: &[&str]) -> (String, String, i32) {
    let result = Command::new(kata_bin())
        .args(args)
        .output()
        .expect("executar kata");
    (
        String::from_utf8_lossy(&result.stdout).to_string(),
        String::from_utf8_lossy(&result.stderr).to_string(),
        result.status.code().unwrap_or(-1),
    )
}

fn run_kata_file(path: &str) -> (String, String, i32) {
    run_kata(&["run", path])
}

// ── Controles verdes (regressão) ───────────────────────────────

/// probeC: Some True + Some False + None — cobertura completa do payload.
#[test]
fn probe_c_completo_verde() {
    let path = write_temp_kata(
        "probeC",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        Some False: "tem false"
        None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeC deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\ntem false\n");
}

/// probeD: cobertura completa + chamada só com Some True.
#[test]
fn probe_d_completo_verde() {
    let path = write_temp_kata(
        "probeD",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        Some False: "tem false"
        None: "nada"

action main
    echo!(foo (Some True))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeD deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem true\n");
}

/// probeF2: wildcard sobre refined — controle verde.
#[test]
fn probe_f2_wildcard_refined_verde() {
    let path = write_temp_kata(
        "probeF2",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        _ : "algum"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeF2 deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "algum\n");
}

/// probeG: pattern aninhado qualificado + guards dentro da cláusula.
#[test]
fn probe_g_guards_intra_clausula_verde() {
    let path = write_temp_kata(
        "probeG",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    > x 0: "positivo"
    <= x 0: "zero ou negativo"
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeG deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}

/// probeJ2: otherwise inútil pós-cobertura — isento (verde).
#[test]
fn probe_j2_otherwise_inutil_verde() {
    let path = write_temp_kata(
        "probeJ2",
        r#"foo :: Result::(Int, Text) => Text
lambda m:
    match m
        Ok v: "tem"
        Err _: "erro"
        otherwise: "impossivel"

action main
    echo!(foo (Ok 42))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeJ2 deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "tem\n");
}

/// probeK_deep: 3 níveis completo — regressão da matriz.
#[test]
fn probe_k_deep_completo_verde() {
    let path = write_temp_kata(
        "probeK_deep",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some Optional::Some True: "true dentro"
        Some Optional::Some False: "false dentro"
        Some Optional::None: "sem dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))
    echo!(foo (Some (Some False)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_deep deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "true dentro\nfalse dentro\n");
}

/// probeK_grid: 2 params Boolean Boolean — grade 2×2 completa.
#[test]
fn probe_k_grid_completo_verde() {
    let path = write_temp_kata(
        "probeK_grid",
        r#"bar :: Boolean Boolean => Text
lambda True True: "vv"
lambda True False: "vf"
lambda False True: "fv"
lambda False False: "ff"

action main
    echo!(bar True True)
    echo!(bar False False)

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_grid deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "vv\nff\n");
}

/// probeK_deep_paren: parêntese interno em braço — resolvido na Fase 0.
#[test]
fn probe_k_deep_paren_verde() {
    let path = write_temp_kata(
        "probeK_deep_paren",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some (Some True): "true dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeK_deep_paren deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "true dentro\n");
}

/// RatUmOuDois_wildcard: wildcard sobre Rational refined — controle F5.
#[test]
fn rat_um_ou_dois_wildcard_verde() {
    let path = write_temp_kata(
        "RatUmOuDois_wildcard",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        _ : "algum"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(
        code, 0,
        "RatUmOuDois_wildcard deve exit 0 — stderr: {stderr}"
    );
    assert_eq!(stdout, "algum\n");
}

// ── RED F1: ArityMismatch (hoje panic 101) ─────────────────────

/// probeE: `lambda Some True:` em função de 1 param — hoje panic 101
/// (helpers.rs:104, index out of bounds). F1: ArityMismatch gracioso.
#[test]
fn probe_e_arity_mismatch_nao_panic() {
    let path = write_temp_kata(
        "probeE",
        r#"foo :: Optional::(Boolean) => Text
lambda Some True: "tem true"
lambda Some False: "tem false"
lambda None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);

    // NÃO pode panicar (exit 101). Deve ser erro gracioso.
    assert_ne!(code, 101, "probeE não deve panicar — stderr: {stderr}");
    assert!(
        stderr.contains("type.arity_mismatch"),
        "probeE deve dar ArityMismatch — stderr: {stderr}"
    );
}

/// probeK_arity_tuple: 2 patterns contra 1 param tupla — hoje panic 101.
/// F1: ArityMismatch gracioso (bound-check §4.2).
#[test]
fn probe_k_arity_tuple_arity_mismatch_nao_panic() {
    let path = write_temp_kata(
        "probeK_arity_tuple",
        r#"bar :: (Boolean, Boolean) => Text
lambda True True: "vv"
lambda True False: "vf"

action main
    echo!(bar (True, True))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);

    assert_ne!(
        code, 101,
        "probeK_arity_tuple não deve panicar — stderr: {stderr}"
    );
    assert!(
        stderr.contains("type.arity_mismatch"),
        "probeK_arity_tuple deve dar ArityMismatch — stderr: {stderr}"
    );
}

// ── RED F1: falso-positivo de redundância (probeE2) ────────────

/// probeE2: cláusulas lambda com variant qualificado aninhado.
/// Hoje: falso-positivo `type.redundant_clause` (a 2ª cláusula Some False
/// é rejeitada como redundante). F2: verde, sem falso-positivo.
/// F1 interim: o erro de redundância é aceitável enquanto o motor
/// não migra, mas NÃO pode panicar.
#[test]
fn probe_e2_nao_panic_redundancia() {
    let path = write_temp_kata(
        "probeE2",
        r#"foo :: Optional::(Boolean) => Text
lambda Optional::Some True: "tem true"
lambda Optional::Some False: "tem false"
lambda Optional::None: "nada"

action main
    echo!(foo (Some True))
    echo!(foo (Some False))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);

    // Hoje é exit 1 com redundant_clause (falso-positivo). F2: verde.
    // F1: não pode panicar (101).
    assert_ne!(code, 101, "probeE2 não deve panicar — stderr: {stderr}");
}

// ── RED F2: verdes por cegueira (oráculos #[ignore] até F2) ────

/// probeA: Some(True) + None, SEM Some(False) — checker aceita por cegueira.
/// F2: NonExhaustiveMatch missing ["Some False"].
#[test]
#[ignore = "F2: motor Maranget — NonExhaustiveMatch [\"Some False\"]"]
fn probe_a_non_exhaustive() {
    let path = write_temp_kata(
        "probeA",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        None: "nada"

action main
    echo!(foo (Some True))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeA deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert!(
        stderr.contains("Some False"),
        "witness deve ser Some False — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeB: chama foo com Some(False) — caso não coberto.
/// Hoje: SIGILL (exit -4/132). F2: NonExhaustiveMatch em compile-time.
#[test]
#[ignore = "F2: motor Maranget — fim do SIGILL, NonExhaustiveMatch em compile"]
fn probe_b_non_exhaustive_compile() {
    let path = write_temp_kata(
        "probeB",
        r#"foo :: Optional::(Boolean) => Text
lambda m:
    match m
        Some True: "tem true"
        None: "nada"

action main
    echo!(foo (Some False))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    // F2: deve falhar em compile-time com NonExhaustiveMatch, NÃO SIGILL.
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeB deve dar NonExhaustiveMatch em compile — stderr: {stderr}"
    );
    assert_ne!(code, -4, "probeB não deve mais dar SIGILL");
}

/// probeM: match parcial sobre Result::(Int, Text) — Ok 0 + Err _.
/// F2: NonExhaustiveMatch missing ["Ok _"].
#[test]
#[ignore = "F2: motor Maranget — NonExhaustiveMatch [\"Ok _\"]"]
fn probe_m_non_exhaustive() {
    let path = write_temp_kata(
        "probeM",
        r#"foo :: Result::(Int, Text) => Text
lambda m:
    match m
        Ok 0: "zero"
        Err _: "erro"

action main
    echo!(foo (Ok 0))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeM deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeK_deep_hole: 3 níveis com buraco — Some (Some False) removida.
/// F2: NonExhaustiveMatch com witness de 3 níveis ["Some (Some False)"].
#[test]
#[ignore = "F2: motor Maranget — witness 3 níveis [\"Some (Some False)\"]"]
fn probe_k_deep_hole_non_exhaustive() {
    let path = write_temp_kata(
        "probeK_deep_hole",
        r#"foo :: Optional::(Optional::(Boolean)) => Text
lambda m:
    match m
        Some Optional::Some True: "true dentro"
        Some Optional::None: "sem dentro"
        None: "nada"

action main
    echo!(foo (Some (Some True)))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeK_deep_hole deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

// ── RED F2: grade parcial (já RED hoje) ────────────────────────

/// probeK_grid_partial: grade 2×2 com 3 de 4 células.
/// Já RED hoje (non_exhaustive_match). F2: mesmo erro, com witness de matriz.
#[test]
fn probe_k_grid_partial_non_exhaustive() {
    let path = write_temp_kata(
        "probeK_grid_partial",
        r#"bar :: Boolean Boolean => Text
lambda True True: "vv"
lambda True False: "vf"
lambda False True: "fv"

action main
    echo!(bar True True)

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeK_grid_partial deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

// ── Parciais de fase: F3 (guards entre cláusulas) ──────────────

/// probeH: guards espalhados por cláusulas com o MESMO pattern.
/// Hoje: non_exhaustive_match (per-cláusula exige otherwise).
/// F3: verde com output correto.
#[test]
#[ignore = "F3: Z3 na folha — guards entre cláusulas sem otherwise"]
fn probe_h_guards_entre_clausulas() {
    let path = write_temp_kata(
        "probeH",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    > x 0: "positivo"
lambda Optional::Some x:
    <= x 0: "zero ou negativo"
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeH deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}

/// probeH_with: igual ao probeH, mas guards via `with`.
#[test]
#[ignore = "F3: Z3 na folha — guards via with entre cláusulas"]
fn probe_h_with_guards_entre_clausulas() {
    let path = write_temp_kata(
        "probeH_with",
        r#"foo :: Optional::(Int) => Text
lambda Optional::Some x:
    positivo: "positivo"
    with
        positivo := > x 0
lambda Optional::Some x:
    nao_positivo: "zero ou negativo"
    with
        nao_positivo := <= x 0
lambda Optional::None:
    "nada"

action main
    echo!(foo (Some 5))
    echo!(foo (Some (- 0 5)))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeH_with deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "positivo\nzero ou negativo\n");
}

// ── Parciais de fase: F4 (refined na folha) ───────────────────

/// probeF: match sobre refined com literais cobrindo o domínio {1, 2}.
/// Hoje: type.mismatch (literal não coerzido ao refined em pattern).
/// F4: verde com output correto.
#[test]
#[ignore = "F4: refined na folha — coerção de literal em pattern"]
fn probe_f_refined_folha() {
    let path = write_temp_kata(
        "probeF",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"

action main
    echo!(foo (1::UmOuDois))
    echo!(foo (2::UmOuDois))

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "probeF deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "um\ndois\n");
}

/// probeF_fora_dominio: literal 0 fora do domínio {1, 2}.
/// F4: TypeMismatch com o literal na mensagem.
#[test]
#[ignore = "F4: refined na folha — TypeMismatch com literal fora do domínio"]
fn probe_f_fora_dominio() {
    let path = write_temp_kata(
        "probeF_fora_dominio",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"
        2: "dois"
        0: "zero"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.mismatch"),
        "probeF_fora_dominio deve dar TypeMismatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

/// probeF_parcial: só `1:`, sem `2:` — buraco no domínio.
/// F4: NonExhaustiveMatch missing ["2"].
#[test]
#[ignore = "F4: refined na folha — NonExhaustiveMatch [\"2\"]"]
fn probe_f_parcial() {
    let path = write_temp_kata(
        "probeF_parcial",
        r#"data (Int, > _ 0, < _ 3) as UmOuDois

foo :: UmOuDois => Text
lambda n:
    match n
        1: "um"

action main
    echo!(foo (1::UmOuDois))

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.non_exhaustive_match"),
        "probeF_parcial deve dar NonExhaustiveMatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}

// ── F5: Rational na folha ──────────────────────────────────────

/// RatUmOuDois: match sobre Rational refined com `rational 1:` / `rational 2:`.
/// Hoje: type.unbound_name (rational não é pattern de folha).
/// F5: verde com output correto nos dois backends.
#[test]
#[ignore = "F5: Rational na folha — const-eval de rational <lit> + par (num, den) no Z3"]
fn rat_um_ou_dois_f5() {
    let path = write_temp_kata(
        "RatUmOuDois",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        rational 1: "um"
        rational 2: "dois"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")
    match (RatUmOuDois (rational 2))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (stdout, stderr, code) = run_kata_file(&path);
    assert_eq!(code, 0, "RatUmOuDois deve exit 0 — stderr: {stderr}");
    assert_eq!(stdout, "um\ndois\n");
}

/// RatUmOuDois_zero: literal 0 fora do domínio — F5: TypeMismatch.
#[test]
#[ignore = "F5: Rational na folha — TypeMismatch com literal fora do domínio"]
fn rat_um_ou_dois_zero_f5() {
    let path = write_temp_kata(
        "RatUmOuDois_zero",
        r#"data (Rational, > _ (rational 0), < _ (rational 3)) as RatUmOuDois

foo :: RatUmOuDois => Text
lambda n:
    match n
        rational 0: "zero"
        rational 1: "um"
        rational 2: "dois"

action main
    match (RatUmOuDois (rational 1))
        Ok v: echo!(foo v)
        Err _: echo!("erro")

main!()"#,
    );
    let (_stdout, stderr, code) = run_kata_file(&path);
    assert!(
        stderr.contains("type.mismatch"),
        "RatUmOuDois_zero deve dar TypeMismatch — stderr: {stderr}"
    );
    assert_ne!(code, 0);
}
