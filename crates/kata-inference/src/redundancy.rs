//! Verificação de cláusulas redundantes (DoD 12).
//!
//! Análise de cobertura de patterns — verifica se uma cláusula é
//! inalcançável porque uma cláusula anterior já cobre todos os
//! valores que ela casaria.
//!
//! Opera sobre `TypedPattern` (pós-resolução do typeck) para distinguir
//! `Ident("True")` resolvido para `Variant { Boolean, True }` de um
//! binding `Ident { name: "x", ty: Int }`. Antes, operava sobre `Pattern`
//! (AST não-tipada) e tratava todo `Ident` como wildcard — causando falso
//! positivo em multi-cláusula com variantes de enum (`lambda True True`
//! cobria `lambda True False`).

use kata_diagnostics::MiddleError;

use crate::infer::helpers::InferResult;
use crate::typed::TypedExpr;
use crate::typed_pattern::{TypedLambdaClause, TypedPattern};

/// Verifica sobreposição de cláusulas (RedundantClause).
///
/// Para cada cláusula N (N > 0), se existe uma cláusula M < N cujos patterns
/// "cobrem" todos os valores que a cláusula N casaria, e a cláusula N não
/// tem guards (sem condição adicional que a diferenciaria), a cláusula N
/// é inalcançável → `RedundantClause`.
pub(crate) fn check_redundant_clauses(clauses: &[TypedLambdaClause]) -> InferResult<()> {
    for (i, clause) in clauses.iter().enumerate().skip(1) {
        // Cláusulas com guards não são redundantes por pattern alone —
        // a condição do guard pode diferenciá-las.
        if !clause.guards.is_empty() {
            continue;
        }
        let clause_patterns: Vec<&TypedPattern> =
            clause.patterns.iter().map(|p| &p.node).collect();

        for prev in &clauses[..i] {
            let prev_patterns: Vec<&TypedPattern> =
                prev.patterns.iter().map(|p| &p.node).collect();

            // Cláusula anterior com guards não torna a posterior redundante
            // — o guard pode falhar e deixar a posterior alcançável.
            if !prev.guards.is_empty() {
                continue;
            }

            if patterns_cover(&prev_patterns, &clause_patterns) {
                return Err(MiddleError::RedundantClause {
                    span: clause.body.span.into(),
                });
            }
        }
    }
    Ok(())
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