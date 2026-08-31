//! Verificação de cláusulas redundantes (DoD 12).
//!
//! Análise de cobertura de patterns — verifica se uma cláusula é
//! inalcançável porque uma cláusula anterior já cobre todos os
//! valores que ela casaria.
//!
//! Opera sobre `TypedPattern` (pós-resolução do typeck) para distinguir
//! `Ident("True")` resolvido para `Variant { Boolean, True }` de um
//! binding `Ident { name: "x", ty: Int }`.
//!
//! ## Interação com guards
//!
//! Quando a cláusula anterior (M) ou posterior (N) tem guards, a
//! verificação considera os guards:
//!
//! | M guards | N guards | Decisão |
//! |----------|-----------|---------|
//! | não | não | `patterns_cover(M, N)` → redundante (caso original) |
//! | não | sim | M sempre dispara sobre os patterns → redundante |
//! | sim | não | Se guards de M são tautologia (Z3) → redundante |
//! | sim | sim | Se `guards_N ⟹ guards_M` (Z3) → redundante |
//!
//! Nos casos com Z3, se o solver não decide (Unknown), a verificação é
//! conservadora: assume não-redundante.

use kata_ast::Span;
use kata_core::EnumRegistry;
use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::guard_completeness::{check_guard_completeness, check_guard_implication};
use crate::infer::helpers::InferResult;
use crate::typed::TypedExpr;
use crate::typed_pattern::{TypedLambdaClause, TypedPattern};

/// Verifica sobreposição de cláusulas (RedundantClause).
///
/// Para cada cláusula N (N > 0), se existe uma cláusula M < N cujos
/// patterns "cobrem" todos os valores que a cláusula N casaria, e
/// os guards de M sempre disparam para esses valores, a cláusula N
/// é inalcançável → `RedundantClause`.
pub(crate) fn check_redundant_clauses(
    clauses: &[TypedLambdaClause],
    param_types: &[Ty],
    enum_registry: &EnumRegistry,
) -> InferResult<()> {
    // Coleta patterns de cada cláusula para o motor Maranget.
    let all_patterns: Vec<Vec<TypedPattern>> = clauses
        .iter()
        .map(|c| c.patterns.iter().map(|p| p.node.clone()).collect())
        .collect();

    for (i, clause_n) in clauses.iter().enumerate().skip(1) {
        let n_has_guards = !clause_n.guards.is_empty();

        // Passo 1 (estrutural): o motor Maranget decide se o braço i é
        // útil w.r.t. os braços anteriores. Se NÃO é útil (is_arm_redundant
        // retorna true), a cláusula é estruturalmente redundante — a menos
        // que guards de M não sejam tautologia (M pode falhar e N ser
        // alcançável).
        let structurally_redundant =
            crate::maranget::is_arm_redundant(&all_patterns, param_types, i, enum_registry);

        if !structurally_redundant {
            continue;
        }

        // Passo 2: decidir baseado em guards.
        // Se M (cláusula anterior que cobre N) não tem guards, ela sempre
        // dispara → N é redundante independente dos guards de N.
        // Se M tem guards, precisamos verificar tautologia/implicação.
        //
        // Como is_arm_redundant testa contra TODOS os braços anteriores,
        // precisamos encontrar QUAL braço M causa a redundância e se M
        // tem guards. Se algum M sem guards cobre N, N é redundante.
        for clause_m in &clauses[..i] {
            let m_has_guards = !clause_m.guards.is_empty();

            // Verifica se M cobre N estruturalmente.
            let m_patterns: Vec<&TypedPattern> =
                clause_m.patterns.iter().map(|p| &p.node).collect();
            let n_patterns: Vec<&TypedPattern> =
                clause_n.patterns.iter().map(|p| &p.node).collect();

            if !patterns_cover(&m_patterns, &n_patterns) {
                continue;
            }

            match (m_has_guards, n_has_guards) {
                (false, false) => {
                    return Err(MiddleError::RedundantClause {
                        span: clause_n.body.span.into(),
                        hint: Some(
                            "a cláusula anterior já cobre todos os patterns \
                             desta cláusula"
                                .to_string(),
                        ),
                    });
                }
                (false, true) => {
                    return Err(MiddleError::RedundantClause {
                        span: clause_n.body.span.into(),
                        hint: Some(
                            "a cláusula anterior cobre os mesmos patterns \
                             sem guards — sempre dispara antes desta cláusula"
                                .to_string(),
                        ),
                    });
                }
                (true, false) => {
                    let span = &clause_m.body.span;
                    if guard_is_tautology(&clause_m.guards, &clause_m.with_bindings, span) {
                        return Err(MiddleError::RedundantClause {
                            span: clause_n.body.span.into(),
                            hint: Some(
                                "a cláusula anterior cobre os mesmos patterns \
                                 e seus guards sempre disparam (são exaustivos)"
                                    .to_string(),
                            ),
                        });
                    }
                    // Guards de M não são tautologia — M pode falhar e N
                    // ser alcançável. Não é redundante.
                }
                (true, true) => {
                    let span = &clause_n.body.span;
                    if check_guard_implication(
                        &clause_n.guards,
                        &clause_m.guards,
                        &clause_n.with_bindings,
                        &clause_m.with_bindings,
                        span,
                    ) {
                        return Err(MiddleError::RedundantClause {
                            span: clause_n.body.span.into(),
                            hint: Some(
                                "qualquer input que satisfaça os guards desta \
                                 cláusula também satisfaz os guards da cláusula \
                                 anterior, que dispara primeiro"
                                    .to_string(),
                            ),
                        });
                    }
                    // Implicação não provada — N pode ser alcançável.
                }
            }
        }

        // Se o motor disse redundante mas nenhum M sem guards cobre N,
        // e nenhum M com guards é tautologia/implicação, então guards
        // de M podem falhar — N pode ser alcançável. Conservador: não reporta.
    }
    Ok(())
}

/// Verifica se os guards formam uma tautologia (sempre disparam).
///
/// Wrapper sobre `check_guard_completeness` que trata `Err(MissingOtherwise)`
/// (Z3 Unknown) como "não provado" — conservador, não reporta redundância.
/// `Err(NonExhaustiveMatch)` (Z3 SAT — contra-exemplo existe) também é
/// "não provado".
///
/// `with_bindings` são os bindings `with` da cláusula dona dos guards —
/// mesmos parâmetros de `check_guard_completeness`.
fn guard_is_tautology(
    guards: &[crate::typed_pattern::TypedGuardClause],
    with_bindings: &[crate::typed_pattern::TypedWithBinding],
    span: &Span,
) -> bool {
    match check_guard_completeness(guards, with_bindings, span) {
        Ok(()) => true,
        Err(_) => false,
    }
}

/// Verifica se `covering` patterns cobrem todos os valores que `covered` patterns casariam.
///
/// `covering` cobre `covered` se, para cada par (c, d), `c` cobre `d`:
/// - `Wildcard` cobre qualquer pattern
/// - `Ident { .. }` cobre qualquer pattern (liga o nome — é um binding, não variante)
/// - `Literal(a)` cobre `Literal(b)` se os valores são iguais
/// - `Variant(e, v)` cobre `Variant(e, v)` (mesma variante do mesmo enum)
/// - `Tuple(as)` cobre `Tuple(bs)` se cada `a_i` cobre `b_i`
fn patterns_cover(covering: &[&TypedPattern], covered: &[&TypedPattern]) -> bool {
    if covering.len() != covered.len() {
        return false;
    }
    covering
        .iter()
        .zip(covered.iter())
        .all(|(c, d)| pattern_covers(c, d))
}

fn pattern_covers(covering: &TypedPattern, covered: &TypedPattern) -> bool {
    match (covering, covered) {
        (TypedPattern::Wildcard, _) => true,
        (TypedPattern::Ident { .. }, _) => true,
        // Literais: compara pelo valor tipado.
        (TypedPattern::Literal { value: a }, TypedPattern::Literal { value: b }) => {
            typed_literal_eq(&a.node, &b.node)
        }
        (
            TypedPattern::Variant {
                enum_name: e1,
                variant: v1,
                ..
            },
            TypedPattern::Variant {
                enum_name: e2,
                variant: v2,
                ..
            },
        ) => e1 == e2 && v1 == v2,
        (TypedPattern::Tuple { elements: as_ }, TypedPattern::Tuple { elements: bs }) => {
            as_.len() == bs.len()
                && as_
                    .iter()
                    .zip(bs.iter())
                    .all(|(a, b)| pattern_covers(&a.node, &b.node))
        }
        (TypedPattern::Nil, TypedPattern::Nil) => true,
        _ => false,
    }
}

/// Compara dois `TypedExpr` literais por conteúdo.
fn typed_literal_eq(a: &TypedExpr, b: &TypedExpr) -> bool {
    use crate::typed::TypedExprKind;
    match (&a.kind, &b.kind) {
        (TypedExprKind::IntLit { text: t1 }, TypedExprKind::IntLit { text: t2 }) => t1 == t2,
        (TypedExprKind::FloatLit { text: t1 }, TypedExprKind::FloatLit { text: t2 }) => t1 == t2,
        (TypedExprKind::TextLit { text: t1 }, TypedExprKind::TextLit { text: t2 }) => t1 == t2,
        (TypedExprKind::Unit, TypedExprKind::Unit) => true,
        _ => false,
    }
}
