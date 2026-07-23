//! Instantiation — type specialization for monomorphization.
//!
//! These functions take a generic `TypedFunction` / `TypedExpr` / `TypedPattern`
//! and a `Substitutions` map, and produce a concrete instance with all
//! `Ty::Var("T")` replaced by the concrete types.
//!
//! Naming helpers (`canonicalize_subs`, `ty_to_string`) foram extraídas para
//! `naming.rs`. Arms de collections (`ListLit`, `ArrayLit`, `RangeLit`, `ForIn`,
//! `In`, `Map`, `Filter`, `Fold`, `FusedStream`) foram extraídos para
//! `instantiate_collections.rs`.

use kata_ast::Spanned;
use kata_core::ty::Ty;
use kata_inference::{
    Substitutions, TypedAction, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause,
    TypedPattern, apply_subs,
};

/// Gera uma instância monomorfizada de uma `TypedFunction`.
///
/// Substitui todos os `Ty::Var("T")` pelos tipos concretos em `subs`
/// nos param_types, ret_ty, e no corpo de cada cláusula.
pub(crate) fn instantiate_function(
    orig: &TypedFunction,
    subs: &Substitutions,
    instance_name: &str,
) -> TypedFunction {
    let param_types: Vec<Ty> = orig
        .param_types
        .iter()
        .map(|t| apply_subs(t, subs))
        .collect();
    let ret_ty = apply_subs(&orig.ret_ty, subs);
    let clauses: Vec<TypedLambdaClause> = orig
        .clauses
        .iter()
        .map(|c| instantiate_clause(c, subs))
        .collect();

    TypedFunction {
        name: instance_name.to_string(),
        param_types,
        ret_ty,
        clauses,
        log: orig.log.clone(),
    }
}

/// Gera uma instância monomorfizada de uma `TypedAction`.
///
/// Análogo a `instantiate_function` mas para Actions: substitui
/// `Ty::Var("T")` e `Ty::Interface("SHOW")` pelos tipos concretos em `subs`
/// nos param_types, ret_ty, e no body (statements). Também propaga as
/// substituições para os tipos dos parâmetros nomeados.
pub(crate) fn instantiate_action(
    orig: &TypedAction,
    subs: &Substitutions,
    instance_name: &str,
) -> TypedAction {
    let param_types: Vec<Ty> = orig
        .param_types
        .iter()
        .map(|t| apply_subs(t, subs))
        .collect();
    let ret_ty = apply_subs(&orig.ret_ty, subs);
    let body: Vec<Spanned<TypedExpr>> = orig
        .body
        .iter()
        .map(|stmt| Spanned::new(instantiate_typed_expr(&stmt.node, subs), stmt.span))
        .collect();

    TypedAction {
        name: instance_name.to_string(),
        param_types,
        param_names: orig.param_names.clone(),
        ret_ty,
        body,
        tests: orig.tests.clone(),
        log: orig.log.clone(),
    }
}

/// Instancia uma cláusula — substitui Ty::Var nos padrões e corpo.
pub(crate) fn instantiate_clause(
    clause: &TypedLambdaClause,
    subs: &Substitutions,
) -> TypedLambdaClause {
    TypedLambdaClause {
        patterns: clause
            .patterns
            .iter()
            .map(|p| Spanned::new(instantiate_pattern(&p.node, subs), p.span))
            .collect(),
        body: Spanned::new(
            instantiate_typed_expr(&clause.body.node, subs),
            clause.body.span,
        ),
        guards: clause
            .guards
            .iter()
            .map(|g| instantiate_guard(g, subs))
            .collect(),
        with_bindings: clause
            .with_bindings
            .iter()
            .map(|wb| kata_inference::TypedWithBinding {
                name: wb.name.clone(),
                value: Spanned::new(instantiate_typed_expr(&wb.value.node, subs), wb.value.span),
            })
            .collect(),
    }
}

/// Instancia um guard.
fn instantiate_guard(
    guard: &kata_inference::TypedGuardClause,
    subs: &Substitutions,
) -> kata_inference::TypedGuardClause {
    kata_inference::TypedGuardClause {
        condition: guard
            .condition
            .as_ref()
            .map(|c| Spanned::new(instantiate_typed_expr(&c.node, subs), c.span)),
        body: Spanned::new(
            instantiate_typed_expr(&guard.body.node, subs),
            guard.body.span,
        ),
    }
}

/// Instancia um `TypedPattern` — substitui Ty::Var no tipo dos bindings.
pub(crate) fn instantiate_pattern(pattern: &TypedPattern, subs: &Substitutions) -> TypedPattern {
    match pattern {
        TypedPattern::Ident { name, ty } => TypedPattern::Ident {
            name: name.clone(),
            ty: apply_subs(ty, subs),
        },
        TypedPattern::Wildcard => TypedPattern::Wildcard,
        TypedPattern::Literal { value } => TypedPattern::Literal {
            value: Spanned::new(instantiate_typed_expr(&value.node, subs), value.span),
        },
        TypedPattern::Variant {
            enum_name,
            variant,
            sub_patterns,
            tag,
        } => TypedPattern::Variant {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            sub_patterns: sub_patterns.as_ref().map(|sps| {
                sps.iter()
                    .map(|sp| Spanned::new(instantiate_pattern(&sp.node, subs), sp.span))
                    .collect()
            }),
            tag: *tag,
        },
        TypedPattern::Tuple { elements } => TypedPattern::Tuple {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_pattern(&e.node, subs), e.span))
                .collect(),
        },
        TypedPattern::Cons { head, tail } => TypedPattern::Cons {
            head: Box::new(Spanned::new(
                instantiate_pattern(&head.node, subs),
                head.span,
            )),
            tail: Box::new(Spanned::new(
                instantiate_pattern(&tail.node, subs),
                tail.span,
            )),
        },
        TypedPattern::Nil => TypedPattern::Nil,
    }
}

/// Instancia um `TypedExpr` — substitui Ty::Var no tipo do nó e recurse nos filhos.
pub(crate) fn instantiate_typed_expr(expr: &TypedExpr, subs: &Substitutions) -> TypedExpr {
    let new_ty = apply_subs(&expr.ty, subs);
    TypedExpr {
        span: expr.span,
        ty: new_ty,
        tail_pos: expr.tail_pos,
        escape: expr.escape,
        effect: expr.effect,
        kind: instantiate_kind(&expr.kind, subs),
    }
}

/// Instancia um `TypedExprKind` — recursão nos filhos.
fn instantiate_kind(kind: &TypedExprKind, subs: &Substitutions) -> TypedExprKind {
    match kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => TypedExprKind::Closure {
            callee: Box::new(Spanned::new(
                instantiate_typed_expr(&callee.node, subs),
                callee.span,
            )),
            args: args
                .iter()
                .map(|a| Spanned::new(instantiate_typed_expr(&a.node, subs), a.span))
                .collect(),
            ffi_symbol: ffi_symbol.clone(),
        },

        TypedExprKind::TypeAscription {
            expr: inner,
            target_ty,
        } => TypedExprKind::TypeAscription {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            target_ty: apply_subs(target_ty, subs),
        },

        TypedExprKind::Grouping { inner } => TypedExprKind::Grouping {
            inner: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
        },

        TypedExprKind::Tuple { elements } => TypedExprKind::Tuple {
            elements: elements
                .iter()
                .map(|e| Spanned::new(instantiate_typed_expr(&e.node, subs), e.span))
                .collect(),
        },

        TypedExprKind::StructConstruct {
            struct_name,
            values,
        } => TypedExprKind::StructConstruct {
            struct_name: struct_name.clone(),
            values: values
                .iter()
                .map(|v| Spanned::new(instantiate_typed_expr(&v.node, subs), v.span))
                .collect(),
        },

        TypedExprKind::FieldAccess {
            expr: inner,
            struct_name,
            field_name,
            field_index,
        } => TypedExprKind::FieldAccess {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            struct_name: struct_name.clone(),
            field_name: field_name.clone(),
            field_index: *field_index,
        },

        TypedExprKind::IndexAccess {
            expr: inner,
            index,
            element_index,
        } => TypedExprKind::IndexAccess {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&inner.node, subs),
                inner.span,
            )),
            index: *index,
            element_index: *element_index,
        },

        TypedExprKind::Let { name, value } => TypedExprKind::Let {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::LetDestruct {
            temp_name,
            value,
            bindings,
        } => TypedExprKind::LetDestruct {
            temp_name: temp_name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
            bindings: bindings
                .iter()
                .map(|(name, expr)| {
                    (
                        name.clone(),
                        Spanned::new(instantiate_typed_expr(&expr.node, subs), expr.span),
                    )
                })
                .collect(),
        },

        TypedExprKind::Lambda {
            func_name,
            param_types,
            ret_ty,
            clauses,
            captures,
        } => TypedExprKind::Lambda {
            func_name: func_name.clone(),
            param_types: param_types.iter().map(|t| apply_subs(t, subs)).collect(),
            ret_ty: apply_subs(ret_ty, subs),
            clauses: clauses
                .iter()
                .map(|c| instantiate_clause(c, subs))
                .collect(),
            captures: captures
                .iter()
                .map(|c| kata_inference::CaptureInfo {
                    name: c.name.clone(),
                    ty: apply_subs(&c.ty, subs),
                })
                .collect(),
        },

        TypedExprKind::Match { scrutinee, arms } => TypedExprKind::Match {
            scrutinee: Box::new(Spanned::new(
                instantiate_typed_expr(&scrutinee.node, subs),
                scrutinee.span,
            )),
            arms: arms
                .iter()
                .map(|arm| kata_inference::TypedMatchArm {
                    pattern: arm
                        .pattern
                        .as_ref()
                        .map(|p| Spanned::new(instantiate_pattern(&p.node, subs), p.span)),
                    guard: arm
                        .guard
                        .as_ref()
                        .map(|g| Spanned::new(instantiate_typed_expr(&g.node, subs), g.span)),
                    body: Spanned::new(instantiate_typed_expr(&arm.body.node, subs), arm.body.span),
                })
                .collect(),
        },

        TypedExprKind::VariantQual {
            enum_name,
            variant,
            tag,
        } => TypedExprKind::VariantQual {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            tag: *tag,
        },

        TypedExprKind::VariantConstruct {
            enum_name,
            variant,
            payload,
            tag,
        } => TypedExprKind::VariantConstruct {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            payload: Box::new(Spanned::new(
                instantiate_typed_expr(&payload.node, subs),
                payload.span,
            )),
            tag: *tag,
        },

        TypedExprKind::Var { name, value } => TypedExprKind::Var {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::Reassign { name, value } => TypedExprKind::Reassign {
            name: name.clone(),
            value: Box::new(Spanned::new(
                instantiate_typed_expr(&value.node, subs),
                value.span,
            )),
        },

        TypedExprKind::Return(inner) => TypedExprKind::Return(Box::new(Spanned::new(
            instantiate_typed_expr(&inner.node, subs),
            inner.span,
        ))),

        TypedExprKind::ActionCall {
            callee,
            args,
            caller_arena,
            ffi_symbol,
            indirect_callee,
        } => TypedExprKind::ActionCall {
            callee: callee.clone(),
            args: Box::new(Spanned::new(
                instantiate_typed_expr(&args.node, subs),
                args.span,
            )),
            caller_arena: *caller_arena,
            ffi_symbol: ffi_symbol.clone(),
            indirect_callee: indirect_callee.as_ref().map(|ic| {
                Box::new(Spanned::new(
                    instantiate_typed_expr(&ic.node, subs),
                    ic.span,
                ))
            }),
        },

        TypedExprKind::TypeOf { expr } => TypedExprKind::TypeOf {
            expr: Box::new(Spanned::new(
                instantiate_typed_expr(&expr.node, subs),
                expr.span,
            )),
        },

        TypedExprKind::Loop { body } => TypedExprKind::Loop {
            body: body
                .iter()
                .map(|s| Spanned::new(instantiate_typed_expr(&s.node, subs), s.span))
                .collect(),
        },

        // Folhas — sem sub-expressões, sem Ty::Var.
        TypedExprKind::IntLit { text } => TypedExprKind::IntLit { text: text.clone() },
        TypedExprKind::FloatLit { text } => TypedExprKind::FloatLit { text: text.clone() },
        TypedExprKind::TextLit { text } => TypedExprKind::TextLit { text: text.clone() },
        TypedExprKind::Unit => TypedExprKind::Unit,
        TypedExprKind::Ident { name } => TypedExprKind::Ident { name: name.clone() },
        TypedExprKind::Break => TypedExprKind::Break,
        TypedExprKind::Continue => TypedExprKind::Continue,

        // ── Coleções + HOFs/FusedStream ──
        // Delegado para `instantiate_collections` — arms de collections.
        _ => {
            if let Some(kind) = crate::instantiate_collections::instantiate_collections(kind, subs)
            {
                kind
            } else {
                unreachable!("instantiate_kind: variante não tratada: {kind:?}")
            }
        }
    }
}
