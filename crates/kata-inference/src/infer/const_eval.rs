//! Avaliação constante de predicados para ascription-refined.
//!
//! Avalia predicados de tipos refinados em compile-time para validar
//! ascription-refined (`5::PositiveInt`). NÃO é comptime — é
//! um avaliador minimal local ao typeck.
//!
//! Suporta: `Apply { Ident(op), [literal, literal] }` para `=`, `<`, `>`,
//! `<=`, `>=` com `IntLit` e `FloatLit`. `Hole` é substituído pelo valor
//! antes da avaliação. Retorna `None` se não consegue avaliar.

use kata_ast::{Expr, Spanned};

/// Avalia um predicado sobre um valor literal.
///
/// `pred` é o predicado com `Hole` como placeholder. `value` é a expressão
/// literal being ascribed (ex: `IntLit("5")`). Substitui `Hole` por `value`
/// e reduz a expressão booleana.
///
/// Retorna `Some(true)` se o predicado é satisfeito, `Some(false)` se falha,
/// `None` se o avaliador não consegue reduzir (predicado muito complexo).
pub(crate) fn const_eval_predicate(pred: &Spanned<Expr>, value: &Spanned<Expr>) -> Option<bool> {
    // Substitui Hole por value recursivamente.
    let substituted = substitute_hole(pred, value);
    eval_bool_expr(&substituted)
}

/// Substitui `Expr::Hole` por `value` recursivamente.
fn substitute_hole(expr: &Spanned<Expr>, value: &Spanned<Expr>) -> Spanned<Expr> {
    let new_node = match &expr.node {
        Expr::Hole => value.node.clone(),
        Expr::Apply { callee, args } => Expr::Apply {
            callee: Box::new(substitute_hole(callee, value)),
            args: args.iter().map(|a| substitute_hole(a, value)).collect(),
        },
        Expr::TypeAscription { expr: inner, ty } => Expr::TypeAscription {
            expr: Box::new(substitute_hole(inner, value)),
            ty: ty.clone(),
        },
        Expr::Grouping { inner } => Expr::Grouping {
            inner: Box::new(substitute_hole(inner, value)),
        },
        Expr::Tuple { elements } => Expr::Tuple {
            elements: elements.iter().map(|e| substitute_hole(e, value)).collect(),
        },
        other => other.clone(),
    };
    Spanned::new(new_node, expr.span)
}

/// Reduz uma expressão booleana para `Some(bool)` ou `None` se não consegue.
fn eval_bool_expr(expr: &Spanned<Expr>) -> Option<bool> {
    match &expr.node {
        Expr::Apply { callee, args } => {
            // Espera: Apply { Ident(op), [lit1, lit2] }
            let op = match &callee.node {
                Expr::Ident { name } => name.as_str(),
                _ => return None,
            };
            if args.len() != 2 {
                return None;
            }

            let left = eval_numeric(&args[0])?;
            let right = eval_numeric(&args[1])?;

            match op {
                "=" => Some(left == right),
                "!=" => Some(left != right),
                "<" => Some(left < right),
                ">" => Some(left > right),
                "<=" => Some(left <= right),
                ">=" => Some(left >= right),
                _ => None,
            }
        }
        // Boolean::True / Boolean::False como VariantQual
        Expr::VariantQual { enum_name, variant } if enum_name == "Boolean" => {
            match variant.as_str() {
                "True" => Some(true),
                "False" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extrai valor numérico (f64) de uma expressão literal.
///
/// IntLit e FloatLit são aceitos. Retorna `None` para outros tipos.
/// Usa f64 para统一 comparação entre Int e Float (suficiente para predicados
/// de ascription-refined).
fn eval_numeric(expr: &Spanned<Expr>) -> Option<f64> {
    match &expr.node {
        Expr::IntLit { text } => text.parse::<f64>().ok(),
        Expr::FloatLit { text } => text.parse::<f64>().ok(),
        // Suporta literais negativos: o parser pode produzir Apply { -, [IntLit] }
        // ou IntLit com texto negativo dependendo do contexto.
        Expr::Apply { callee, args } if args.len() == 1 => match &callee.node {
            Expr::Ident { name } if name == "-" => eval_numeric(&args[0]).map(|v| -v),
            _ => None,
        },
        _ => None,
    }
}
