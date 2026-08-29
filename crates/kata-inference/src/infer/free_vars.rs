//! Coleta de free variables e pattern binds — extraído de captures.rs.
//!
//! `collect_free_vars` percorre a TAST coletando identificadores não-ligados
//! (free vars) contra um conjunto de bindings locais. `collect_pattern_binds`
//! coleta nomes ligados por patterns.

use std::collections::HashSet;

use kata_ast::Spanned;
use kata_core::dispatch::DispatchTable;

use crate::typed::{TypedExpr, TypedExprKind, TypedPattern};
use crate::typed_pattern::TypedLambdaClause;

use super::walk::for_each_subexpr;

/// Coleta free variables de uma expressão TAST contra um conjunto de
/// bindings locais. Uma free var é um `Ident` cujo nome não está em
/// `local_bindings`, não começa com `__` (compiler-generated), e **não é
/// função conhecida no DispatchTable** (funções globais são resolvidas em
/// compile-time via call direto, não são captures).
pub(crate) fn collect_free_vars(
    expr: &TypedExpr,
    local_bindings: &HashSet<String>,
    dispatch: &DispatchTable,
    out: &mut HashSet<String>,
) {
    for_each_subexpr(expr, &mut |e| {
        match &e.kind {
            TypedExprKind::Ident { name } => {
                if !local_bindings.contains(name)
                    && !name.starts_with("__")
                    && !dispatch.has_function(name)
                {
                    out.insert(name.clone());
                }
            }
            TypedExprKind::Closure { callee, .. } => {
                // As captures do callee (se Lambda) são free vars do
                // escopo atual. Closure não tem mais campo captures —
                // lê do Lambda interno.
                if let TypedExprKind::Lambda { captures, .. } = &callee.node.kind {
                    for cap in captures {
                        if !local_bindings.contains(&cap.name) {
                            out.insert(cap.name.clone());
                        }
                    }
                }
                // Deixa o visitor deser no callee e args normalmente.
            }
            TypedExprKind::Lambda { clauses, .. } => {
                // Lambda é boundary de escopo: os params do lambda são
                // locais ao corpo, não propagam para fora. Assumimos o
                // controle da descida.
                for clause in clauses {
                    collect_free_vars_lambda_clause(clause, local_bindings, dispatch, out);
                }
                return false; // não deixa o visitor descer — já fizemos
            }
            _ => {}
        }
        true
    });
}

/// Coleta nomes ligados por `let` em uma lista de exprs synthetic.
/// Usado para que bindings de `synthetic_pre`/`synthetic_post` (`_msg`, `_name`,
/// etc.) não sejam contados como free vars.
fn collect_let_bound_names(exprs: &[Spanned<TypedExpr>], out: &mut HashSet<String>) {
    for expr in exprs {
        if let TypedExprKind::Let { name, .. } = &expr.node.kind {
            out.insert(name.clone());
        }
    }
}

/// Coleta free vars de uma cláusula lambda, considerando que os pattern
/// binds e with bindings da cláusula são locais ao seu corpo.
///
/// `synthetic_pre`/`synthetic_post` são traversados com bindings expandidos
/// (inclui let-bound names do synthetic), mas o body/guards usam apenas os
/// bindings originais (synthetic não vaza para o body do usuário).
fn collect_free_vars_lambda_clause(
    clause: &TypedLambdaClause,
    local_bindings: &HashSet<String>,
    dispatch: &DispatchTable,
    out: &mut HashSet<String>,
) {
    let mut inner_locals = local_bindings.clone();
    collect_pattern_binds(&clause.patterns, &mut inner_locals);
    for wb in &clause.with_bindings {
        inner_locals.insert(wb.name.clone());
    }

    // Synthetic pre/post: let-bound names são locais ao escopo synthetic.
    // O body do usuário não vê esses bindings.
    let mut synthetic_locals = inner_locals.clone();
    collect_let_bound_names(&clause.synthetic_pre, &mut synthetic_locals);
    collect_let_bound_names(&clause.synthetic_post, &mut synthetic_locals);
    for expr in &clause.synthetic_pre {
        collect_free_vars(&expr.node, &synthetic_locals, dispatch, out);
    }
    for expr in &clause.synthetic_post {
        collect_free_vars(&expr.node, &synthetic_locals, dispatch, out);
    }

    // Body e guards: bindings originais (sem synthetic)
    if clause.guards.is_empty() {
        collect_free_vars(&clause.body.node, &inner_locals, dispatch, out);
    } else {
        for guard in &clause.guards {
            if let Some(cond) = &guard.condition {
                collect_free_vars(&cond.node, &inner_locals, dispatch, out);
            }
            collect_free_vars(&guard.body.node, &inner_locals, dispatch, out);
        }
    }
}

/// Coleta nomes ligados por patterns (Ident patterns).
pub(crate) fn collect_pattern_binds(patterns: &[Spanned<TypedPattern>], out: &mut HashSet<String>) {
    for pattern in patterns {
        collect_pattern_binds_one(&pattern.node, out);
    }
}

/// Coleta binds de um pattern recursivamente.
pub(crate) fn collect_pattern_binds_one(pattern: &TypedPattern, out: &mut HashSet<String>) {
    match pattern {
        TypedPattern::Ident { name, .. } => {
            out.insert(name.clone());
        }
        TypedPattern::Wildcard => {}
        TypedPattern::Literal { .. } => {}
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                collect_pattern_binds_one(&sub.node, out);
            }
        }
        TypedPattern::Variant { .. } => {}
        TypedPattern::Tuple { elements } => {
            for el in elements {
                collect_pattern_binds_one(&el.node, out);
            }
        }
        TypedPattern::Cons { head, tail } => {
            collect_pattern_binds_one(&head.node, out);
            collect_pattern_binds_one(&tail.node, out);
        }
        TypedPattern::Nil => {}
    }
}

/// Coleta binds de um pattern recursivamente, com seus tipos.
/// Usado por captures.rs para resolver tipos de captures que são
/// bindings locais da cláusula (não estão no type_env global).
pub(crate) fn collect_pattern_tys(
    patterns: &[Spanned<TypedPattern>],
    out: &mut std::collections::HashMap<String, kata_core::ty::Ty>,
) {
    for pattern in patterns {
        collect_pattern_tys_one(&pattern.node, out);
    }
}

/// Coleta binds + tipos de um pattern recursivamente.
fn collect_pattern_tys_one(
    pattern: &TypedPattern,
    out: &mut std::collections::HashMap<String, kata_core::ty::Ty>,
) {
    match pattern {
        TypedPattern::Ident { name, ty } => {
            out.insert(name.clone(), ty.clone());
        }
        TypedPattern::Wildcard => {}
        TypedPattern::Literal { .. } => {}
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            for sub in subs {
                collect_pattern_tys_one(&sub.node, out);
            }
        }
        TypedPattern::Variant { .. } => {}
        TypedPattern::Tuple { elements } => {
            for el in elements {
                collect_pattern_tys_one(&el.node, out);
            }
        }
        TypedPattern::Cons { head, tail } => {
            collect_pattern_tys_one(&head.node, out);
            collect_pattern_tys_one(&tail.node, out);
        }
        TypedPattern::Nil => {}
    }
}
