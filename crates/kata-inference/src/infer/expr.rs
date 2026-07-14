//! Núcleo da inferência de expressões — o grande match sobre `Expr`.
//!
//! `infer_expr` é o entry point público (usado por todos os submódulos).
//! `infer_expr_hinted` aceita um type hint opcional (DoD 29) para inferência
//! bidirecional top-down.

use kata_ast::{DotIndex, Expr, Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::enum_registry::EnumRegistry;
use kata_core::escape::EscapeTarget;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{Effect, TypedExpr, TypedExprKind};

use super::_match::infer_match;
use super::apply::infer_apply;
use super::helpers::{InferResult, resolve_type_expr};
use super::lambda::infer_lambda;
use super::sugar::{infer_assert, infer_pipe_fallback, infer_question};
use super::variant::resolve_unqual_variant;

/// Contexto de inferência — carrega dependências compartilhadas entre
/// todas as funções de inferência. Substitui parâmetros individuais
/// `table` e `enum_registry`, e adiciona `ret_ty` para validação de
/// `return` em Actions (Fase 2).
pub(crate) struct InferCtx<'a> {
    pub table: &'a DispatchTable,
    pub enum_registry: &'a EnumRegistry,
    /// Catálogo de structs com campos — para field access e
    /// ascription-construção (Fio 5).
    pub struct_registry: &'a StructRegistry,
    /// Tipo de retorno da Action atual — `Some(ty)` quando inferindo
    /// o body de uma Action, `None` caso contrário. Usado por `infer_return`
    /// para verificar que `return expr` produz o tipo esperado.
    pub ret_ty: Option<&'a Ty>,
    /// `true` quando inferindo dentro de um `loop`. Usado por `infer_break`
    /// e `infer_continue` para validar que só aparecem dentro de loop.
    pub in_loop: bool,
}

/// Infere o tipo de uma expressão, produzindo um `TypedExpr`.
///
/// `tail_pos` é `true` quando a expressão está em posição de cauda. O entry
/// point é sempre `tail_pos = true`. Sub-expressões de `Let` value são
/// `tail_pos = false`. Argumentos de `Apply` são `tail_pos = false`.
/// Body de lambda em tail position é `tail_pos = true`. Body de match arm
/// em tail position é `tail_pos = true`.
pub(crate) fn infer_expr(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    infer_expr_hinted(expr, span, env, ctx, tail_pos, None)
}

/// Verifica se `actual` cabe em `declared` — direcional (não simétrica).
/// `Var("T")` no actual significa "não-constrangido" e aceita o declarado.
/// Recursiva dentro de `Generic`.
pub(crate) fn fits_return(actual: &Ty, declared: &Ty) -> bool {
    match (actual, declared) {
        (Ty::Var(_), _) => true,
        (Ty::Generic(n1, a1), Ty::Generic(n2, a2)) if n1 == n2 && a1.len() == a2.len() => {
            a1.iter().zip(a2).all(|(x, y)| fits_return(x, y))
        }
        _ => actual == declared,
    }
}

///
/// When `hint` is `Some(Ty::Function(params, ret))` and `expr` is a `Lambda`,
/// the params are used as the lambda's parameter types instead of InferVar.
/// When `hint` is `Some(ty)` and `expr` is a `TypeAscription`, the hint is
/// propagated to the inner expression (ascription already provides a target
/// type, so the hint is redundant there but harmless).
#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_expr_hinted(
    expr: &Expr,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let (ty, kind, effect) = match expr {
        // ── Literais ─────────────────────────────────────────
        Expr::IntLit { text } => (
            Ty::int(),
            TypedExprKind::IntLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::FloatLit { text } => (
            Ty::float(),
            TypedExprKind::FloatLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::TextLit { text } => (
            Ty::text(),
            TypedExprKind::TextLit { text: text.clone() },
            Effect::Puro,
        ),
        Expr::Unit => (Ty::Unit, TypedExprKind::Unit, Effect::Puro),

        // ── Identificador ────────────────────────────────────
        Expr::Ident { name } => {
            // Caminho normal: é uma binding no type_env.
            if let Some(ty) = env.lookup(name).cloned() {
                (
                    ty,
                    TypedExprKind::Ident { name: name.clone() },
                    Effect::Puro,
                )
            } else {
                // Fallback: variante unitária desqualificada (ex: `True`,
                // `None`, `Vermelho`). Busca no EnumRegistry.
                resolve_unqual_variant(name, span, ctx)?
            }
        }

        // ── Aplicação prefixa ────────────────────────────────
        Expr::Apply { callee, args } => infer_apply(callee, args, span, env, ctx)?,

        // ── Ascription de tipo ───────────────────────────────
        Expr::TypeAscription { expr, ty } => {
            let target_ty = resolve_type_expr(&ty.node, env);
            // Propaga o tipo anotado como hint top-down (DoD 29).
            let inner =
                infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, Some(&target_ty))?;

            // Fase 7: Ascription-construção — `(a, b)::Pessoa` → StructConstruct.
            // Se inner é Tuple e target é Struct, e o shape bate (mesmo nº de
            // elementos, tipos compatíveis), produz StructConstruct.
            if let Ty::Struct(ref struct_name) = target_ty
                && let TypedExprKind::Tuple { elements } = &inner.kind
                && let Some(struct_info) = ctx.struct_registry.get(struct_name)
                && !struct_info.fields.is_empty()
                && struct_info.alias_of.is_none()
            {
                // Shape check: mesmo número de elementos
                if elements.len() != struct_info.fields.len() {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!(
                            "Struct {} with {} fields",
                            struct_name,
                            struct_info.fields.len()
                        ),
                        found: format!("Tuple with {} elements", elements.len()),
                        span: expr.span.into(),
                    });
                }
                // Verifica tipos compatíveis
                let mut shape_ok = true;
                for (elem, field) in elements.iter().zip(struct_info.fields.iter()) {
                    if elem.node.ty != field.ty {
                        shape_ok = false;
                        break;
                    }
                }
                if shape_ok {
                    let values = elements
                        .iter()
                        .map(|e| Spanned::new(e.node.clone(), e.span))
                        .collect();
                    return Ok(TypedExpr {
                        span: *span,
                        ty: target_ty.clone(),
                        tail_pos,
                        escape: if ctx.ret_ty.is_some() {
                            if tail_pos {
                                EscapeTarget::Caller
                            } else {
                                EscapeTarget::Local
                            }
                        } else {
                            EscapeTarget::Ancestor(0)
                        },
                        effect: Effect::Puro,
                        kind: TypedExprKind::StructConstruct {
                            struct_name: struct_name.clone(),
                            values,
                        },
                    });
                }
                // Shape mismatch (tipos incompatíveis) → error
                return Err(MiddleError::TypeMismatch {
                    expected: format!(
                        "Struct {} fields {:?}",
                        struct_name,
                        struct_info.fields.iter().map(|f| &f.ty).collect::<Vec<_>>()
                    ),
                    found: format!(
                        "Tuple elements {:?}",
                        elements.iter().map(|e| &e.node.ty).collect::<Vec<_>>()
                    ),
                    span: expr.span.into(),
                });
            }

            let rebaixa_ok = match (&inner.kind, &target_ty) {
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Int)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Float)) => true,
                (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
                (TypedExprKind::TextLit { .. }, Ty::Prim(PrimTy::Text)) => true,
                _ if inner.ty == target_ty => true,
                _ => false,
            };

            if !rebaixa_ok {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{:?}", target_ty),
                    found: format!("{:?}", inner.ty),
                    span: expr.span.into(),
                });
            }

            (
                target_ty.clone(),
                TypedExprKind::TypeAscription {
                    expr: Box::new(Spanned::new(inner, expr.span)),
                    target_ty,
                },
                Effect::Puro,
            )
        }

        // ── Grouping — transparente, propaga hint ────────────
        Expr::Grouping { inner } => {
            let typed_inner =
                infer_expr_hinted(&inner.node, &inner.span, env, ctx, tail_pos, hint)?;
            (
                typed_inner.ty.clone(),
                TypedExprKind::Grouping {
                    inner: Box::new(Spanned::new(typed_inner, inner.span)),
                },
                Effect::Puro,
            )
        }

        // ── Tuple ────────────────────────────────────────────
        Expr::Tuple { elements } => {
            let mut typed_elements = Vec::with_capacity(elements.len());
            let mut element_tys = Vec::with_capacity(elements.len());
            for elem in elements {
                let typed = infer_expr(&elem.node, &elem.span, env, ctx, false)?;
                element_tys.push(typed.ty.clone());
                typed_elements.push(Spanned::new(typed, elem.span));
            }
            (
                Ty::Tuple(element_tys),
                TypedExprKind::Tuple {
                    elements: typed_elements,
                },
                Effect::Puro,
            )
        }

        // ── Let binding ──────────────────────────────────────
        Expr::Let { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();

            env.define(name, val_ty);

            (
                Ty::Unit,
                TypedExprKind::Let {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Qualificação de variante (sem Apply = unitária) ─────
        Expr::VariantQual { enum_name, variant } => {
            let enum_ty =
                env.lookup(enum_name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: enum_name.clone(),
                        span: (*span).into(),
                    })?;

            match super::variant_qual::infer_variant_qual(enum_name, variant, &enum_ty, span, ctx)?
            {
                Some(result) => result,
                None => Err(MiddleError::TypeMismatch {
                    expected: "enum".to_string(),
                    found: format!("{:?}", enum_ty),
                    span: (*span).into(),
                })?,
            }
        }

        // ── Fio 2: desugared antes do typeck ──────────────────
        Expr::Hole => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Hole deve ter sido desugared)".into(),
                found: "Hole".into(),
                span: (*span).into(),
            });
        }
        Expr::Pipe { .. } => {
            return Err(MiddleError::TypeMismatch {
                expected: "expressão (Pipe deve ter sido desugared)".into(),
                found: "Pipe".into(),
                span: (*span).into(),
            });
        }

        // ── Fio 2 Fase 8: Lambda ──────────────────────────────
        Expr::Lambda {
            patterns,
            body,
            guards,
            with_bindings,
        } => infer_lambda(patterns, body, guards, with_bindings, span, env, ctx, hint)?,

        // ── Fio 2 Fase 8: Match ───────────────────────────────
        Expr::Match { scrutinee, arms } => infer_match(scrutinee, arms, span, env, ctx, tail_pos)?,

        // ── Fio 3: ActionCall — dispatch para Action builtin ou definida ──
        Expr::ActionCall { callee, args } => {
            // Fase 9: assert! é desugared no typeck para
            // match cond { True: Unit, False: panic!(msg) }.
            if callee == "assert" {
                return infer_assert(args, span, env, ctx);
            }

            // Lowera a tupla de argumentos.
            let typed_args = infer_expr(&args.node, &args.span, env, ctx, false)?;

            // Normaliza Grouping → Tuple de 1 elemento para ActionCall args.
            // `action!(x)` produz Grouping no parser; o codegen precisa de Tuple
            // (ponteiro para array na arena) para passar args_ptr corretamente.
            let typed_args = match &typed_args.kind {
                TypedExprKind::Grouping { inner } => {
                    let inner = inner.clone();
                    TypedExpr {
                        ty: Ty::Tuple(vec![inner.node.ty.clone()]),
                        kind: TypedExprKind::Tuple {
                            elements: vec![*inner],
                        },
                        span: typed_args.span,
                        tail_pos: typed_args.tail_pos,
                        escape: typed_args.escape,
                        effect: typed_args.effect,
                    }
                }
                _ => typed_args,
            };

            // Extrai tipos dos elementos da tupla para dispatch.
            let arg_tys: Vec<Ty> = match &typed_args.kind {
                TypedExprKind::Tuple { elements } => {
                    elements.iter().map(|e| e.node.ty.clone()).collect()
                }
                TypedExprKind::Unit => Vec::new(), // `!()` = tupla vazia
                _ => vec![typed_args.ty.clone()],  // args não-tupla (não deveria acontecer)
            };

            // Resolve no DispatchTable.
            let overload = ctx
                .table
                .resolve(callee, &arg_tys)
                .map_err(|e| super::helpers::dispatch_to_middle_error(e, *span))?;

            // Verifica que é uma Action (is_action = true).
            if !overload.is_action {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("Action `{callee}` (is_action=true)"),
                    found: format!("função pura `{callee}` — use sem `!`"),
                    span: (*span).into(),
                });
            }

            (
                overload.ret,
                TypedExprKind::ActionCall {
                    callee: callee.clone(),
                    args: Box::new(Spanned::new(typed_args, args.span)),
                    caller_arena: 0, // placeholder — preenchido no codegen
                    ffi_symbol: overload.ffi_symbol.clone().filter(|_s| overload.is_action),
                },
                Effect::Puro, // Fio 3 não ativa Effect
            )
        }

        // ── Fio 3: var — binding mutável (exclusivo de Actions) ──
        Expr::Var { name, value } => {
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            let val_ty = typed_value.ty.clone();
            env.define_mutable(name, val_ty);
            (
                Ty::Unit,
                TypedExprKind::Var {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Fio 3: Reassign — reatribuição a variável `var` ──
        Expr::Reassign { name, value } => {
            // Verifica que a variável existe e foi declarada como mutável.
            let existing_ty =
                env.lookup(name)
                    .cloned()
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: name.clone(),
                        span: (*span).into(),
                    })?;
            if !env.is_mutable(name) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("variável mutável `{name}` (declarada com `var`)"),
                    found: format!("variável imutável `{name}` (declarada com `let`)"),
                    span: (*span).into(),
                });
            }
            let typed_value = infer_expr(&value.node, &value.span, env, ctx, false)?;
            if typed_value.ty != existing_ty {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{existing_ty:?}"),
                    found: format!("{:?}", typed_value.ty),
                    span: value.span.into(),
                });
            }
            (
                Ty::Unit,
                TypedExprKind::Reassign {
                    name: name.clone(),
                    value: Box::new(Spanned::new(typed_value, value.span)),
                },
                Effect::Puro,
            )
        }

        // ── Fio 3: return — early return de Action (Fase 2) ──
        Expr::Return(inner) => {
            let ret_ty = ctx.ret_ty.ok_or_else(|| MiddleError::TypeMismatch {
                expected: "return dentro de Action".into(),
                found: "return fora de Action".into(),
                span: (*span).into(),
            })?;
            let typed_inner = infer_expr(&inner.node, &inner.span, env, ctx, false)?;
            if !fits_return(&typed_inner.ty, ret_ty) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{ret_ty:?}"),
                    found: format!("{:?}", typed_inner.ty),
                    span: inner.span.into(),
                });
            }
            (
                typed_inner.ty.clone(),
                TypedExprKind::Return(Box::new(Spanned::new(typed_inner, inner.span))),
                Effect::Puro,
            )
        }
        Expr::Loop { body } => {
            // Loop body é inferido com in_loop = true.
            // Cada expr do body é inferida em sequência no mesmo escopo.
            // O tipo do loop é Unit (break sem valor na Fase 4).
            let loop_ctx = InferCtx {
                table: ctx.table,
                enum_registry: ctx.enum_registry,
                struct_registry: ctx.struct_registry,
                ret_ty: ctx.ret_ty,
                in_loop: true,
            };
            let mut typed_body = Vec::new();
            for expr in body {
                let typed = infer_expr(
                    &expr.node, &expr.span, env, &loop_ctx,
                    false, // body do loop nunca é tail_pos (loop retorna Unit)
                )?;
                typed_body.push(Spanned::new(typed, expr.span));
            }
            (
                Ty::Unit,
                TypedExprKind::Loop { body: typed_body },
                Effect::Puro,
            )
        }
        Expr::Break => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (break só existe dentro de loop)".into(),
                    found: "Break".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Break, Effect::Puro)
        }
        Expr::Continue => {
            if !ctx.in_loop {
                return Err(MiddleError::TypeMismatch {
                    expected: "expressão (continue só existe dentro de loop)".into(),
                    found: "Continue".into(),
                    span: (*span).into(),
                });
            }
            (Ty::Unit, TypedExprKind::Continue, Effect::Puro)
        }

        // ── Fase 7: `?` fail-fast — desugar para Match + Return ──
        Expr::Question(inner) => {
            return infer_question(inner, span, env, ctx, tail_pos);
        }
        // ── Fase 8: `|` fallback — desugar para Match (coalescência pura) ──
        Expr::PipeFallback { lhs, rhs } => {
            return infer_pipe_fallback(lhs, rhs, span, env, ctx);
        }
        // ── Fio 5: DotAccess (field access + index access) ──
        Expr::DotAccess { expr, index } => {
            return infer_dot_access(expr, index, span, env, ctx, tail_pos);
        }
        // ── Fio 5: Spread ($) — typeck expande, nunca deveria chegar aqui ──
        Expr::Spread => {
            return Err(MiddleError::UnboundName {
                name: "Spread ($) em posição inesperada — typeck deveria ter expandido".into(),
                span: (*span).into(),
            });
        }
    };

    // Deriva EscapeTarget de tail_pos + contexto (Action vs função pura/entry).
    let escape = if ctx.ret_ty.is_some() {
        // Dentro de Action: tail_pos = true → Caller, false → Local.
        if tail_pos {
            EscapeTarget::Caller
        } else {
            EscapeTarget::Local
        }
    } else {
        // Função pura / entry point: sem fiber_arena, tudo vai para a raiz.
        EscapeTarget::Ancestor(0)
    };

    Ok(TypedExpr {
        span: *span,
        ty,
        tail_pos,
        escape,
        effect,
        kind,
    })
}

// ── Fio 5: DotAccess — field access em struct + index access em tupla ──

/// Infere `expr.nome` (field access) ou `expr.N` (index access).
///
/// Desambiguação pelo tipo do receptor:
/// - `Ty::Struct(name)` + `DotIndex::Field` → `FieldAccess`
/// - `Ty::Struct(name)` + `DotIndex::Int` → erro `IndexAccessOnStruct`
/// - `Ty::Tuple(elements)` + `DotIndex::Int(n)` → `IndexAccess` (negativos
///   normalizados, bounds check compile-time)
/// - `Ty::Tuple(elements)` + `DotIndex::Field` → erro `FieldAccessOnTuple`
/// - Outro → erro `NotIndexable`
fn infer_dot_access(
    expr: &Spanned<Expr>,
    index: &DotIndex,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
) -> InferResult<TypedExpr> {
    let inner = infer_expr(&expr.node, &expr.span, env, ctx, false)?;
    let inner_spanned = Spanned::new(inner.clone(), expr.span);
    let inner_box = Box::new(inner_spanned);

    match (&inner.ty, index) {
        (Ty::Struct(struct_name), DotIndex::Field(field_name)) => {
            let info =
                ctx.struct_registry
                    .get(struct_name)
                    .ok_or_else(|| MiddleError::UnboundName {
                        name: format!("struct `{struct_name}` não registrado no StructRegistry"),
                        span: (*span).into(),
                    })?;
            let (field_index, field_info) =
                info.find_field(field_name)
                    .ok_or_else(|| MiddleError::UnknownField {
                        struct_name: struct_name.clone(),
                        field_name: field_name.clone(),
                        span: (*span).into(),
                    })?;
            let ty = field_info.ty.clone();
            Ok(TypedExpr {
                span: *span,
                ty,
                tail_pos,
                escape: inner.escape,
                effect: inner.effect,
                kind: TypedExprKind::FieldAccess {
                    expr: inner_box,
                    struct_name: struct_name.clone(),
                    field_name: field_name.clone(),
                    field_index,
                },
            })
        }
        (Ty::Struct(_), DotIndex::Int(_)) => Err(MiddleError::IndexAccessOnStruct {
            span: (*span).into(),
        }),
        (Ty::Tuple(elements), DotIndex::Int(n)) => {
            let len = elements.len() as i64;
            // Normaliza negativo: -1 = len-1, -2 = len-2, etc.
            let resolved = if *n < 0 { len + n } else { *n };
            if resolved < 0 || resolved >= len {
                return Err(MiddleError::IndexOutOfBounds {
                    index: *n,
                    len: len as usize,
                    span: (*span).into(),
                });
            }
            let element_index = resolved as u32;
            let ty = elements[resolved as usize].clone();
            Ok(TypedExpr {
                span: *span,
                ty,
                tail_pos,
                escape: inner.escape,
                effect: inner.effect,
                kind: TypedExprKind::IndexAccess {
                    expr: inner_box,
                    index: *n,
                    element_index,
                },
            })
        }
        (Ty::Tuple(_), DotIndex::Field(_)) => Err(MiddleError::FieldAccessOnTuple {
            span: (*span).into(),
        }),
        (other_ty, _) => Err(MiddleError::NotIndexable {
            ty: format!("{other_ty:?}"),
            span: (*span).into(),
        }),
    }
}
