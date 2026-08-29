//! Testes das regras de re-binding de `var` — o que `var` pode reusar.
//!
//! Responsabilidade: cravar o contrato de nomes no MESMO escopo:
//! - Action tem escopo ÚNICO (2026-08-30): braços de match e corpos
//!   de for/loop são o MESMO namespace da action — `var`/`let` de
//!   braço sobre binding externo é `DuplicateDecl` (o binding
//!   externo pertence ao mesmo escopo)
//! - `var` pode reusar nome de `var` existente no same-scope (re-binding
//!   explícito, idioma de loop)
//! - `var` NÃO pode reusar nome de imutável (`let`/param) no same-scope
//!   — `let` é único por escopo; mutável às escondidas destrói a
//!   garantia de imutabilidade
//!
//! Bugs encontrados pelos probes P1/P11 (2026-08-29):
//! - P1: `let d := n` … `var d := 0` no mesmo escopo compilava (bug)
//! - P11: `var` sobre param de action compilava (bug)
//!
//! Fix: check `is_locally_defined` no caminho `Expr::Var` de expr.rs,
//! análogo ao do caminho `Expr::Let`, com o mesmo erro `DuplicateDecl`.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_stdlib_for_tests, resolve};

// ── Helpers (duplicados de guard_completeness.rs para isolamento) ──

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
    ResolvedModule {
        type_env,
        signatures,
        internal_signatures: Vec::new(),
        enum_registry,
        struct_registry,
        refined_decls: Vec::new(),
        enum_pred_decls: Vec::new(),
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

fn infer_src(src: &str) -> kata_inference::TypedModule {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect("inferência deve succeed")
}

fn infer_src_err(src: &str) -> kata_diagnostics::MiddleError {
    let tokens = lex(src).unwrap();
    let module = parse(tokens).unwrap();
    let prelude = load_stdlib_for_tests().unwrap();
    let user = resolve(&module).unwrap();
    let resolved = merge_resolved(prelude, user);
    infer_module(&module, &resolved).expect_err("inferência deve falhar")
}

// ── RED: var sobre imutável (same-scope) → duplicate_decl ──────

/// P1: `let d` seguido de `var d` no mesmo escopo compilava e MUTAVA
/// o binding imutável às escondidas. Deve ser `type.duplicate_decl`.
#[test]
fn var_sobre_let_mesmo_escopo_rejeitado() {
    let src = "\
action main (n::Int)\n\
\x20   let d := n\n\
\x20   var d := 0\n\
\x20   echo!(d)\n\
main!(5)";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "d"),
        "esperava DuplicateDecl para `d`, obtive: {err:?}"
    );
}

/// P11: `var` sobre param de action (params vivem no mesmo escopo do
/// corpo — P12 prova: `let` sobre param já é duplicate_decl).
#[test]
fn var_sobre_param_rejeitado() {
    let src = "\
action main (n::Int)\n\
\x20   var n := 0\n\
\x20   echo!(n)\n\
main!(5)";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "n"),
        "esperava DuplicateDecl para `n`, obtive: {err:?}"
    );
}

/// `var` sobre `var` no mesmo escopo continua LEGAL — re-binding
/// explícito, idioma documentado de loop.
#[test]
fn var_sobre_var_mesmo_escopo_ok() {
    let src = "\
action main (n::Int)\n\
\x20   var d := 0\n\
\x20   var d := n\n\
\x20   echo!(d)\n\
main!(5)";
    let _tmod = infer_src(src);
}

/// `var` de braço sobre `let` externo → `DuplicateDecl` — action tem
/// escopo ÚNICO (2026-08-30): o braço de match é o MESMO namespace da
/// action; reusar nome de imutável no mesmo escopo é proibido.
/// (Flip 2026-08-30: era "escopo filho legal, sem leak" — probes
/// P3b/P7b/P16/P19; contrato invertido pelo modelo plano sancionado.)
#[test]
fn var_aninhado_sobre_externo_rejeitado() {
    let src = "\
action main (n::Int)\n\
\x20   let d := n\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True:\n\
\x20   \x20   \x20   var d := 0\n\
\x20   \x20   \x20   echo!(d)\n\
\x20   \x20   Boolean::False:\n\
\x20   \x20   \x20   echo!(2)\n\
\x20   echo!(d)\n\
main!(5)";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "d"),
        "esperava DuplicateDecl para `d`, obtive: {err:?}"
    );
}

/// `_`-prefixados continuam isentos (diretivas reutilizam nomes).
#[test]
fn var_underscore_prefix_isento() {
    let src = "\
action main\n\
\x20   var _acc := 0\n\
\x20   var _acc := 1\n\
\x20   echo!(_acc)\n\
main!()";
    let _tmod = infer_src(src);
}

// ── Regressão: regras existentes seguem valendo ─────────────────

/// P9: `let` após `var` no mesmo escopo → duplicate_decl (já valia).
#[test]
fn let_apos_var_rejeitado() {
    let src = "\
action main (n::Int)\n\
\x20   var d := 0\n\
\x20   let d := n\n\
\x20   echo!(d)\n\
main!(5)";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::DuplicateDecl { ref name, .. } if name == "d"),
        "esperava DuplicateDecl para `d`, obtive: {err:?}"
    );
}
