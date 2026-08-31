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
use kata_core::caps::ConstVal;

use std::cmp::Ordering;

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
pub(crate) fn substitute_hole(expr: &Spanned<Expr>, value: &Spanned<Expr>) -> Spanned<Expr> {
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

            let left = eval_const(&args[0])?;
            let right = eval_const(&args[1])?;

            // Compara via ConstVal::cmp (cross-multiplication para Rat).
            // Para `=` e `!=`, usa PartialEq (estrutural). Para ordinais,
            // usa cmp().
            match op {
                "=" => Some(left == right),
                "!=" => Some(left != right),
                "<" | ">" | "<=" | ">=" => {
                    let ord = left.compare(&right)?;
                    let result = match op {
                        "<" => ord == Ordering::Less,
                        ">" => ord == Ordering::Greater,
                        "<=" => ord != Ordering::Greater,
                        ">=" => ord != Ordering::Less,
                        _ => unreachable!(),
                    };
                    Some(result)
                }
                _ => None,
            }
        }
        // Boolean::True / Boolean::False como VariantQual
        Expr::VariantQual {
            enum_name, variant, ..
        } if enum_name == "Boolean" => match variant.as_str() {
            "True" => Some(true),
            "False" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Extrai valor constante canônico (`ConstVal`) de uma expressão literal.
///
/// IntLit → `ConstVal::Int`, FloatLit → `ConstVal::Float`,
/// TextLit → `ConstVal::Text`, Unit → `ConstVal::Unit`,
/// `rational N` → `ConstVal::Rat(N, 1)`, Boolean True/False → `ConstVal::Bool`.
/// Suporta literais negativos: `Apply { -, [IntLit] }` → `ConstVal::Int(-N)`.
/// Retorna `None` para expressões não-literais.
pub(crate) fn eval_const(expr: &Spanned<Expr>) -> Option<ConstVal> {
    match &expr.node {
        Expr::IntLit { text } => Some(ConstVal::Int(text.parse::<i64>().ok()?)),
        Expr::FloatLit { text } => Some(ConstVal::Float(text.parse::<f64>().ok()?)),
        Expr::TextLit { text } => Some(ConstVal::Text(text.clone())),
        Expr::Unit => Some(ConstVal::Unit),
        // `rational N` → ConstVal::Rat(N, 1)
        Expr::Apply { callee, args }
            if args.len() == 1
                && matches!(&callee.node, Expr::Ident { name } if name == "rational") =>
        {
            let inner = eval_const(&args[0])?;
            match inner {
                ConstVal::Int(n) => Some(ConstVal::Rat(n, 1)),
                ConstVal::Float(f) => {
                    // Float → Rational: denom = 1, numer = f as i64 (truncado).
                    // Para precisão total seria necessário fração, mas para
                    // const-eval de predicados simples isto basta.
                    Some(ConstVal::Rat(f as i64, 1))
                }
                _ => None,
            }
        }
        // Suporta literais negativos: Apply { -, [IntLit] } ou Apply { -, [FloatLit] }
        Expr::Apply { callee, args } if args.len() == 1 => match &callee.node {
            Expr::Ident { name } if name == "-" => {
                let inner = eval_const(&args[0])?;
                match inner {
                    ConstVal::Int(v) => Some(ConstVal::Int(-v)),
                    ConstVal::Float(v) => Some(ConstVal::Float(-v)),
                    ConstVal::Rat(n, d) => Some(ConstVal::Rat(-n, d)),
                    _ => None,
                }
            }
            _ => None,
        },
        // Boolean::True / Boolean::False
        Expr::VariantQual {
            enum_name, variant, ..
        } if enum_name == "Boolean" => match variant.as_str() {
            "True" => Some(ConstVal::Bool(true)),
            "False" => Some(ConstVal::Bool(false)),
            _ => None,
        },
        // Grouping: desembrulha recursivamente.
        Expr::Grouping { inner } => eval_const(inner),
        _ => None,
    }
}
