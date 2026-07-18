//! Desugar — elimina `Expr::Hole` da AST.
//!
//! Para cada `Apply` com `Hole` em args: gera `Expr::Lambda` com N parâmetros
//! fresh (um por Hole), substitui cada Hole pelo `Ident` do parâmetro
//! correspondente.
//!
//! Nomes fresh: `__hole_0`, `__hole_1`, … — prefixo `__` é reservado para o
//! compilador. Cada Lambda cria escopo próprio, então nomes repetidos entre
//! Lambdas aninhadas não conflitam (shadowing lexical normal).

use kata_ast::{
    Expr, GuardClause, MatchArm, Pattern, SelectArm, Span, Spanned, TypeExpr, WithBinding,
};

/// Desugar holes — elimina todos `Expr::Hole` da AST.
///
/// Deve ser chamado APÓS `desugar_pipes`.
pub(crate) fn desugar_holes(expr: &Spanned<Expr>) -> Spanned<Expr> {
    // Bottom-up: desugar children primeiro, depois processa o nó atual.
    match &expr.node {
        Expr::Apply { callee, args } => {
            let callee_d = desugar_holes(callee);
            let args_d: Vec<Spanned<Expr>> = args.iter().map(desugar_holes).collect();

            // Conta holes nos args (após desugar recursivo).
            // Um "hole" é tanto `Expr::Hole` puro quanto `TypeAscription { expr: Hole, ty }`
            // (DoD 28: `_::Int` em posição de argumento).
            let hole_positions: Vec<usize> = args_d
                .iter()
                .enumerate()
                .filter(|(_, a)| is_hole_or_ascribed_hole(&a.node))
                .map(|(i, _)| i)
                .collect();

            if hole_positions.is_empty() {
                Spanned::new(
                    Expr::Apply {
                        callee: Box::new(callee_d),
                        args: args_d,
                    },
                    expr.span,
                )
            } else {
                // Gera Lambda com um parâmetro fresh por Hole.
                // Se o hole tem ascription (_::Int), preserva a ascription no arg
                // substituído para que try_partial_dispatch use o tipo anotado.
                // Extrai ascriptions ANTES de mover args_d para new_args.
                let hole_ascriptions: Vec<Option<Spanned<TypeExpr>>> = hole_positions
                    .iter()
                    .map(|&pos| match &args_d[pos].node {
                        Expr::TypeAscription { ty, .. } => Some(ty.clone()),
                        Expr::Hole => None,
                        _ => unreachable!(
                            "is_hole_or_ascribed_hole garante que só Hole ou TypeAscription(Hole)"
                        ),
                    })
                    .collect();
                let mut new_args = args_d;
                let mut patterns: Vec<Spanned<Pattern>> = Vec::with_capacity(hole_positions.len());

                for (idx, &pos) in hole_positions.iter().enumerate() {
                    let name = format!("__hole_{idx}");
                    let ident = Spanned::new(Expr::Ident { name: name.clone() }, Span::synthetic());

                    new_args[pos] = match &hole_ascriptions[idx] {
                        Some(ty) => Spanned::new(
                            Expr::TypeAscription {
                                expr: Box::new(ident),
                                ty: ty.clone(),
                            },
                            new_args[pos].span,
                        ),
                        None => ident,
                    };
                    patterns.push(Spanned::new(Pattern::Ident(name), Span::synthetic()));
                }

                Spanned::new(
                    Expr::Lambda {
                        patterns,
                        body: Box::new(Spanned::new(
                            Expr::Apply {
                                callee: Box::new(callee_d),
                                args: new_args,
                            },
                            expr.span,
                        )),
                        guards: Vec::new(),
                        with_bindings: Vec::new(),
                    },
                    expr.span,
                )
            }
        }
        Expr::Let { name, value } => Spanned::new(
            Expr::Let {
                name: name.clone(),
                value: Box::new(desugar_holes(value)),
            },
            expr.span,
        ),
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => Spanned::new(
            Expr::Lambda {
                patterns: patterns.clone(),
                body: Box::new(desugar_holes(body)),
                guards: guards
                    .iter()
                    .map(|g| GuardClause {
                        condition: g.condition.as_ref().map(desugar_holes),
                        body: desugar_holes(&g.body),
                    })
                    .collect(),
                with_bindings: with_bindings
                    .iter()
                    .map(|w| WithBinding {
                        name: w.name.clone(),
                        value: desugar_holes(&w.value),
                    })
                    .collect(),
            },
            expr.span,
        ),
        Expr::Match { scrutinee, arms } => Spanned::new(
            Expr::Match {
                scrutinee: Box::new(desugar_holes(scrutinee)),
                arms: arms
                    .iter()
                    .map(|arm| MatchArm {
                        pattern: arm.pattern.clone(),
                        guard: arm.guard.as_ref().map(desugar_holes),
                        body: desugar_holes(&arm.body),
                    })
                    .collect(),
            },
            expr.span,
        ),
        Expr::TypeAscription { expr: inner, ty } => Spanned::new(
            Expr::TypeAscription {
                expr: Box::new(desugar_holes(inner)),
                ty: ty.clone(),
            },
            expr.span,
        ),
        Expr::Grouping { inner } => Spanned::new(
            Expr::Grouping {
                inner: Box::new(desugar_holes(inner)),
            },
            expr.span,
        ),
        Expr::Tuple { elements } => Spanned::new(
            Expr::Tuple {
                elements: elements.iter().map(desugar_holes).collect(),
            },
            expr.span,
        ),
        // Terminais — sem holes para eliminar
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::Ident { .. }
        | Expr::Unit
        | Expr::Hole
        | Expr::VariantQual { .. }
        | Expr::Break
        | Expr::Continue => expr.clone(),
        // Pipe não deve aparecer aqui (desugar_pipes roda primeiro)
        Expr::Pipe { .. } => expr.clone(),

        // ── Novos nós — recursão nos filhos ──────────
        Expr::ActionCall { callee, args } => Spanned::new(
            Expr::ActionCall {
                callee: callee.clone(),
                args: Box::new(desugar_holes(args)),
            },
            expr.span,
        ),
        Expr::Return(inner) => {
            Spanned::new(Expr::Return(Box::new(desugar_holes(inner))), expr.span)
        }
        Expr::Loop { body } => Spanned::new(
            Expr::Loop {
                body: body.iter().map(desugar_holes).collect(),
            },
            expr.span,
        ),
        Expr::Var { name, value } => Spanned::new(
            Expr::Var {
                name: name.clone(),
                value: Box::new(desugar_holes(value)),
            },
            expr.span,
        ),
        Expr::Reassign { name, value } => Spanned::new(
            Expr::Reassign {
                name: name.clone(),
                value: Box::new(desugar_holes(value)),
            },
            expr.span,
        ),
        Expr::Question(inner) => {
            Spanned::new(Expr::Question(Box::new(desugar_holes(inner))), expr.span)
        }
        Expr::PipeFallback { lhs, rhs } => Spanned::new(
            Expr::PipeFallback {
                lhs: Box::new(desugar_holes(lhs)),
                rhs: Box::new(desugar_holes(rhs)),
            },
            expr.span,
        ),
        Expr::DotAccess { expr: inner, index } => Spanned::new(
            Expr::DotAccess {
                expr: Box::new(desugar_holes(inner)),
                index: index.clone(),
            },
            expr.span,
        ),
        Expr::Spread => expr.clone(),
        // ── Coleções — recursão nos elementos ───────────
        Expr::ListLit { elements } => Spanned::new(
            Expr::ListLit {
                elements: elements.iter().map(desugar_holes).collect(),
            },
            expr.span,
        ),
        Expr::ArrayLit { elements } => Spanned::new(
            Expr::ArrayLit {
                elements: elements.iter().map(desugar_holes).collect(),
            },
            expr.span,
        ),
        Expr::RangeLit {
            start,
            step,
            end,
            inclusive,
        } => Spanned::new(
            Expr::RangeLit {
                start: Box::new(desugar_holes(start)),
                step: Box::new(desugar_holes(step)),
                end: Box::new(desugar_holes(end)),
                inclusive: *inclusive,
            },
            expr.span,
        ),
        // ── ForIn e In ───────────────────────────────
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => Spanned::new(
            Expr::ForIn {
                var_name: var_name.clone(),
                iterable: Box::new(desugar_holes(iterable)),
                body: body.iter().map(desugar_holes).collect(),
            },
            expr.span,
        ),
        Expr::In { item, collection } => Spanned::new(
            Expr::In {
                item: Box::new(desugar_holes(item)),
                collection: Box::new(desugar_holes(collection)),
            },
            expr.span,
        ),

        // ── Nós CSP não contêm holes, preservam estrutura ──
        Expr::ChannelSend { channel, value } => Spanned::new(
            Expr::ChannelSend {
                channel: Box::new(desugar_holes(channel)),
                value: Box::new(desugar_holes(value)),
            },
            expr.span,
        ),
        Expr::ChannelRecv { channel, bind_name } => Spanned::new(
            Expr::ChannelRecv {
                channel: Box::new(desugar_holes(channel)),
                bind_name: bind_name.clone(),
            },
            expr.span,
        ),
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            let arms: Vec<SelectArm> = arms
                .iter()
                .map(|arm| SelectArm {
                    channel: desugar_holes(&arm.channel),
                    bind_name: arm.bind_name.clone(),
                    body: desugar_holes(&arm.body),
                })
                .collect();
            Spanned::new(
                Expr::Select {
                    arms,
                    timeout_ms: timeout_ms.as_ref().map(|t| Box::new(desugar_holes(t))),
                    timeout_body: timeout_body.as_ref().map(|t| Box::new(desugar_holes(t))),
                },
                expr.span,
            )
        }
    }
}

/// Verifica se uma expressão é um hole puro ou um hole com ascription (`_::Int`).
fn is_hole_or_ascribed_hole(expr: &Expr) -> bool {
    match expr {
        Expr::Hole => true,
        Expr::TypeAscription { expr, .. } => matches!(expr.node, Expr::Hole),
        _ => false,
    }
}
