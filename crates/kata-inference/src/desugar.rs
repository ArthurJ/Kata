//! Desugar pass — elimina `Expr::Pipe` e `Expr::Hole` da AST antes do typeck.
//!
//! Pipeline: `desugar(expr)` → AST sem Pipe nem Hole → `infer_expr`.
//!
//! Duas fases independentes, executadas em sequência:
//!
//! 1. **`desugar_pipes`** — elimina todos `Expr::Pipe`. Para cada `lhs |> rhs`:
//!    - Se `rhs` é `Apply` com `Hole` em algum arg: substitui o **primeiro** Hole
//!      por `lhs`. Holes restantes permanecem para a fase 2.
//!    - Se `rhs` é `Apply` sem `Hole`: injeta `lhs` como primeiro argumento.
//!    - Se `rhs` é `Ident`: vira `Apply { callee: rhs, args: [lhs] }`.
//!    - Grouping é transparente — peel para encontrar o Apply/Ident interno.
//!
//! 2. **`desugar_holes`** — elimina todos `Expr::Hole`. Para cada `Apply` com
//!    `Hole` em args: gera `Expr::Lambda` com N parâmetros fresh (um por Hole),
//!    substitui cada Hole pelo `Ident` do parâmetro correspondente.
//!
//! Nomes fresh: `__hole_0`, `__hole_1`, … — prefixo `__` é reservado para o
//! compilador. Cada Lambda cria escopo próprio, então nomes重复 entre
//! Lambdas aninhadas não conflitam (shadowing lexical normal).

use kata_ast::{Expr, GuardClause, MatchArm, Pattern, Span, Spanned, TypeExpr, WithBinding};

/// Ponto de entrada do desugar. Elimina Pipe e Hole da AST.
///
/// Deve ser chamado antes de `infer_expr` em `infer_module`.
pub fn desugar(expr: &Spanned<Expr>) -> Spanned<Expr> {
    let no_pipes = desugar_pipes(expr);
    desugar_holes(&no_pipes)
}

// ── Fase 1: eliminar Pipe ──────────────────────────────────────────

fn desugar_pipes(expr: &Spanned<Expr>) -> Spanned<Expr> {
    match &expr.node {
        Expr::Pipe { lhs, rhs } => {
            let lhs_d = desugar_pipes(lhs);
            let rhs_d = desugar_pipes(rhs);
            apply_pipe(&lhs_d, &rhs_d, expr.span)
        }
        // Recursão para todos os outros variants
        Expr::Apply { callee, args } => Spanned::new(
            Expr::Apply {
                callee: Box::new(desugar_pipes(callee)),
                args: args.iter().map(desugar_pipes).collect(),
            },
            expr.span,
        ),
        Expr::Let { name, value } => Spanned::new(
            Expr::Let {
                name: name.clone(),
                value: Box::new(desugar_pipes(value)),
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
                body: Box::new(desugar_pipes(body)),
                guards: guards
                    .iter()
                    .map(|g| GuardClause {
                        condition: g.condition.as_ref().map(desugar_pipes),
                        body: desugar_pipes(&g.body),
                    })
                    .collect(),
                with_bindings: with_bindings
                    .iter()
                    .map(|w| WithBinding {
                        name: w.name.clone(),
                        value: desugar_pipes(&w.value),
                    })
                    .collect(),
            },
            expr.span,
        ),
        Expr::Match { scrutinee, arms } => Spanned::new(
            Expr::Match {
                scrutinee: Box::new(desugar_pipes(scrutinee)),
                arms: arms
                    .iter()
                    .map(|arm| MatchArm {
                        pattern: arm.pattern.clone(),
                        guard: arm.guard.as_ref().map(desugar_pipes),
                        body: desugar_pipes(&arm.body),
                    })
                    .collect(),
            },
            expr.span,
        ),
        Expr::TypeAscription { expr: inner, ty } => Spanned::new(
            Expr::TypeAscription {
                expr: Box::new(desugar_pipes(inner)),
                ty: ty.clone(),
            },
            expr.span,
        ),
        Expr::Grouping { inner } => Spanned::new(
            Expr::Grouping {
                inner: Box::new(desugar_pipes(inner)),
            },
            expr.span,
        ),
        Expr::Tuple { elements } => Spanned::new(
            Expr::Tuple {
                elements: elements.iter().map(desugar_pipes).collect(),
            },
            expr.span,
        ),
        // Terminais — sem pipes para eliminar
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::Ident { .. }
        | Expr::Unit
        | Expr::Hole
        | Expr::VariantQual { .. }
        | Expr::Break
        | Expr::Continue => expr.clone(),

        // ── Fio 3: novos nós — recursão nos filhos ──────────
        Expr::ActionCall { callee, args } => Spanned::new(
            Expr::ActionCall {
                callee: callee.clone(),
                args: Box::new(desugar_pipes(args)),
            },
            expr.span,
        ),
        Expr::Return(inner) => {
            Spanned::new(Expr::Return(Box::new(desugar_pipes(inner))), expr.span)
        }
        Expr::Loop { body } => Spanned::new(
            Expr::Loop {
                body: body.iter().map(desugar_pipes).collect(),
            },
            expr.span,
        ),
        Expr::Var { name, value } => Spanned::new(
            Expr::Var {
                name: name.clone(),
                value: Box::new(desugar_pipes(value)),
            },
            expr.span,
        ),
        Expr::Question(inner) => {
            Spanned::new(Expr::Question(Box::new(desugar_pipes(inner))), expr.span)
        }
        Expr::PipeFallback { lhs, rhs } => {
            // `|` fallback — não é Pipe (`|>`). Desugar do pipe não toca.
            // Mas os filhos podem conter pipes internos.
            let lhs_d = desugar_pipes(lhs);
            let rhs_d = desugar_pipes(rhs);
            Spanned::new(
                Expr::PipeFallback {
                    lhs: Box::new(lhs_d),
                    rhs: Box::new(rhs_d),
                },
                expr.span,
            )
        }
    }
}

/// Aplica semântica do pipe: `lhs |> rhs` → Apply com Hole preenchido ou
/// `lhs` injetado como primeiro argumento.
fn apply_pipe(lhs: &Spanned<Expr>, rhs: &Spanned<Expr>, span: Span) -> Spanned<Expr> {
    // Peel Grouping — é transparente, não afeta a semântica do pipe.
    let rhs_core = peel_grouping(rhs);
    match &rhs_core.node {
        Expr::Apply { callee, args } => {
            let has_hole = args.iter().any(|a| matches!(a.node, Expr::Hole));
            if has_hole {
                // Substitui o PRIMEIRO Hole por lhs. Holes restantes ficam
                // para desugar_holes processar.
                let mut replaced = false;
                let new_args: Vec<Spanned<Expr>> = args
                    .iter()
                    .map(|a| {
                        if !replaced && matches!(a.node, Expr::Hole) {
                            replaced = true;
                            lhs.clone()
                        } else {
                            a.clone()
                        }
                    })
                    .collect();
                Spanned::new(
                    Expr::Apply {
                        callee: callee.clone(),
                        args: new_args,
                    },
                    span,
                )
            } else {
                // Sem Hole: injeta lhs como primeiro argumento
                let mut new_args = vec![lhs.clone()];
                new_args.extend(args.iter().cloned());
                Spanned::new(
                    Expr::Apply {
                        callee: callee.clone(),
                        args: new_args,
                    },
                    span,
                )
            }
        }
        Expr::Ident { name } => Spanned::new(
            Expr::Apply {
                callee: Box::new(Spanned::new(
                    Expr::Ident { name: name.clone() },
                    rhs_core.span,
                )),
                args: vec![lhs.clone()],
            },
            span,
        ),
        _ => {
            // rhs é algo else (Lambda, Match, etc.) — injeta como primeiro arg.
            // O typeck fará dispatch do callee não-Ident na Fase 8+.
            Spanned::new(
                Expr::Apply {
                    callee: Box::new(rhs.clone()),
                    args: vec![lhs.clone()],
                },
                span,
            )
        }
    }
}

/// Remove camadas de `Expr::Grouping` — retorna a expressão interna.
fn peel_grouping(expr: &Spanned<Expr>) -> &Spanned<Expr> {
    match &expr.node {
        Expr::Grouping { inner } => peel_grouping(inner),
        _ => expr,
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

// ── Fase 2: eliminar Hole ──────────────────────────────────────────

fn desugar_holes(expr: &Spanned<Expr>) -> Spanned<Expr> {
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

        // ── Fio 3: novos nós — recursão nos filhos ──────────
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
    }
}
