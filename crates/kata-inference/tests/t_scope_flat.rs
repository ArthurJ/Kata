//! Testes do modelo de escopo plano da action — débito 2 redesenhado.
//!
//! Responsabilidade: cravar o contrato de nomes no modelo novo:
//! - **Action tem escopo ÚNICO**: match/select/for/loop não abrem escopo.
//!   Bindings nascidos em braço/corpo **evaporam** no fim do construto.
//! - **`for` é `var`**: sobre `var` existente → reuso (o laço dirige o var,
//!   persiste pós-laço com o último elemento); sobre `let`/param →
//!   `DuplicateDecl`; sem nome prévio → loop-var fresco que evapora.
//! - **Reassign do loop-var no corpo é legal** — o laço reatribui
//!   `i := próximo` a cada iteração (semântica de var).
//! - **Binding de braço/corpo sobre externo de mesmo nome**: `var` →
//!   re-binding legal (tipo igual); `let`/param/`constant` → erro.
//! - **`constant` é sagrada**: `let`/`var`/`for`/pattern não podem
//!   redefinir/sombrear nome de constant (`DuplicateConstant`).
//! - **Lambdas continuam namespace próprio** (param sobre constant é legal).
//! - **Re-binding deve manter o tipo** do binding original (join sound).
//!
//! Bugs encontrados pelos probes P1–P26 (2026-08-29/30):
//! - P13/P14: `let`/`var` local sobre constant compilava (typeck cego) e o
//!   JIT imprimia o valor da constant (comptime substitui Ident por nome).
//! - P15: `for i` sobre `let i` compilava e vazava no JIT.
//! - P20: `for i` sobre `var i` — JIT `1 2 2` era o comportamento
//!   CORRETO do novo modelo (reuso); interp `1 2 99` é que estava errado.
//! - P21: reassign do loop-var no corpo era rejeitado como "imutável".
//! - P26: interp não via constants dentro de actions.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (padrão de t_var_rebinding_rules.rs) ─────────────────

fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);
    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);
    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry: {
            let mut ir = prelude.interface_registry.clone();
            ir.merge(user.interface_registry.clone());
            ir
        },
        refines_registry: {
            let mut rr = prelude.refines_registry.clone();
            rr.merge(user.refines_registry.clone());
            rr
        },
        type_graph: {
            let mut tg = prelude.type_graph.clone();
            tg.merge(&user.type_graph);
            tg
        },
        functions: {
            let mut fns = prelude.functions;
            let user_fn_names: std::collections::HashSet<&str> =
                user.functions.iter().map(|f| f.name.as_str()).collect();
            fns.retain(|f| !user_fn_names.contains(f.name.as_str()));
            fns.extend(user.functions);
            fns
        },
        actions: {
            let mut acts = prelude.actions;
            let user_action_names: std::collections::HashSet<&str> =
                user.actions.iter().map(|a| a.name.as_str()).collect();
            acts.retain(|a| !user_action_names.contains(a.name.as_str()));
            acts.extend(user.actions);
            acts
        },
        directive_registry: kata_resolution::DirectiveRegistry::new(),
    }
}

fn infer_ok(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn src_action(body: &str) -> String {
    format!("action main => Unit\n{body}\nmain!()")
}

// ── 1. constant é sagrada ────────────────────────────────────────

/// P13: `let x` local sobre `constant x` → DuplicateConstant.
#[test]
fn let_sobre_constant_rejeitado() {
    let err = infer_err(&format!(
        "constant x := 5\n\n{}",
        src_action("    let x := 3\n    echo!(x)")
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateConstant { ref name, .. } if name == "x"),
        "esperava DuplicateConstant para `x`, obtive: {err:?}"
    );
}

/// P14: `var x` local sobre `constant x` → DuplicateConstant.
#[test]
fn var_sobre_constant_rejeitado() {
    let err = infer_err(&format!(
        "constant x := 5\n\n{}",
        src_action("    var x := 0\n    echo!(x)")
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateConstant { ref name, .. } if name == "x"),
        "esperava DuplicateConstant para `x`, obtive: {err:?}"
    );
}

/// P22: `for x` sobre `constant x` → DuplicateConstant.
#[test]
fn for_sobre_constant_rejeitado() {
    let err = infer_err(&format!(
        "constant x := 5\n\n{}",
        src_action("    for x in [1..2]\n        echo!(x)")
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateConstant { ref name, .. } if name == "x"),
        "esperava DuplicateConstant para `x`, obtive: {err:?}"
    );
}

/// Pattern binding sobre constant (`Some x` com `constant x`) → DuplicateConstant.
#[test]
fn pattern_sobre_constant_rejeitado() {
    let err = infer_err(&format!(
        "constant x := 5\n\n{}",
        src_action("    match (Some 2)\n        Some x: echo!(x)\n        None: echo!(0)")
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateConstant { ref name, .. } if name == "x"),
        "esperava DuplicateConstant para `x`, obtive: {err:?}"
    );
}

/// P26: action lendo constant SEM shadow — continua legal.
#[test]
fn action_le_constant_ok() {
    infer_ok(&format!(
        "constant x := 5\n\n{}",
        src_action("    echo!(x)")
    ));
}

// ── 2. for é var — reuso / duplicate / fresco-evapora ───────────

/// P15: `for i` sobre `let i` → DuplicateDecl.
#[test]
fn for_sobre_let_rejeitado() {
    let err = infer_err(&src_action(
        "    let i := 99\n    for i in [1..3]\n        echo!(i)\n    echo!(i)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "i"),
        "esperava DuplicateDecl para `i`, obtive: {err:?}"
    );
}

/// Param de action: `for n` sobre param `n` → DuplicateDecl.
#[test]
fn for_sobre_param_rejeitado() {
    let err =
        infer_err("action main (n::Int) => Unit\n    for n in [1..3]\n        echo!(n)\nmain!(5)");
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "n"),
        "esperava DuplicateDecl para `n`, obtive: {err:?}"
    );
}

/// P20: `for i` sobre `var i` → REUSO legal. Pós-laço, `i` vale o
/// último elemento (o laço dirige o var externo).
#[test]
fn for_sobre_var_reuso() {
    infer_ok(&src_action(
        "    var i := 99\n    for i in [1..3]\n        echo!(i)\n    echo!(i)",
    ));
}

/// Loop-var fresco evapora: leitura pós-laço → UnboundName.
#[test]
fn loop_var_fresco_evapora() {
    let err = infer_err(&src_action(
        "    for i in [1..3]\n        echo!(i)\n    echo!(i)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::UnboundName { ref name, .. } if name == "i"),
        "esperava UnboundName para `i`, obtive: {err:?}"
    );
}

/// Reuso com tipo divergente: `var i` Float + `for i` sobre Ints →
/// TypeMismatch (o laço dirige o var; tipo do elemento deve casar).
#[test]
fn for_sobre_var_tipo_diferente_rejeitado() {
    let err = infer_err(&src_action(
        "    var i := 1.5\n    for i in [1..3]\n        echo!(i)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::TypeMismatch { .. }),
        "esperava TypeMismatch, obtive: {err:?}"
    );
}

/// P21: reassign do loop-var no corpo é legal — o laço reatribui
/// `i := próximo` a cada iteração (semântica de var).
#[test]
fn reassign_do_loop_var_no_corpo_ok() {
    infer_ok(&src_action(
        "    for i in [1..3]\n        i := 99\n        echo!(i)",
    ));
}

/// Reassign com tipo diferente do loop-var → TypeMismatch.
#[test]
fn reassign_do_loop_var_tipo_diferente_rejeitado() {
    let err = infer_err(&src_action(
        "    for i in [1..3]\n        i := \"texto\"\n        echo!(i)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::TypeMismatch { .. }),
        "esperava TypeMismatch, obtive: {err:?}"
    );
}

/// Loop-var fresco com reassign no corpo: evapora → leitura pós-laço
/// UnboundName (mesmo com reassign).
#[test]
fn loop_var_fresco_reassign_evapora() {
    let err = infer_err(&src_action(
        "    for i in [1..3]\n        i := 99\n        echo!(i)\n    echo!(i)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::UnboundName { ref name, .. } if name == "i"),
        "esperava UnboundName para `i`, obtive: {err:?}"
    );
}

/// Reuso de `for` sequencial (ranges.kata): 4 laços com mesmo loop-var
/// fresco — cada um evapora, sem colisão entre eles.
#[test]
fn for_sequencial_mesmo_nome_ok() {
    infer_ok(&src_action(
        "    for x in [1..2]\n        echo!(x)\n    for x in [3..4]\n        echo!(x)",
    ));
}

// ── 3. match/select/loop flat — bindings evaporam ───────────────

/// P3b: `let d` em braço sobre `let d` externo → DuplicateDecl
/// (braço é o mesmo namespace da action).
#[test]
fn let_em_braço_sobre_let_externo_rejeitado() {
    let err = infer_err(&src_action(
        "    let d := 5\n    match (> d 0)\n        Boolean::True:\n            let d := 7\n            echo!(d)\n        Boolean::False:\n            echo!(2)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "d"),
        "esperava DuplicateDecl para `d`, obtive: {err:?}"
    );
}

/// P7b: `var d` em braço sobre `let d` externo → DuplicateDecl.
#[test]
fn var_em_braço_sobre_let_externo_rejeitado() {
    let err = infer_err(&src_action(
        "    let d := 5\n    match (> d 0)\n        Boolean::True:\n            var d := 0\n            echo!(d)\n        Boolean::False:\n            echo!(d)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "d"),
        "esperava DuplicateDecl para `d`, obtive: {err:?}"
    );
}

/// P16: pattern binding (`Some v`) sobre `let v` externo → DuplicateDecl.
#[test]
fn pattern_sobre_let_externo_rejeitado() {
    let err = infer_err(&src_action(
        "    let v := 1\n    match (Some 2)\n        Some v: echo!(v)\n        None: echo!(0)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "v"),
        "esperava DuplicateDecl para `v`, obtive: {err:?}"
    );
}

/// P18: `var d` em braço sobre `var d` externo → re-binding legal
/// (tipo igual). No runtime o braço que rodar dirige o var externo.
#[test]
fn var_em_braço_sobre_var_externo_rebinding_ok() {
    infer_ok(&src_action(
        "    var d := 1\n    match (> d 0)\n        Boolean::True:\n            var d := 0\n            echo!(d)\n        Boolean::False:\n            echo!(2)",
    ));
}

/// P19: pattern binding (`Some v`) sobre `var v` externo → reuso legal
/// (o braço define o payload no var externo).
#[test]
fn pattern_sobre_var_externo_reuso_ok() {
    infer_ok(&src_action(
        "    var v := 1\n    match (Some 2)\n        Some v: echo!(v)\n        None: echo!(0)",
    ));
}

/// Binding nascido em braço evapora: leitura pós-match → UnboundName.
#[test]
fn binding_de_braço_evapora() {
    let err = infer_err(&src_action(
        "    match (> 1 0)\n        Boolean::True:\n            var e := 7\n            echo!(e)\n        Boolean::False:\n            var e := 8\n            echo!(e)\n    echo!(e)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::UnboundName { ref name, .. } if name == "e"),
        "esperava UnboundName para `e`, obtive: {err:?}"
    );
}

/// Braços com tipo divergente no re-binding do mesmo var → TypeMismatch
/// (join sound: o env pós-match deve ter um tipo só).
#[test]
fn rebinding_em_braço_tipo_divergente_rejeitado() {
    let err = infer_err(&src_action(
        "    var d := 1\n    match (> d 0)\n        Boolean::True:\n            var d := \"texto\"\n            echo!(d)\n        Boolean::False:\n            echo!(2)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::TypeMismatch { .. }),
        "esperava TypeMismatch, obtive: {err:?}"
    );
}

/// Loop body: binding nascido no corpo evapora; reassign de var
/// externo persiste (idioma correto).
#[test]
fn binding_de_loop_evapora_reassign_persiste() {
    infer_ok(&src_action(
        "    var cont := 0\n    loop\n        cont := + cont 1\n        match (> cont 5)\n            Boolean::True: break\n            Boolean::False: continue",
    ));
}

/// Binding nascido no corpo do loop, lido fora → UnboundName.
#[test]
fn binding_de_loop_lido_fora_rejeitado() {
    let err = infer_err(&src_action(
        "    loop\n        var x := 1\n        match (> x 1)\n            Boolean::True: break\n            Boolean::False: continue\n    echo!(x)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::UnboundName { ref name, .. } if name == "x"),
        "esperava UnboundName para `x`, obtive: {err:?}"
    );
}

/// Select: binding do braço (`valor`) evapora — leitura pós-select →
/// UnboundName. Uso dentro do braço ok (test_select.kata).
#[test]
fn binding_de_select_evapora() {
    let err = infer_err(&src_action(
        "    let (tx, rx) := channel!()\n    select\n        rx !> valor: echo!(1)\n        timeout 50: echo!(2)\n    echo!(valor)",
    ));
    assert!(
        matches!(err, kata_diagnostics::MiddleError::UnboundName { ref name, .. } if name == "valor"),
        "esperava UnboundName para `valor`, obtive: {err:?}"
    );
}

// ── 4. Namespace do lambda continua próprio ─────────────────────

/// P17: param de lambda sobre constant → legal (lambda é unidade de
/// abstração com namespace próprio).
#[test]
fn lambda_param_sobre_constant_ok() {
    infer_ok("constant x := 5\n\nf :: Int => Int\nlambda x: + x 1\n\necho!(f 1)");
}
