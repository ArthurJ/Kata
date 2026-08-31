//! Materialização de domínio finito de tipos refined.
//!
//! Extraído de `maranget.rs` (Passo 6, zeladoria). Responsabilidade única:
//! enumerar o domínio de um tipo refined cujos predicados definem um
//! intervalo finito (`> _ 0`, `< _ 3` → `{1, 2}`) ou pontos de igualdade
//! (`= _ N`), para o motor de exaustividade decidir cobertura sem
//! `otherwise`.
//!
//! Domain unificado (F5.2): `Vec<ConstVal>`. Para tipos discretos com `ord`
//! (Int, Rational, Bool), enumera o intervalo `[lo, hi]`. Para tipos com
//! `eq` mas sem `ord`, coleta pontos de predicados `= _ N`.
//!
//! Depende apenas de `kata_core::caps` (ConstVal/Repr/CapsIndex),
//! `kata_ast` (Expr) e `infer::const_eval` — não conhece a matriz de
//! patterns nem o algoritmo de usefulness.

use kata_ast::{Expr, Span, Spanned};
use kata_core::caps::{CapsIndex, ConstVal, Repr};
use kata_core::struct_registry::StructRegistry;
use kata_resolution::RefinedDeclInfo;

use crate::infer::const_eval::{const_eval_predicate, eval_const};

/// Operador de bound extraído de predicado refined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

/// Extrai (operador, valor) de um predicado de bound: `> _ 5`, `< _ 3`, etc.
///
/// O predicado é `Apply { Ident(op), [Hole, literal] }` ou
/// `Apply { Ident(op), [literal, Hole] }` (ordem pode variar).
/// Retorna `None` se não é um predicado de bound reconhecido.
pub(crate) fn extract_bound(expr: &Expr) -> Option<(BoundOp, ConstVal)> {
    let Expr::Apply { callee, args } = expr else {
        return None;
    };
    let Expr::Ident { name: op } = &callee.node else {
        return None;
    };
    if args.len() != 2 {
        return None;
    }
    // Um argumento é Hole, o outro é literal.
    let (bound_op, val): (BoundOp, ConstVal) = match (&args[0].node, &args[1].node) {
        (Expr::Hole, _) => {
            let v = eval_const(&args[1])?;
            let bop = match op.as_str() {
                ">" => BoundOp::Gt,
                "<" => BoundOp::Lt,
                ">=" => BoundOp::Ge,
                "<=" => BoundOp::Le,
                "=" => BoundOp::Eq,
                "!=" => BoundOp::Ne,
                _ => return None,
            };
            (bop, v)
        }
        (_, Expr::Hole) => {
            // Ordem invertida — o valor vem antes do Hole.
            // O parser produz `> _ N` (Hole primeiro), então este caso
            // não deveria ocorrer. Defensivo: inverter a comparação.
            let v = eval_const(&args[0])?;
            let bop = match op.as_str() {
                ">" => BoundOp::Lt,
                "<" => BoundOp::Gt,
                ">=" => BoundOp::Le,
                "<=" => BoundOp::Ge,
                "=" => BoundOp::Eq,
                "!=" => BoundOp::Ne,
                _ => return None,
            };
            (bop, v)
        }
        _ => return None,
    };
    Some((bound_op, val))
}

/// Tenta enumerar o domínio finito de um tipo refined.
///
/// Recebe os campos relevantes de `MarangetEnv` como parâmetros explícitos
/// (em vez de `&self`), para manter esta fronteira livre do motor.
///
/// Retorna `None` se o domínio é infinito ou não-enumerável.
pub(crate) fn enum_refined_domain(
    struct_registry: Option<&StructRegistry>,
    refined_decls: Option<&[RefinedDeclInfo]>,
    caps_index: Option<&CapsIndex>,
    name: &str,
) -> Option<Vec<ConstVal>> {
    let _struct_registry = struct_registry?;
    let refined_decls = refined_decls?;
    let caps_index = caps_index?;

    // Consulta Repr e TypeCaps via CapsIndex.
    let caps = caps_index.get(name);
    let repr = caps.repr.clone();

    // Só enumera tipos discretos com ord (Int, Rational, Bool) ou
    // tipos com eq (para coleta de pontos).
    // Opaque e Struct (sem ord nem eq) não são enumeráveis.
    if !caps.ord && !caps.eq {
        return None;
    }

    // Busca RefinedDeclInfo para os predicados.
    let refined_decl = refined_decls.iter().find(|rd| rd.name == name)?;
    if refined_decl.predicates.is_empty() {
        return None;
    }

    // Extrai bounds dos predicados.
    // Para tipos com ord: `> _ N` → lo, `< _ M` → hi, etc.
    // Para tipos com eq (sem ord): coleta pontos de `= _ N`.
    let mut lo: Option<ConstVal> = None;
    let mut hi: Option<ConstVal> = None;
    let mut eq_points: Vec<ConstVal> = Vec::new();

    for pred_expr in &refined_decl.predicates {
        let (op, val) = extract_bound(&pred_expr.node)?;
        match op {
            BoundOp::Gt => {
                if !caps.ord {
                    return None;
                }
                // lo = val + 1 (para Int); para outros tipos, usa cmp.
                lo = Some(raise_lo(lo, succ(&val, &repr)?, |a, b| a.compare(b)));
            }
            BoundOp::Ge => {
                if !caps.ord {
                    return None;
                }
                lo = Some(raise_lo(lo, val, |a, b| a.compare(b)));
            }
            BoundOp::Lt => {
                if !caps.ord {
                    return None;
                }
                hi = Some(lower_hi(hi, pred(&val, &repr)?, |a, b| a.compare(b)));
            }
            BoundOp::Le => {
                if !caps.ord {
                    return None;
                }
                hi = Some(lower_hi(hi, val, |a, b| a.compare(b)));
            }
            BoundOp::Eq => {
                // `= _ N` — se temos ord, seta lo=hi=val.
                // Se não temos ord (só eq), coleta como ponto.
                if caps.ord {
                    lo = Some(raise_lo(lo, val.clone(), |a, b| a.compare(b)));
                    hi = Some(lower_hi(hi, val, |a, b| a.compare(b)));
                } else {
                    eq_points.push(val);
                }
            }
            BoundOp::Ne => {
                // `!= _ N` — não afeta bounds, filtra depois.
            }
        }
    }

    // Caminho 1: tipo com ord — enumera intervalo.
    if caps.ord {
        let lo = lo?;
        let hi = hi?;

        // Verifica que hi >= lo.
        match lo.compare(&hi) {
            Some(std::cmp::Ordering::Greater) => return None,
            None => return None, // tipos incomparáveis
            _ => {}
        }

        // Domínio pequeno o suficiente para enumerar.
        // Para Int, usa a contagem discreta. Para outros, limita em 1000.
        let count = domain_count(&lo, &hi, &repr)?;
        if count > 1000 {
            return None;
        }

        // Enumera e filtra por const_eval_predicate.
        let mut domain = Vec::new();
        let mut cur = lo.clone();
        loop {
            let expr = const_val_to_expr(&cur, &repr);
            if refined_decl
                .predicates
                .iter()
                .all(|pred| const_eval_predicate(pred, &expr) == Some(true))
            {
                domain.push(cur.clone());
            }
            // Próximo valor.
            match next_in_domain(&cur, &hi, &repr) {
                Some(next) => cur = next,
                None => break,
            }
        }

        return Some(domain);
    }

    // Caminho 2: tipo com eq mas sem ord — coleta pontos.
    if caps.eq && !eq_points.is_empty() {
        // Filtra pontos por todos os predicados.
        let domain: Vec<ConstVal> = eq_points
            .into_iter()
            .filter(|val| {
                let expr = const_val_to_expr(val, &repr);
                refined_decl
                    .predicates
                    .iter()
                    .all(|pred| const_eval_predicate(pred, &expr) == Some(true))
            })
            .collect();
        return Some(domain);
    }

    None
}

/// Sucedor de um ConstVal discreto (Int: n+1, Rat com den=1: n+1).
fn succ(val: &ConstVal, repr: &Repr) -> Option<ConstVal> {
    match (repr, val) {
        (Repr::Int, ConstVal::Int(n)) => Some(ConstVal::Int(n + 1)),
        (Repr::Rational, ConstVal::Rat(n, d)) if *d == 1 => Some(ConstVal::Rat(n + 1, 1)),
        _ => None, // Bool, Rational com d≠1, Float, Text não têm sucessor discreto
    }
}

/// Predecessor de um ConstVal discreto.
fn pred(val: &ConstVal, repr: &Repr) -> Option<ConstVal> {
    match (repr, val) {
        (Repr::Int, ConstVal::Int(n)) => Some(ConstVal::Int(n - 1)),
        (Repr::Rational, ConstVal::Rat(n, d)) if *d == 1 => Some(ConstVal::Rat(n - 1, 1)),
        _ => None,
    }
}

/// Conta elementos no intervalo [lo, hi] para tipos discretos.
fn domain_count(lo: &ConstVal, hi: &ConstVal, repr: &Repr) -> Option<i64> {
    match (repr, lo, hi) {
        (Repr::Int, ConstVal::Int(l), ConstVal::Int(h)) => Some(h - l + 1),
        (Repr::Rational, ConstVal::Rat(l, 1), ConstVal::Rat(h, 1)) => Some(h - l + 1),
        _ => None, // Não-discreto: não enumera
    }
}

/// Próximo valor no domínio, ou None se chegou em hi.
fn next_in_domain(cur: &ConstVal, hi: &ConstVal, repr: &Repr) -> Option<ConstVal> {
    match (repr, cur, hi) {
        (Repr::Int, ConstVal::Int(c), ConstVal::Int(h)) => {
            if c >= h {
                None
            } else {
                Some(ConstVal::Int(c + 1))
            }
        }
        (Repr::Rational, ConstVal::Rat(c, 1), ConstVal::Rat(h, 1)) => {
            if c >= h {
                None
            } else {
                Some(ConstVal::Rat(c + 1, 1))
            }
        }
        _ => None,
    }
}

/// Converte ConstVal para Expr (para const_eval_predicate).
fn const_val_to_expr(val: &ConstVal, _repr: &Repr) -> Spanned<Expr> {
    match val {
        ConstVal::Int(n) => Spanned::new(
            Expr::IntLit {
                text: n.to_string(),
            },
            Span::zero(),
        ),
        ConstVal::Float(f) => Spanned::new(
            Expr::FloatLit {
                text: f.to_string(),
            },
            Span::zero(),
        ),
        ConstVal::Rat(n, _d) => Spanned::new(
            Expr::Apply {
                callee: Box::new(Spanned::new(
                    Expr::Ident {
                        name: "rational".to_string(),
                    },
                    Span::zero(),
                )),
                args: vec![Spanned::new(
                    Expr::IntLit {
                        text: n.to_string(),
                    },
                    Span::zero(),
                )],
            },
            Span::zero(),
        ),
        ConstVal::Bool(b) => Spanned::new(
            Expr::VariantQual {
                enum_name: "Boolean".to_string(),
                variant: if *b { "True" } else { "False" }.to_string(),
                module_path: None,
            },
            Span::zero(),
        ),
        ConstVal::Text(s) => Spanned::new(Expr::TextLit { text: s.clone() }, Span::zero()),
        ConstVal::Unit => Spanned::new(Expr::Unit, Span::zero()),
        ConstVal::Struct(_) => Spanned::new(Expr::Unit, Span::zero()), // TODO
    }
}

/// Raises lo: max(old_lo, candidate) via cmp.
fn raise_lo<F>(old: Option<ConstVal>, candidate: ConstVal, cmp_fn: F) -> ConstVal
where
    F: Fn(&ConstVal, &ConstVal) -> Option<std::cmp::Ordering>,
{
    match old {
        None => candidate,
        Some(old_val) => match cmp_fn(&old_val, &candidate) {
            Some(std::cmp::Ordering::Greater) => old_val,
            _ => candidate,
        },
    }
}

/// Lowers hi: min(old_hi, candidate) via cmp.
fn lower_hi<F>(old: Option<ConstVal>, candidate: ConstVal, cmp_fn: F) -> ConstVal
where
    F: Fn(&ConstVal, &ConstVal) -> Option<std::cmp::Ordering>,
{
    match old {
        None => candidate,
        Some(old_val) => match cmp_fn(&old_val, &candidate) {
            Some(std::cmp::Ordering::Less) => old_val,
            _ => candidate,
        },
    }
}
