//! Transformação recursiva de returns e hook Transform.

use kata_ast::{ActionStmt, Expr, MatchArm, Span, Spanned};
use kata_resolution::{CustomDirectiveApp, DirectiveDef};

use super::reflection::{
    ReflectionInfo, action_stmts_to_exprs, synthesize_args_binding, synthesize_return_binding,
    synthesize_static_bindings,
};

/// Transform: envolve pontos de saída com `let __result := ...; <bindings>;
/// <body da diretiva (último statement = valor transformado)>`.
pub(super) fn apply_transform_to_action_body(
    body: Vec<ActionStmt>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
    app: &CustomDirectiveApp,
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
            let wrapped = wrap_transform_expr(&stmt.expr, def, refl, app);
            result.push(ActionStmt {
                expr: wrapped,
                has_semicolon: false,
            });
        } else if let Expr::Return(inner) = &stmt.expr.node {
            // return expr — envolver.
            let wrapped = wrap_transform_expr(inner, def, refl, app);
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
                wrap_transform_expr(e, def, refl, app)
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
/// <body da diretiva — último statement = valor transformado>` como um `Expr::Block`.
fn wrap_transform_expr(
    expr: &Spanned<Expr>,
    def: &DirectiveDef,
    refl: &ReflectionInfo,
    app: &CustomDirectiveApp,
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

    // Args do site de aplicação (let _msg := "..." etc.)
    stmts.extend(super::action_hooks::synthesize_site_arg_bindings(app));

    // _return binding
    stmts.push(synthesize_return_binding());

    // Statements do body da diretiva — o último é o valor transformado.
    stmts.extend(action_stmts_to_exprs(&def.body));

    Spanned {
        node: Expr::Block { stmts },
        span,
    }
}

// ── Transformação recursiva de returns ──────────────────────────────

/// Percorre uma `Expr` recursivamente, transformando todos os `Expr::Return`
/// encontrados. O closure recebe a expr interna do return e retorna a
/// expr envolvida.
pub(super) fn transform_returns_in_expr<F>(expr: Expr, wrap: &F) -> Expr
where
    F: Fn(&Spanned<Expr>) -> Spanned<Expr>,
{
    match expr {
        Expr::Return(inner) => {
            let wrapped = wrap(&inner);
            Expr::Return(Box::new(wrapped))
        }
        Expr::Let { name, value } => Expr::Let {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Var { name, value } => Expr::Var {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Reassign { name, value } => Expr::Reassign {
            name,
            value: Box::new(transform_spanned_expr(*value, wrap)),
        },
        Expr::Apply { callee, args } => Expr::Apply {
            callee: Box::new(transform_spanned_expr(*callee, wrap)),
            args: args
                .into_iter()
                .map(|a| transform_spanned_expr(a, wrap))
                .collect(),
        },
        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(transform_spanned_expr(*scrutinee, wrap)),
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern: arm.pattern,
                    guard: arm.guard.map(|g| transform_spanned_expr(g, wrap)),
                    body: transform_spanned_expr(arm.body, wrap),
                })
                .collect(),
        },
        Expr::Block { stmts } => Expr::Block {
            stmts: stmts
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        Expr::Loop { body } => Expr::Loop {
            body: body
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        Expr::ForIn {
            var_name,
            iterable,
            body,
        } => Expr::ForIn {
            var_name,
            iterable: Box::new(transform_spanned_expr(*iterable, wrap)),
            body: body
                .into_iter()
                .map(|s| transform_spanned_expr(s, wrap))
                .collect(),
        },
        // Nós que não contêm sub-expressões com return potencial.
        other => other,
    }
}

fn transform_spanned_expr<F>(expr: Spanned<Expr>, wrap: &F) -> Spanned<Expr>
where
    F: Fn(&Spanned<Expr>) -> Spanned<Expr>,
{
    Spanned {
        node: transform_returns_in_expr(expr.node, wrap),
        span: expr.span,
    }
}
