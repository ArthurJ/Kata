//! Facts de path conditions sobre bindings `var` — débito 1.
//!
//! Responsabilidade: cravar que o Z3 NUNCA usa bindings mutáveis:
//! - Facts que referenciam `var` são descartados na coleta (stale
//!   após reassign → provar com eles é insound, refutar é falso erro).
//! - Ascription não-literal sobre `var` é conservadoramente rejeitada
//!   ("use construtor") — nunca provada nem refutada via Z3.
//! - Provas legítimas sobre `let` continuam funcionando (regressão).
//!
//! Bugs provados pelos probes (2026-08-30):
//! - P4: `var d := n` + guard `> d 0` + `d := 0` no braço →
//!   `d::PosInt` aceita insound (d=0 imprime `PosInt()`).
//! - P28: `d := -1` + `d::NegInt` (correta!) refutada por fact stale.
//! - P29-mixed: fact de `let` legit + ascription sobre `var` →
//!   refutação falsa com var como variável livre no Z3.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (padrão de t_scope_flat.rs) ─────────────────────────

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

fn infer_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

fn infer_ok(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

// ── 1. Facts sobre var não são coletados ─────────────────────────

/// P4: guard `> d 0` sobre `var d` não vira fact — reassign `d := 0`
/// no braço invalidaria a prova. Sem o fact, ascription `d::PosInt`
/// cai no gate conservador ("use construtor") em vez de provar insound.
#[test]
fn fact_sobre_var_nao_e_coletado_p4() {
    let err = infer_err(
        "\
data (Int, > _ 0) as PosInt

action main (n::Int)
\x20   var d := n
\x20   match (> d 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   d := 0
\x20   \x20   \x20   echo!(show (d::PosInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (1::PosInt))
main!(5)",
    );
    // Gate conservador: literal esperado para ascription não-literal
    // (fact do guard foi descartado — Z3 não tem material sobre d).
    assert!(
        err.to_string().contains("construtor") || err.to_string().contains("literal"),
        "esperava erro de ascription conservador (use construtor), obtive: {err:?}"
    );
}

/// P28: fact stale `> d 0` sobre `var d` não pode refutar `d::NegInt`
/// após `d := -1` (ascription CORRETA em runtime). Descartado na
/// coleta → gate conservador, não refutação.
#[test]
fn fact_stale_sobre_var_nao_refuta_p28() {
    let err = infer_err(
        "\
data (Int, < _ 0) as NegInt

action main (n::Int)
\x20   var d := n
\x20   match (> d 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   d := -1
\x20   \x20   \x20   echo!(show (d::NegInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (-1::NegInt))
main!(5)",
    );
    assert!(
        err.to_string().contains("construtor") || err.to_string().contains("literal"),
        "esperava erro de ascription conservador (use construtor), obtive: {err:?}"
    );
}

/// P29-mixed: fact legítimo de `let` NÃO habilita provar ascription
/// sobre `var` — o var não é semeado e vira variável livre, podendo
/// tanto "provar" quanto "refutar" com valores arbitrários. Gate
/// conservador vale mesmo com facts de let no escopo.
#[test]
fn fact_de_let_nao_prova_ascription_sobre_var_p29() {
    let err = infer_err(
        "\
data (Int, > _ 0) as PosInt

action main (n::Int)
\x20   var d := n
\x20   let m := * n 2
\x20   match (> m 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   echo!(show (d::PosInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (1::PosInt))
main!(5)",
    );
    assert!(
        err.to_string().contains("construtor") || err.to_string().contains("literal"),
        "esperava erro de ascription conservador (use construtor), obtive: {err:?}"
    );
}

// ── 2. Regressão: provas legítimas sobre let continuam ───────────

/// P4 com `let` no lugar de `var`: o fact `> d 0` é legítimo (let é
/// imutável, única por escopo — o fact vale por todo o tempo de vida
/// do binding). A prova `d::PosInt` DEVE continuar aceita.
#[test]
fn fact_sobre_let_continua_provando_p4_let() {
    let _tmod = infer_ok(
        "\
data (Int, > _ 0) as PosInt

action main (n::Int)
\x20   let d := n
\x20   match (> d 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   echo!(show (d::PosInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (1::PosInt))
main!(5)",
    );
}

/// P28 com `let` no lugar de `var`: o fact `> d 0` legítimo REFUTA
/// `d::NegInt` corretamente (d = n = 5 > 0 — ascription realmente
/// errada). A refutação deve continuar.
#[test]
fn fact_sobre_let_continua_refutando_p28_let() {
    let err = infer_err(
        "\
data (Int, < _ 0) as NegInt

action main (n::Int)
\x20   let d := n
\x20   match (> d 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   echo!(show (d::NegInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (-1::NegInt))
main!(5)",
    );
    assert!(
        err.to_string().contains("refutado"),
        "esperava refutação legítima (let imutável), obtive: {err:?}"
    );
}

/// Seeding de let-binding: `let d := * n 2` + fact `> n 0` prova
/// `d::PosInt` via aliasing (Nível 4, 87487ef). Não pode regredir.
#[test]
fn seeding_de_let_continua_provando() {
    let _tmod = infer_ok(
        "\
data (Int, > _ 0) as PosInt

action main (n::Int)
\x20   let d := * n 2
\x20   match (> n 0)
\x20   \x20   Boolean::True:
\x20   \x20   \x20   echo!(show (d::PosInt))
\x20   \x20   Boolean::False:
\x20   \x20   \x20   echo!(show (1::PosInt))
main!(5)",
    );
}
