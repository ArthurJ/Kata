//! Testes E2E da Fase 9 — codegen de funções nomeadas, lambdas, match, guards.
//!
//! Pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
//! Cada teste verifica o valor retornado pelo JIT.

use kata_codegen::jit_eval;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};

/// Executa o pipeline completo e retorna o valor bruto do JIT + tipo.
fn eval_src(src: &str) -> (i64, Ty) {
    let tokens = lex(src).expect("lex deve succeed");
    let module = parse(tokens).expect("parse deve succeed");
    let prelude = load_prelude().expect("prelude deve carregar");
    let user = resolve(&module).expect("resolve deve succeed");
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).expect("infer deve succeed");
    let typed = optimize(typed);
    let jit = jit_eval(&typed).expect("codegen+JIT deve succeed");
    (jit.raw, jit.ty)
}

/// Combina prelude + módulo do usuário (replica do driver).
fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);
    let type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    ResolvedModule {
        type_env,
        signatures,
        enum_registry: prelude.enum_registry,
        functions: user.functions,
    }
}

/// Decodifica um SMI (val << 1 | 1) de volta para i64.
fn untag_smi(raw: i64) -> i64 {
    raw >> 1
}

// ── Fatorial recursivo ──────────────────────────────────────────

/// `fat :: Int Int => Int` com 2 cláusulas.
/// `fat 5 1` deve retornar 120.
#[test]
fn fatorial_recursivo() {
    let src = "\
fat :: Int Int => Int\n\
\x20   lambda 0 acc: acc\n\
\x20   lambda n acc: fat (- n 1) (* n acc)\n\
fat 5 1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 120, "fat 5 1 deve ser 120");
}

// ── Guards ──────────────────────────────────────────────────────

/// `abs :: Int => Int` com guards `> x 0` e `otherwise`.
/// `abs (-5)` deve retornar 5.
#[test]
fn abs_com_guards() {
    let src = "\
abs :: Int => Int\n\
\x20   lambda x:\n\
\x20\x20\x20\x20\x20   > x 0: x\n\
\x20\x20\x20\x20\x20   otherwise: - 0 x\n\
abs (- 0 5)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 5, "abs (-5) deve ser 5");
}

// ── Match Boolean ───────────────────────────────────────────────

/// `match Boolean::True` com braços `True: 1` e `False: 0` → 1
#[test]
fn match_boolean_true() {
    let src = "\
match Boolean::True\n\
\x20   True: 1\n\
\x20   False: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1, "match True deve ser 1");
}

/// `match Boolean::False` com braços `True: 1` e `False: 0` → 0
#[test]
fn match_boolean_false() {
    let src = "\
match Boolean::False\n\
\x20   True: 1\n\
\x20   False: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0, "match False deve ser 0");
}

// ── Hole (partial dispatch) ─────────────────────────────────────

/// `let soma_dez := + 10 _` + `soma_dez 5` → 15
#[test]
fn hole_partial_dispatch() {
    let src = "let soma_dez := + 10 _\nsoma_dez 5";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 15, "soma_dez 5 deve ser 15");
}

// ── Pipe ────────────────────────────────────────────────────────

/// `5 |> + 1 _ |> * 2 _` → 12
#[test]
fn pipe_chain() {
    let src = "5 |> + 1 _ |> * 2 _";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 12, "5 |> +1 |> *2 deve ser 12");
}

// ── Lambda como valor ───────────────────────────────────────────

/// `let inc := lambda x: + x 1` + `let g := inc` + `g 41` → 42
#[test]
fn lambda_como_valor() {
    let src = "let inc := lambda x: + x 1\nlet g := inc\ng 41";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 42, "g 41 deve ser 42");
}

// ── With block ───────────────────────────────────────────────────

/// Função com `with doubled := * x 2` nos guards.
/// `double_or_zero 5` → 10 (doubled = 10, > 0, retorna doubled)
#[test]
fn with_block_em_guard() {
    let src = "\
double_or_zero :: Int => Int\n\
\x20   lambda x:\n\
\x20\x20\x20\x20\x20   > doubled 0: doubled\n\
\x20\x20\x20\x20\x20   otherwise: 0\n\
\x20\x20\x20\x20\x20   with\n\
\x20\x20\x20\x20\x20\x20\x20\x20\x20   doubled := * x 2\n\
double_or_zero 5";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 10, "double_or_zero 5 deve ser 10");
}

// ── DoD 21: Função nomeada atribuída a variável (call_indirect) ───

/// `let g := fat` carrega function pointer de função nomeada.
/// `g 5 1` → 120 via call_indirect.
#[test]
fn funcao_nomeada_como_valor() {
    let src = "\
fat :: Int Int => Int\n\
\x20   lambda 0 acc: acc\n\
\x20   lambda n acc: fat (- n 1) (* n acc)\n\
let g := fat\n\
g 5 1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        120,
        "g 5 1 deve ser 120 (fat via call_indirect)"
    );
}

// ── DoD 22: Tuple pattern em match e lambda ──────────────────────

/// `match (1, 2)` com braço `(a, b): a` + `otherwise: 0` → 1
#[test]
fn match_tuple_pattern_fst() {
    let src = "match (1, 2)\n   (a, b): a\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 1, "match (1,2) (a,b): a deve ser 1");
}

/// `match (1, 2)` com braço `(a, b): b` + `otherwise: 0` → 2
#[test]
fn match_tuple_pattern_snd() {
    let src = "match (1, 2)\n   (a, b): b\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 2, "match (1,2) (a,b): b deve ser 2");
}

/// `match (10, 20, 30)` com braço `(a, b, c): c` + `otherwise: 0` → 30
#[test]
fn match_tuple_pattern_third() {
    let src = "match (10, 20, 30)\n   (a, b, c): c\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(
        untag_smi(raw),
        30,
        "match (10,20,30) (a,b,c): c deve ser 30"
    );
}

/// Lambda com tuple pattern — **bloqueado pela inferência de tipos**.
///
/// `let fst := lambda (a, b): a` falha com `LambdaInferenceFail` porque o
/// typeck não consegue inferir os tipos de `a` e `b` sem contexto. A solução
/// seria uma assinatura com tipo tupla (`fst :: (Int, Int) => Int`), mas o
/// parser de type expressions não suporta vírgula em tipos — `TypeExpr::Tuple`
/// é trabalho de Fio 5.
///
/// O codegen de `TypedPattern::Tuple` em lambda ESTÁ implementado (via
/// `test_single_pattern`). O bloqueio é upstream (parser + inferência).
#[test]
#[ignore = "bloqueado pela inferência: TypeExpr::Tuple é Fio 5"]
fn lambda_tuple_pattern() {
    let src = "let fst := lambda (a, b): a\nfst (10, 20)";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 10, "fst (10,20) deve ser 10");
}

/// Tuple pattern com wildcard: `match (1, 2) (_, b): b` + `otherwise: 0` → 2
#[test]
fn match_tuple_pattern_wildcard() {
    let src = "match (1, 2)\n   (_, b): b\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 2, "match (1,2) (_,b): b deve ser 2");
}

/// Tuple pattern com literal: `match (1, 2)` com `(1, b): b` e `otherwise: 0`
/// → 2 (o primeiro braço encaixa porque o primeiro elemento é 1)
#[test]
fn match_tuple_pattern_with_literal() {
    let src = "match (1, 2)\n   (1, b): b\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 2, "match (1,2) (1,b): b deve ser 2");
}

/// Tuple pattern com literal que NÃO encaixa: `match (5, 2)` com `(1, b): b`
/// e `otherwise: 0` → 0 (o primeiro braço não encaixa, cai em otherwise)
#[test]
fn match_tuple_pattern_literal_miss() {
    let src = "match (5, 2)\n   (1, b): b\n   otherwise: 0";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    assert_eq!(untag_smi(raw), 0, "match (5,2) (1,b) não encaixa → 0");
}

// ── DoD 9: TCO — fatorial com profundidade alta ─────────────────

/// `fat 100000 1` deve executar sem stack overflow.
/// O Cranelift faz TCO quando o `call` está em tail position (call → return_).
/// O codegen emite `return_(body_val)` direto em cada cláusula, sem `jump`
/// intermediário. Se TCO não funcionar, este teste crasha com SIGSEGV.
///
/// Nota: o resultado é 0 porque 100000! mod 2^63 contém fatores de 2
/// suficientes para zerar todos os bits. O importante é não crashar.
#[test]
fn fatorial_tco_profundidade_alta() {
    let src = "\
fat :: Int Int => Int\n\
\x20   lambda 0 acc: acc\n\
\x20   lambda n acc: fat (- n 1) (* n acc)\n\
fat 100000 1";
    let (raw, ty) = eval_src(src);
    assert_eq!(ty, Ty::Prim(PrimTy::Int));
    // Não verificamos o valor — só que não houve stack overflow.
    let _ = untag_smi(raw);
}
