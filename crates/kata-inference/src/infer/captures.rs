//! Fase 12 — Coleta de captures (free variables) de closures.
//!
//! Percorre a TAST pós-inferência e, para cada `Closure` cujo callee é um
//! `Lambda`, coleta as free variables do body do lambda contra os bindings
//! locais (params + with bindings + pattern binds). Cada free var vira um
//! `CaptureInfo { name, ty, storage: Stack }`.
//!
//! Roda após `infer_module` construir a `TypedModule`, mutando in-place os
//! campos `captures` de cada `Closure`.

use std::collections::HashSet;

use kata_core::ty::TypeEnv;

use crate::typed::{
    CaptureInfo, TypedExpr, TypedExprKind, TypedMatchArm, TypedModule, TypedPattern,
};

/// Ponto de entrada — percorre toda a TAST e popula `captures` de cada Closure.
///
/// Percorre: pre_entry, entry, functions (clauses), actions (body).
/// Para cada `Closure` cujo callee é `Lambda`, coleta free vars do body.
pub fn run(typed_module: &mut TypedModule) {
    // Percorre pre_entry
    for expr in &mut typed_module.pre_entry {
        collect_captures_in_expr(&mut expr.node, &typed_module.type_env);
    }
    // Percorre entry
    collect_captures_in_expr(&mut typed_module.entry.node, &typed_module.type_env);

    // Percorre funções nomeadas
    for func in &mut typed_module.functions {
        for clause in &mut func.clauses {
            // Bindings locais da cláusula: params + with bindings
            let mut local_bindings = HashSet::new();
            collect_pattern_binds(&clause.patterns, &mut local_bindings);
            for wb in &clause.with_bindings {
                local_bindings.insert(wb.name.clone());
            }
            // Guard bodies e clause body podem ter captures
            for guard in &mut clause.guards {
                if let Some(cond) = &mut guard.condition {
                    collect_captures_in_expr(&mut cond.node, &typed_module.type_env);
                }
                collect_captures_in_expr(&mut guard.body.node, &typed_module.type_env);
            }
            if clause.guards.is_empty() {
                collect_captures_in_expr_with_locals(
                    &mut clause.body.node,
                    &local_bindings,
                    &typed_module.type_env,
                );
            } else {
                // Com guards, o body é Unit (placeholder) — captures vêm dos guards.
                // Mas percorremos mesmo assim para cobrir Closures aninhadas.
                collect_captures_in_expr_with_locals(
                    &mut clause.body.node,
                    &local_bindings,
                    &typed_module.type_env,
                );
            }
        }
    }

    // Percorre Actions
    for action in &mut typed_module.actions {
        for stmt in &mut action.body {
            collect_captures_in_expr(&mut stmt.node, &typed_module.type_env);
        }
    }
}

/// Percorre uma expressão TAST, populando captures de cada Closure cujo callee
/// é Lambda. Usa escopo vazio (sem locals) — captures são relativas ao escopo
/// onde a closure é definida, não ao escopo da expressão atual.
///
/// Na prática, as free vars do body do lambda são coletadas contra os bindings
/// locais do lambda (params + pattern binds + with bindings). Tudo que não é
/// local é free var (captura do escopo externo).
fn collect_captures_in_expr(expr: &mut TypedExpr, outer_env: &TypeEnv) {
    match &mut expr.kind {
        TypedExprKind::Closure { callee, args, .. } => {
            // Lowera args primeiro (captures aninhadas em args)
            for arg in args {
                collect_captures_in_expr(&mut arg.node, outer_env);
            }

            // Recursa no callee — se for Lambda, o braço Lambda abaixo
            // popula lambda.captures diretamente (single source of truth).
            collect_captures_in_expr(&mut callee.node, outer_env);
        }

        TypedExprKind::TypeAscription { expr, .. } => {
            collect_captures_in_expr(&mut expr.node, outer_env);
        }
        TypedExprKind::Grouping { inner } => {
            collect_captures_in_expr(&mut inner.node, outer_env);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                collect_captures_in_expr(&mut el.node, outer_env);
            }
        }
        TypedExprKind::Let { value, .. } | TypedExprKind::Var { value, .. } => {
            collect_captures_in_expr(&mut value.node, outer_env);
        }
        TypedExprKind::Reassign { value, .. } => {
            collect_captures_in_expr(&mut value.node, outer_env);
        }
        TypedExprKind::Return(inner) => {
            collect_captures_in_expr(&mut inner.node, outer_env);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_captures_in_expr(&mut scrutinee.node, outer_env);
            for arm in arms {
                collect_captures_in_arm(arm, outer_env);
            }
        }
        TypedExprKind::Lambda {
            clauses,
            captures: lambda_captures,
            ..
        } => {
            // Lambda fora de Closure (ex: `let f := lambda x: ...`).
            // Coleta free vars do body contra os bindings locais do lambda.
            let mut all_captures: Vec<CaptureInfo> = Vec::new();

            // Primeira passada: coleta free vars (imutável)
            for clause in clauses.iter() {
                let mut local_bindings = HashSet::new();
                collect_pattern_binds(&clause.patterns, &mut local_bindings);
                for wb in &clause.with_bindings {
                    local_bindings.insert(wb.name.clone());
                }

                let mut free_vars = HashSet::new();
                if clause.guards.is_empty() {
                    collect_free_vars(&clause.body.node, &local_bindings, &mut free_vars);
                } else {
                    for guard in &clause.guards {
                        if let Some(cond) = &guard.condition {
                            collect_free_vars(&cond.node, &local_bindings, &mut free_vars);
                        }
                        collect_free_vars(&guard.body.node, &local_bindings, &mut free_vars);
                    }
                }

                for name in &free_vars {
                    if name.starts_with("__") {
                        continue;
                    }
                    if let Some(ty) = outer_env.lookup(name)
                        && !all_captures.iter().any(|c| c.name == *name)
                    {
                        all_captures.push(CaptureInfo {
                            name: name.clone(),
                            ty: ty.clone(),
                        });
                    }
                }
            }

            all_captures.sort_by(|a, b| a.name.cmp(&b.name));
            *lambda_captures = all_captures;

            // Segunda passada: percorre bodies para Closures aninhadas (mutável)
            for clause in clauses.iter_mut() {
                for guard in &mut clause.guards {
                    if let Some(cond) = &mut guard.condition {
                        collect_captures_in_expr(&mut cond.node, outer_env);
                    }
                    collect_captures_in_expr(&mut guard.body.node, outer_env);
                }
                if clause.guards.is_empty() {
                    collect_captures_in_expr(&mut clause.body.node, outer_env);
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_captures_in_expr(&mut stmt.node, outer_env);
            }
        }
        TypedExprKind::ActionCall { args, .. } => {
            collect_captures_in_expr(&mut args.node, outer_env);
        }
        // Folhas sem sub-expressões
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Versão de collect_captures_in_expr que recebe local bindings do escopo
/// onde a expressão está (ex: body de função nomeada). Usado para percorrer
/// o body de funções nomeadas, onde o escopo já tem bindings de params.
fn collect_captures_in_expr_with_locals(
    expr: &mut TypedExpr,
    _local_bindings: &HashSet<String>,
    outer_env: &TypeEnv,
) {
    // Por enquanto, delega para collect_captures_in_expr.
    // As local_bindings são usadas por collect_free_vars dentro do lambda,
    // não aqui — aqui só percorremos a árvore para encontrar Closures.
    collect_captures_in_expr(expr, outer_env);
}

/// Coleta free variables de uma expressão TAST contra um conjunto de
/// bindings locais. Uma free var é um `Ident` cujo nome não está em
/// `local_bindings` e não começa com `__` (compiler-generated).
fn collect_free_vars(
    expr: &TypedExpr,
    local_bindings: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    match &expr.kind {
        TypedExprKind::Ident { name } => {
            if !local_bindings.contains(name) && !name.starts_with("__") {
                out.insert(name.clone());
            }
        }
        TypedExprKind::Closure { callee, args, .. } => {
            // As captures do callee (se Lambda) são free vars do escopo atual.
            // Closure não tem mais campo captures — lê do Lambda interno.
            if let TypedExprKind::Lambda { captures, .. } = &callee.node.kind {
                for cap in captures {
                    if !local_bindings.contains(&cap.name) {
                        out.insert(cap.name.clone());
                    }
                }
            }
            collect_free_vars(&callee.node, local_bindings, out);
            for arg in args {
                collect_free_vars(&arg.node, local_bindings, out);
            }
        }
        TypedExprKind::TypeAscription { expr, .. } => {
            collect_free_vars(&expr.node, local_bindings, out);
        }
        TypedExprKind::Grouping { inner } => {
            collect_free_vars(&inner.node, local_bindings, out);
        }
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                collect_free_vars(&el.node, local_bindings, out);
            }
        }
        TypedExprKind::Let { name: _, value } | TypedExprKind::Var { name: _, value } => {
            // value é avaliado antes do binding — free vars do value
            collect_free_vars(&value.node, local_bindings, out);
            // name vira local para as expressões seguintes — MAS como
            // estamos coletando free vars do body de um lambda, o name
            // já está em local_bindings (foi adicionado via pattern binds).
        }
        TypedExprKind::Reassign { value, .. } => {
            collect_free_vars(&value.node, local_bindings, out);
        }
        TypedExprKind::Return(inner) => {
            collect_free_vars(&inner.node, local_bindings, out);
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_free_vars(&scrutinee.node, local_bindings, out);
            for arm in arms {
                // Pattern binds são locais para o arm — não propagam free vars
                // Pattern não tem free vars (só literals/sub-patterns)
                if let Some(guard) = &arm.guard {
                    collect_free_vars(&guard.node, local_bindings, out);
                }
                collect_free_vars(&arm.body.node, local_bindings, out);
            }
        }
        TypedExprKind::Lambda { clauses, .. } => {
            // Lambda aninhada: suas free vars são free vars do escopo atual
            for clause in clauses {
                let mut inner_locals = local_bindings.clone();
                collect_pattern_binds(&clause.patterns, &mut inner_locals);
                for wb in &clause.with_bindings {
                    inner_locals.insert(wb.name.clone());
                }
                if clause.guards.is_empty() {
                    collect_free_vars(&clause.body.node, &inner_locals, out);
                } else {
                    for guard in &clause.guards {
                        if let Some(cond) = &guard.condition {
                            collect_free_vars(&cond.node, &inner_locals, out);
                        }
                        collect_free_vars(&guard.body.node, &inner_locals, out);
                    }
                }
            }
        }
        TypedExprKind::Loop { body } => {
            for stmt in body {
                collect_free_vars(&stmt.node, local_bindings, out);
            }
        }
        TypedExprKind::ActionCall { args, .. } => {
            collect_free_vars(&args.node, local_bindings, out);
        }
        // Folhas sem sub-expressões
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::VariantConstruct { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue => {}
    }
}

/// Coleta nomes ligados por patterns (Ident patterns).
fn collect_pattern_binds(patterns: &[kata_ast::Spanned<TypedPattern>], out: &mut HashSet<String>) {
    for pattern in patterns {
        collect_pattern_binds_one(&pattern.node, out);
    }
}

/// Coleta binds de um pattern recursivamente.
fn collect_pattern_binds_one(pattern: &TypedPattern, out: &mut HashSet<String>) {
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
    }
}

/// Percorre um TypedMatchArm para coletar captures.
fn collect_captures_in_arm(arm: &mut TypedMatchArm, outer_env: &TypeEnv) {
    if let Some(guard) = &mut arm.guard {
        collect_captures_in_expr(&mut guard.node, outer_env);
    }
    collect_captures_in_expr(&mut arm.body.node, outer_env);
}
