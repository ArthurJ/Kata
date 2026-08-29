//! Testes de seeding de let-bindings nas path conditions (Z3).
//!
//! Responsabilidade: quando uma ascription refined sobre não-literal
//! precisa provar o predicado (`try_prove_with_path_conditions`),
//! os bindings `let` do escopo em vigor são semeados no tradutor Z3 —
//! `let d := x` faz o predicado `> d 0` provar via `d = x` (aliasing
//! no var_cache, mais forte que asserir `d = x` como constraint).
//!
//! Regras sancionadas (sessão 2026-08-29):
//! - `let` é imutável e sembrado — identidade vale pela vida do
//!   binding (mesma justificativa dos learned_facts)
//! - `var` NÃO é sembrado (mutável, sem SSA — insound)
//! - bindings do corpo são trans-escopo; bindings nascidos DENTRO do
//!   braço morrem no rollback (braço é escopo fechado — probes
//!   P3b/P7b/P15/P16/P18/P19)
//! - seeding é LAST-WINS: binding mais recente do mesmo nome vence
//!   (shadowing aninhado let-let é legal — o interno sombreia)
//! - o gate de tentativa de prova continua em FACTS (bindings sem
//!   facts não viram `true ⟹ ¬pred` = refutação falsa)
//!
//! Fora do escopo (bugs pré-existentes, débito separado):
//! - refutação falsa de `var` com facts (P5) e facts insound sobre
//!   var reassignada (P4) — mecanismo de facts sobre mutáveis
//!
//! Repro original do gap: `let d := x` + fact `> x 0` + ascription
//! `d::PosInt` no braço refutava falsamente — o fact existe mas o Z3
//! não conectava `d` a `x`.

use kata_inference::infer_module;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::{load_stdlib_for_tests, resolve, ResolvedModule};

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

// ── Caso 1: repro — let em braço + fact prova via binding ──────

/// O repro original do gap: fact `> x 0` no braço True, binding
/// `let d := x`, ascription `d::PosInt` deve provar via `d = x`.
#[test]
fn let_em_braco_prova_via_binding() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
f :: Int => PosInt\n\
lambda x:\n\
\x20   match (> x 0)\n\
\x20   \x20   Boolean::True:\n\
\x20   \x20   \x20   let d := x\n\
\x20   \x20   \x20   d::PosInt\n\
\x20   \x20   Boolean::False: 1::PosInt\n\
\n\
action main\n\
\x20   echo!(f 5)\n\
main!()";
    let _tmod = infer_src(src);
}

/// Derivado: fact `> x 0`, binding com EXPRESSÃO `let d := * x 2`,
/// ascription prova via `x*2 > 0` — translate_int inlinado no
/// var_cache, mais forte que variável livre.
#[test]
fn let_derivado_prova_via_expressao() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
f :: Int => PosInt\n\
lambda x:\n\
\x20   match (> x 0)\n\
\x20   \x20   Boolean::True:\n\
\x20   \x20   \x20   let d := * x 2\n\
\x20   \x20   \x20   d::PosInt\n\
\x20   \x20   Boolean::False: 1::PosInt\n\
\n\
action main\n\
\x20   echo!(f 5)\n\
main!()";
    let _tmod = infer_src(src);
}

// ── Caso 2: trans-escopo — let do corpo atravessa braços ────────

/// Binding do corpo (antes dos matchs), fact no braço do match 2 —
/// o binding sobrevive ao rollback do match 1 (trans-escopo, como
/// learned_facts) e a prova usa `d = n ∧ n = 5 ⟹ d > 0`.
#[test]
fn let_do_corpo_transita_matchs() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let d := n\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True: echo!(1)\n\
\x20   \x20   Boolean::False: echo!(2)\n\
\x20   match (= n 5)\n\
\x20   \x20   Boolean::True: echo!(show (d::PosInt))\n\
\x20   \x20   Boolean::False: echo!(show (1::PosInt))\n\
main!(5)";
    let _tmod = infer_src(src);
}

// ── Caso 3: LAST-WINS — sombreamento aninhado usa binding novo ──

/// Corpo: `let d := - 0 n` (d = -n, NÃO prova com n > 0).
/// Braço: `let d := * n 2` (d = 2n, prova com n > 0) + ascription.
/// Seeding LAST-WINS: o binding interno vence → prova → OK.
/// First-wins (binding externo) ou sem seeding → refuta → erro.
/// Crava que o sombreamento aninhado legal (let-let em escopo filho)
/// usa o binding VISIBLE no ponto da ascription.
#[test]
fn let_sombreado_no_braco_usa_binding_novo() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let d := - 0 n\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True:\n\
\x20   \x20   \x20   let d := * n 2\n\
\x20   \x20   \x20   echo!(show (d::PosInt))\n\
\x20   \x20   Boolean::False: echo!(show (1::PosInt))\n\
main!(5)";
    let _tmod = infer_src(src);
}

// ── Caso 4: rollback — binding de braço morre no braço ──────────

/// Corpo: `let d := * n 2` (prova com n > 0). Match 1 braço True
/// sombreia com `let d := - 0 n` (refutante, mas SEM ascription no
/// braço — só declara). Match 2 braço True: ascription `d::PosInt`
/// com fact `> n 0` — deve provar via binding EXTERNO d = 2n.
/// Sem rollback, o binding refutante do match 1 venceria (last-wins)
/// e refutaria falsamente. Crava o truncamento por checkpoint.
#[test]
fn let_do_braco_morre_no_rollback() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let d := * n 2\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True:\n\
\x20   \x20   \x20   let d := - 0 n\n\
\x20   \x20   \x20   echo!(d)\n\
\x20   \x20   Boolean::False: echo!(2)\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True: echo!(show (d::PosInt))\n\
\x20   \x20   Boolean::False: echo!(2)\n\
main!(5)";
    let _tmod = infer_src(src);
}

// ── Caso 5: gate continua em FACTS, não em bindings ─────────────

/// Binding existe, fact NÃO. O gate de tentativa de prova continua
/// em facts: `is_empty` considera apenas facts/learned_facts. Sem
/// facts, o caminho é o de antes — erro "literal para ascription
/// refined (use construtor)". Se bindings entrassem no gate, a
/// conjunção vazia seria `true` e `true ⟹ ¬pred` refutaria ANY
/// predicado não-tautológico — falso negativo em massa.
#[test]
fn gate_sem_facts_binding_sozinho_nao_prova_nem_refuta() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let d := n\n\
\x20   echo!(show (d::PosInt))\n\
main!(5)";
    let err = infer_src_err(src);
    assert!(
        matches!(err, kata_diagnostics::MiddleError::TypeMismatch { .. }),
        "esperava TypeMismatch (literal para ascription refined), obtive: {err:?}"
    );
}

// ── Caso 6: binding não-traduzível não quebra a prova ───────────

/// Binding Text (`show n`) no corpo — não-traduzível pelo Z3, vira
/// variável livre inócua. A prova da ascription `n::PosInt` (fact
/// `> n 0`) NÃO pode ser afetada: nem crash, nem falso erro.
#[test]
fn binding_text_nao_quebra_prova_de_outro_binding() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let s := show n\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True: echo!(show (n::PosInt))\n\
\x20   \x20   Boolean::False: echo!(show (1::PosInt))\n\
main!(5)";
    let _tmod = infer_src(src);
}

// ── Caso 7: LetDestruct — sub-bindings sembrados ────────────────

/// `let (a, b) := (n, * n 2)` desugara em FieldAccess(temp, i) — os
/// sub-bindings a e b são imutáveis e sembrados como bindings normais.
/// Ascription `b::PosInt` prova via b = 2n.
#[test]
fn let_destruct_subbindings_sembrados() {
    let src = "\
data (Int, > _ 0) as PosInt\n\
\n\
action main (n::Int)\n\
\x20   let (a, b) := (n, * n 2)\n\
\x20   match (> n 0)\n\
\x20   \x20   Boolean::True: echo!(show (b::PosInt))\n\
\x20   \x20   Boolean::False: echo!(show (1::PosInt))\n\
main!(5)";
    let _tmod = infer_src(src);
}