//! Verificação de cláusulas redundantes (DoD 12).
//!
//! Análise de cobertura de patterns — verifica se uma cláusula é
//! inalcançável porque uma cláusula anterior já cobre todos os
//! valores que ela casaria.

use kata_ast::{Expr, LambdaClause, Pattern, Spanned};
use kata_diagnostics::MiddleError;

use crate::infer::helpers::InferResult;

/// Verifica sobreposição de cláusulas (RedundantClause).
///
/// Para cada cláusula N (N > 0), se existe uma cláusula M < N cujos patterns
/// "cobrem" todos os valores que a cláusula N casaria, e a cláusula N não
/// tem guards (sem condição adicional que a diferenciaria), a cláusula N
/// é inalcançável → `RedundantClause`.
pub(crate) fn check_redundant_clauses(clauses: &[Spanned<LambdaClause>]) -> InferResult<()> {
    for (i, clause) in clauses.iter().enumerate().skip(1) {
        // Cláusulas com guards não são redundantes por pattern alone —
        // a condição do guard pode diferenciá-las.
        if !clause.node.guards.is_empty() {
            continue;
        }
        let clause_patterns: Vec<&Pattern> = clause.node.patterns.iter().map(|p| &p.node).collect();

        for prev in &clauses[..i] {
            let prev_patterns: Vec<&Pattern> = prev.node.patterns.iter().map(|p| &p.node).collect();

            // Cláusula anterior com guards não torna a posterior redundante
            // — o guard pode falhar e deixar a posterior alcançável.
            if !prev.node.guards.is_empty() {
                continue;
            }

            if patterns_cover(&prev_patterns, &clause_patterns) {
                return Err(MiddleError::RedundantClause {
                    span: clause.span.into(),
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
/// - `Ident(_)` cobre qualquer pattern (liga o nome)
/// - `Literal(x)` cobre `Literal(x)` (mesmo literal)
/// - `Variant(E, V)` cobre `Variant(E, V)` (mesma variante)
/// - `Tuple(as)` cobre `Tuple(bs)` se cada `a_i` cobre `b_i`
fn patterns_cover(covering: &[&Pattern], covered: &[&Pattern]) -> bool {
    if covering.len() != covered.len() {
        return false;
    }
    covering
        .iter()
        .zip(covered.iter())
        .all(|(c, d)| pattern_covers(c, d))
}

fn pattern_covers(covering: &Pattern, covered: &Pattern) -> bool {
    match (covering, covered) {
        (Pattern::Wildcard, _) => true,
        (Pattern::Ident(_), _) => true,
        // Compara literais pelo conteúdo da expr, não pelo span.
        (Pattern::Literal(a), Pattern::Literal(b)) => literal_eq(&a.node, &b.node),
        (
            Pattern::Variant {
                enum_name: e1,
                variant: v1,
                ..
            },
            Pattern::Variant {
                enum_name: e2,
                variant: v2,
                ..
            },
        ) => e1 == e2 && v1 == v2,
        (Pattern::Tuple(as_), Pattern::Tuple(bs)) => {
            as_.len() == bs.len()
                && as_
                    .iter()
                    .zip(bs.iter())
                    .all(|(a, b)| pattern_covers(&a.node, &b.node))
        }
        _ => false,
    }
}

/// Compara duas expressões literais por conteúdo (ignora span).
fn literal_eq(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::IntLit { text: t1 }, Expr::IntLit { text: t2 }) => t1 == t2,
        (Expr::FloatLit { text: t1 }, Expr::FloatLit { text: t2 }) => t1 == t2,
        (Expr::TextLit { text: t1 }, Expr::TextLit { text: t2 }) => t1 == t2,
        (Expr::Unit, Expr::Unit) => true,
        _ => false,
    }
}
