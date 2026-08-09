//! Aplicação de diretivas (hooks Enter/Exit/ShortCircuit) em bodies de Actions.

use kata_ast::{ActionStmt, Expr, MatchArm, Pattern, Span, Spanned};
use kata_resolution::{DirectiveDef, DirectiveRegistry, Hook, Target};

use super::reflection::{
    ReflectionInfo, action_stmts_to_exprs, directive_body_to_expr, synthesize_args_binding,
    synthesize_static_bindings,
};
use super::transform::transform_returns_in_expr;

// ── Aplicação de diretivas ───────────────────────────────────────────

/// Aplica diretivas customizadas ao body de uma Action.
/// `custom_names` em ordem: primeira = mais externa.
pub(super) fn apply_directives_to_action_body(
    body: Vec<ActionStmt>,
    custom_names: &[String],
    refl: &ReflectionInfo,
    registry: &DirectiveRegistry,
) -> Vec<ActionStmt> {
    // Processar de dentro para fora: última diretiva é a mais interna.
    // Aplicamos de trás para frente para que a primeira diretiva envolva todas.
    let mut current_body = body;

    for name in custom_names.iter().rev() {
        // Coletar todas as defs aplicáveis a este item (action).
        let defs: Vec<&DirectiveDef> = registry
            .lookup_by_name(name)
            .into_iter()
            .filter(|d| matches!(d.key.on, Target::Action | Target::Any))
            .collect();

        if defs.is_empty() {
            continue;
        }

        // Para cada hook presente, aplicar o inlining correspondente.
        // Múltiplos hooks da mesma diretiva = múltiplas injeções.
        for def in &defs {
            current_body = apply_hook_to_action_body(current_body, def, refl);
        }
    }

    current_body
}

/// Aplica um hook específico ao body de uma Action.
fn apply_hook_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    match def.key.when {
        Hook::Enter => apply_enter_to_action_body(body, def, refl),
        Hook::Exit => apply_exit_to_action_body(body, def, refl),
        Hook::ShortCircuit => apply_shortcircuit_to_action_body(body, def, refl),
        Hook::Transform => super::transform::apply_transform_to_action_body(body, def, refl),
    }
}

/// Enter: prependa bindings de reflexão + statements da diretiva antes do body.
fn apply_enter_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    let span = Span::synthetic();

    // Bindings estáticos
    let mut injected: Vec<ActionStmt> = synthesize_static_bindings(refl)
        .into_iter()
        .map(|e| ActionStmt {
            expr: e,
            has_semicolon: true,
        })
        .collect();

    // _args binding
    injected.push(ActionStmt {
        expr: synthesize_args_binding(refl),
        has_semicolon: true,
    });

    // Statements do body da diretiva
    for stmt in &def.body {
        injected.push(ActionStmt {
            expr: Spanned {
                node: stmt.expr.node.clone(),
                span: stmt.expr.span,
            },
            has_semicolon: stmt.has_semicolon,
        });
    }

    // Body original
    injected.extend(body);
    let _ = span;
    injected
}

/// Exit: envolve todos os pontos de saída com `let __result := ...; <bindings>;
/// <body da diretiva>; __result`.
fn apply_exit_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    if body.is_empty() {
        return body;
    }

    let total = body.len();
    let mut result = Vec::with_capacity(total + 16);

    for (i, stmt) in body.into_iter().enumerate() {
        let is_last = i == total - 1;
        if is_last && !stmt.has_semicolon {
            // Retorno implícito — envolver a expr.
            let wrapped = wrap_exit_expr(&stmt.expr, def, refl);
            result.push(ActionStmt {
                expr: wrapped,
                has_semicolon: false,
            });
        } else if let Expr::Return(inner) = &stmt.expr.node {
            // return expr — envolver.
            let wrapped = wrap_exit_expr(inner, def, refl);
            result.push(ActionStmt {
                expr: Spanned {
                    node: Expr::Return(Box::new(wrapped)),
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        } else {
            // Statement intermediário — pode conter return em sub-expressões.
            let transformed = transform_returns_in_expr(stmt.expr.node.clone(), &|e| {
                wrap_exit_expr(e, def, refl)
            });
            result.push(ActionStmt {
                expr: Spanned {
                    node: transformed,
                    span: stmt.expr.span,
                },
                has_semicolon: stmt.has_semicolon,
            });
        }
    }

    result
}

/// Envolve uma expressão de saída com `let __result := <expr>; <bindings>;
/// <body da diretiva>; __result` como um `Expr::Block`.
fn wrap_exit_expr(
    expr: &Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Spanned<Expr> {
    let span = Span::synthetic();

    let mut stmts = Vec::new();

    // let __result := <expr>
    stmts.push(Spanned {
        node: Expr::Let {
            name: "__result".into(),
            value: Box::new(Spanned {
                node: expr.node.clone(),
                span: expr.span,
            }),
        },
        span,
    });

    // Bindings estáticos
    stmts.extend(synthesize_static_bindings(refl));

    // _args binding
    stmts.push(synthesize_args_binding(refl));

    // _return binding
    stmts.push(super::reflection::synthesize_return_binding());

    // Statements do body da diretiva
    stmts.extend(action_stmts_to_exprs(&def.body));

    // __result como valor de retorno
    stmts.push(Spanned {
        node: Expr::Ident {
            name: "__result".into(),
        },
        span,
    });

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}

/// ShortCircuit: insere `let __decision := <body da diretiva>; match __decision
/// { Optional::Some(r): r, Optional::None: <body original> }` no início.
fn apply_shortcircuit_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
) -> Vec<ActionStmt> {
    let span = Span::synthetic();

    let mut injected: Vec<ActionStmt> = Vec::new();

    // Bindings estáticos
    for e in synthesize_static_bindings(refl) {
        injected.push(ActionStmt {
            expr: e,
            has_semicolon: true,
        });
    }

    // _args binding
    injected.push(ActionStmt {
        expr: synthesize_args_binding(refl),
        has_semicolon: true,
    });

    // let __decision := <body da diretiva>
    let decision_expr = directive_body_to_expr(&def.body);
    injected.push(ActionStmt {
        expr: Spanned {
            node: Expr::Let {
                name: "__decision".into(),
                value: Box::new(decision_expr),
            },
            span,
        },
        has_semicolon: true,
    });

    // match __decision { Optional::Some(r): r, Optional::None: <body original> }
    let body_exprs: Vec<Spanned<Expr>> = body
        .into_iter()
        .map(|s| Spanned {
            node: s.expr.node,
            span: s.expr.span,
        })
        .collect();

    let original_body = if body_exprs.len() == 1 {
        body_exprs
            .into_iter()
            .next()
            .expect("len()==1 garante next()=Some")
    } else {
        Spanned {
            node: Expr::Block { stmts: body_exprs },
            span,
        }
    };

    let match_expr = Spanned {
        node: Expr::Match {
            scrutinee: Box::new(Spanned {
                node: Expr::Ident {
                    name: "__decision".into(),
                },
                span,
            }),
            arms: vec![
                MatchArm {
                    pattern: Some(Spanned {
                        node: Pattern::Variant {
                            enum_name: "Optional".into(),
                            variant: "Some".into(),
                            payload: Some(vec![Spanned {
                                node: Pattern::Ident("r".into()),
                                span,
                            }]),
                        },
                        span,
                    }),
                    guard: None,
                    body: Spanned {
                        node: Expr::Ident { name: "r".into() },
                        span,
                    },
                },
                MatchArm {
                    pattern: Some(Spanned {
                        node: Pattern::Variant {
                            enum_name: "Optional".into(),
                            variant: "None".into(),
                            payload: None,
                        },
                        span,
                    }),
                    guard: None,
                    body: original_body,
                },
            ],
        },
        span,
    };

    injected.push(ActionStmt {
        expr: match_expr,
        has_semicolon: false,
    });

    injected
}
