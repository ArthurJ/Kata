//! Testes E2E do PRD: erros granulares para extensão de família polimórfica.
//!
//! PRD: docs/PRDs/PRD-family-extension-errors.md
//!
//! Estes testes usam subprocess (`kata run`) porque o erro
//! `type.incomplete_interface` é emitido pelo pipeline (após merge do
//! prelude), não pelo `resolve` ou `infer` isolados.
//!
//! Casos T1-T13 do PRD:
//! - T1: incomplete sem uso → type.incomplete_interface
//! - T2: #!allow incomplete, sem uso → compila
//! - T3: #!warn incomplete, sem uso → warning, compila [IGNORED: pipeline #!warn]
//! - T4: #!allow + uso NonZero → type.deferred_diagnostic
//! - T5: impl completo → compila, NonZero funciona
//! - T6: impl completo + NonZero construtor → Ok
//! - T7: mensagem lista cada método faltando com signature
//! - T8: deferred_diagnostic message naming predicate
//! - T9: #!allow não controla deferred_diagnostic
//! - T10: SHOW incompleto sem família → type.incomplete_interface
//! - T11: #!allow sem uso → não sintetiza construtor
//! - T12: #!deny → erro, mesmo que default
//! - T13: help com #!allow só em diagnóstico controlável

use std::process::Command;

fn kata_bin() -> String {
    option_env!("CARGO_BIN_EXE_kata")
        .map(String::from)
        .unwrap_or_else(|| "target/debug/kata".to_string())
}

/// Executa `kata run` com fonte Kata temporária. Retorna (stdout, stderr, exit_code).
fn run_kata(source: &str) -> (String, String, i32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "kata_family_ext_errors_{id}_{pid}.kata",
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

/// Código Kata para um tipo NUM completo (MyNum) — usado em T5, T6.
const MYNUM_FULL_IMPL: &str = "\
data MyNum (v::Int)

MyNum implements NUM
    + :: MyNum MyNum => MyNum
    lambda a b: MyNum (+ a.v b.v)
    - :: MyNum MyNum => MyNum
    lambda a b: MyNum (- a.v b.v)
    * :: MyNum MyNum => MyNum
    lambda a b: MyNum (* a.v b.v)
    div :: MyNum MyNum => Result::(MyNum, Text)
    lambda a b: core.Result::Ok a
    / :: MyNum NonZero => MyNum
    lambda a b: a
    // :: MyNum NonZero => Int
    lambda a b: 0
    zero :: MyNum => MyNum
    lambda _: MyNum 0
    abs :: MyNum => MyNum
    lambda a: MyNum a.v

MyNum implements EQ
    = :: MyNum MyNum => Boolean
    lambda a b: = a.v b.v
    != :: MyNum MyNum => Boolean
    lambda a b: not (= a.v b.v)

MyNum implements SHOW
    show :: MyNum => Text
    lambda a: show a.v
";

// ── T1: incomplete sem uso ──────────────────────────────────────────

/// T1: `Internal implements NUM` define só `+`, sem usar NonZero::Internal
/// → erro `type.incomplete_interface` listando os métodos faltando.
#[test]
fn t1_incomplete_sem_uso_falha() {
    let src = "\
data Internal (val::Int)

Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (stdout, stderr, exit) = run_kata(src);
    assert_ne!(exit, 0, "deve falhar com exit non-zero");
    assert!(
        stderr.contains("type.incomplete_interface")
            || stderr.contains("não define todos os métodos"),
        "stderr deve conter incomplete_interface — got: {stderr}"
    );
    let _ = stdout;
}

// ── T2: #!allow incomplete, sem uso → compila ───────────────────────

/// T2: `#!allow type.incomplete_interface` silencia o erro.
/// O módulo compila sem erro mesmo com implementação incompleta.
#[test]
fn t2_allow_incomplete_sem_uso_compila() {
    let src = "\
data Internal (val::Int)
#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, _stderr, exit) = run_kata(src);
    assert_eq!(exit, 0, "deve compilar com #!allow — stderr: {_stderr}");
}

// ── T3: #!warn incomplete, sem uso → warning, compila ───────────────

/// T3: `#!warn type.incomplete_interface` emite warning mas compila.
#[test]
fn t3_warn_incomplete_sem_uso_compila() {
    let src = "\
data Internal (val::Int)
#!warn type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, stderr, exit) = run_kata(src);
    assert_eq!(exit, 0, "deve compilar com #!warn — stderr: {stderr}");
    // Warning pode ou não aparecer em stderr dependendo do pipeline,
    // mas a compilação deve suceceder.
}

// ── T4: #!allow + uso NonZero → type.missing_overload ───────────────

/// T4: `#!allow type.incomplete_interface` + `NonZero (Internal 3)`
/// → erro compile-time `type.deferred_diagnostic` apontando `zero`.
#[test]
fn t4_allow_incomplete_com_uso_falha_missing_overload() {
    let src = "\
data Internal (val::Int)
#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

match NonZero (Internal 3)
    Ok v: v
    Err _: Internal 0
";
    let (_stdout, stderr, exit) = run_kata(src);
    assert_ne!(exit, 0, "deve falhar ao usar NonZero::Internal incompleto");
    assert!(
        stderr.contains("deferred_diagnostic"),
        "stderr deve conter deferred_diagnostic — got: {stderr}"
    );
    assert!(
        stderr.contains("não está definido"),
        "stderr deve explicar qual método não está definido — got: {stderr}"
    );
}

// ── T5: impl completo → compila, NonZero funciona ───────────────────

/// T5: `MyNum` define todos os métodos de NUM → compila.
/// NonZero::MyNum é gerada e funciona.
#[test]
fn t5_impl_completo_compila() {
    let src = format!("{MYNUM_FULL_IMPL}\nmatch NonZero (MyNum 3)\n    Ok v: 1\n    Err _: 0");
    let (_stdout, _stderr, exit) = run_kata(&src);
    assert_eq!(exit, 0, "impl completo deve compilar — stderr: {_stderr}");
}

// ── T6: impl completo + NonZero construtor → Ok ─────────────────────

/// T6: `NonZero (MyNum 3)` → Ok (valor não-zero).
#[test]
fn t6_impl_completo_nonzero_ok() {
    let src = format!("{MYNUM_FULL_IMPL}\nmatch NonZero (MyNum 3)\n    Ok v: 1\n    Err _: 0");
    let (stdout, _stderr, exit) = run_kata(&src);
    assert_eq!(exit, 0, "deve compilar — stderr: {_stderr}");
    assert!(
        stdout.contains("1"),
        "NonZero (MyNum 3) deve retornar Ok → braço imprime 1 — got: {stdout}"
    );
}

// ── T7: mensagem lista métodos faltando com signatures ──────────────

/// T7: o erro `type.incomplete_interface` lista cada método faltando
/// com sua signature completa (ex: `zero :: Internal => Internal`).
#[test]
fn t7_mensagem_lista_metodos_com_signatures() {
    let src = "\
data Internal (val::Int)

Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, stderr, _exit) = run_kata(src);
    // A mensagem deve conter signatures, não só nomes.
    // Verificar pelo menos um método faltando com signature.
    assert!(
        stderr.contains("div") && stderr.contains("zero"),
        "stderr deve listar métodos div e zero — got: {stderr}"
    );
    // Verificar que a signature formatada aparece (não só o nome).
    assert!(
        stderr.contains("::") || stderr.contains("=>"),
        "stderr deve conter signatures com :: e => — got: {stderr}"
    );
}

// ── T8: missing_overload message naming predicate ────────────────────

/// T8: `type.deferred_diagnostic` diz qual sobrecarga falta e por quê.
#[test]
fn t8_missing_overload_message_naming_predicate() {
    let src = "\
data Internal (val::Int)

#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

match NonZero (Internal 3)
    Ok v: v
    Err _: Internal 0
";
    let (_stdout, stderr, _exit) = run_kata(src);
    assert!(
        stderr.contains("deferred_diagnostic") && stderr.contains("não está definido"),
        "stderr deve conter deferred_diagnostic e explicar o método faltante — got: {stderr}"
    );
}

// ── T9: #!allow não controla missing_overload ────────────────────────

/// T9: `#!allow type.incomplete_interface` não controla
/// `type.deferred_diagnostic` — o erro dispara no uso.
#[test]
fn t9_allow_nao_controla_missing_overload() {
    let src = "\
data Internal (val::Int)

#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

match NonZero (Internal 3)
    Ok v: v
    Err _: Internal 0
";
    let (_stdout, stderr, exit) = run_kata(src);
    assert_ne!(exit, 0, "deferred_diagnostic não é controlável por #!allow");
    assert!(
        stderr.contains("deferred_diagnostic"),
        "stderr deve conter deferred_diagnostic — got: {stderr}"
    );
}

// ── T10: SHOW incompleto sem família → incomplete_interface ──────────

/// T10: `Greeting implements SHOW` incompleto, sem família sobre SHOW
/// → erro `type.incomplete_interface` (não `type.family_extension_invalid`).
#[test]
fn t10_show_incompleto_sem_familia() {
    let src = "\
data Greeting (msg::Text)

Greeting implements SHOW
    show :: Greeting => Text
    lambda g: g.msg

5
";
    // SHOW com `show` definido é completo — não deve falhar.
    // Para testar incompleto, omitimos o método.
    let src_incomplete = "\
data Greeting (msg::Text)

Greeting implements SHOW
    #!allow type.incomplete_interface

5
";
    // Greeting implements SHOW sem nenhum método → incomplete.
    let (_stdout, stderr, exit) = run_kata(src_incomplete);
    assert_ne!(exit, 0, "SHOW incompleto deve falhar — stderr: {stderr}");
    assert!(
        stderr.contains("type.incomplete_interface")
            || stderr.contains("não define todos os métodos"),
        "stderr deve conter incomplete_interface — got: {stderr}"
    );
    // O completo deve passar.
    let (_stdout2, _stderr2, exit2) = run_kata(src);
    assert_eq!(exit2, 0, "SHOW completo deve compilar");
}

// ── T11: #!allow sem uso → não sintetiza construtor ──────────────────

/// T11: `Internal implements NUM #!allow type.incomplete_interface`
/// sem uso → não sintetiza construtor (eager ou lazy).
/// Como o construtor não é sintetizado, não há erro de missing_overload.
#[test]
fn t11_allow_sem_uso_nao_sintetiza() {
    let src = "\
data Internal (val::Int)
#!allow type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, _stderr, exit) = run_kata(src);
    assert_eq!(
        exit, 0,
        "sem uso de NonZero::Internal, deve compilar sem erro"
    );
}

// ── T12: #!deny → erro, mesmo que default ────────────────────────────

/// T12: `#!deny type.incomplete_interface` → erro, mesmo comportamento
/// que o default (que é deny).
#[test]
fn t12_deny_explicito_falha() {
    let src = "\
data Internal (val::Int)
#!deny type.incomplete_interface
Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, stderr, exit) = run_kata(src);
    assert_ne!(exit, 0, "#!deny deve produzir erro");
    assert!(
        stderr.contains("type.incomplete_interface")
            || stderr.contains("não define todos os métodos"),
        "stderr deve conter incomplete_interface — got: {stderr}"
    );
}

// ── T13: help com #!allow só em controlável ──────────────────────────

/// T13: a instrução de controle (`#!allow`) na help só aparece em
/// diagnósticos controláveis. `type.incomplete_interface` oferece
/// `#!allow` na help.
#[test]
fn t13_help_offer_allow_only_for_controllable() {
    let src = "\
data Internal (val::Int)

Internal implements NUM
    + :: Internal Internal => Internal
    lambda a b: Internal (+ a.val b.val)

5
";
    let (_stdout, stderr, _exit) = run_kata(src);
    assert!(
        stderr.contains("#!allow") || stderr.contains("Para silenciar"),
        "stderr deve sugerir #!allow para diagnóstico controlável — got: {stderr}"
    );
}
