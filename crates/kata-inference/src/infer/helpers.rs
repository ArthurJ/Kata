//! Helpers compartilhados entre os submódulos de inferência.
//!
//! Funções utilitárias usadas por múltiplos submódulos do `infer/`:
//! conversão de erros, resolução de TypeExpr, peeling de Grouping,
//! populate do DispatchTable e span fallback.

use kata_ast::{Item, Pattern, Span, Spanned, TypeExpr, WithBinding};
use kata_core::dispatch::{DispatchError, DispatchTable, OverloadInfo};
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::{MiddleError, MietteSpan};
use kata_resolution::Signature;

use crate::patterns;
use crate::typed::{TypedPattern, TypedWithBinding};

use super::expr::{InferCtx, infer_expr};

/// Erro de inferência — wrapped `MiddleError` (carrega Span).
pub type InferResult<T> = Result<T, MiddleError>;

/// Popula o DispatchTable a partir das assinaturas do ResolvedModule.
pub(crate) fn populate_dispatch_table(signatures: &[Signature]) -> DispatchTable {
    let mut table = DispatchTable::new();
    for sig in signatures {
        let ffi_symbol = sig.ffi_symbol.clone();
        let associative_neutral = sig.associative_neutral;

        table.insert(OverloadInfo {
            name: sig.name.clone(),
            params: sig.param_types.clone(),
            ret: sig.return_type.clone(),
            ffi_symbol,
            is_action: sig.is_action,
            is_generic: false,
            is_constructor: false,
            associative_neutral,
            type_params: sig.type_params.clone(),
            substitutions: None,
        });

        // Fase 7: marca comutativa quando a assinatura tem @commutative.
        if sig.is_commutative {
            table.mark_commutative(&sig.name);
        }
    }
    table
}

/// Span do último item ou sintético se módulo vazio.
pub(crate) fn item_span_or_synthetic(items: &[Spanned<Item>]) -> MietteSpan {
    items
        .last()
        .map(|i| i.span.into())
        .unwrap_or(MietteSpan(Span::synthetic()))
}

/// Converte `DispatchError` em `MiddleError` para diagnóstico.
pub(crate) fn dispatch_to_middle_error(err: DispatchError, span: Span) -> MiddleError {
    match err {
        DispatchError::FunctionNotFound { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::TypeMismatch { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::AmbiguousDispatch { name, .. } => MiddleError::AmbiguousDispatch {
            name,
            span: span.into(),
        },
    }
}

/// Resolve `TypeExpr` → `Ty` usando o TypeEnv. Igual ao `resolve_type_expr`
/// do resolution, mas replicado aqui para evitar depender de função privada.
pub(crate) fn resolve_type_expr(expr: &TypeExpr, env: &TypeEnv) -> Ty {
    match expr {
        TypeExpr::Named(name) => {
            if let Some(ty) = env.lookup(name) {
                ty.clone()
            } else {
                match name.as_str() {
                    "Int" => Ty::Prim(PrimTy::Int),
                    "Float" => Ty::Prim(PrimTy::Float),
                    "Text" => Ty::Prim(PrimTy::Text),
                    "Rational" => Ty::Prim(PrimTy::Rational),
                    "Boolean" => Ty::Sum("Boolean".into()),
                    "Unit" => Ty::Unit,
                    _ => Ty::Struct(name.clone()),
                }
            }
        }
        TypeExpr::Unit => Ty::Unit,
        TypeExpr::Grouping(inner) => resolve_type_expr(&inner.node, env),
        TypeExpr::Tuple(elements) => {
            let tys: Vec<Ty> = elements
                .iter()
                .map(|t| resolve_type_expr(&t.node, env))
                .collect();
            Ty::Tuple(tys)
        }
        TypeExpr::Func { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env);
            Ty::Function(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, .. } => Ty::Sum(name.clone()),
        // Fio 7: Self é resolvido na Fase 2. Placeholder por ora.
        TypeExpr::SelfRef => Ty::Var("Self".into()),
    }
}

/// Remove camadas de `Expr::Grouping` — retorna a expressão interna.
pub(crate) fn peel_grouping_expr(expr: &kata_ast::Expr) -> &kata_ast::Expr {
    match expr {
        kata_ast::Expr::Grouping { inner } => peel_grouping_expr(&inner.node),
        _ => expr,
    }
}

/// Casa uma lista de padrões contra tipos esperados, registrando bindings no escopo.
///
/// Extrai o loop comum a `infer_named_function`, `infer_lambda`, e aos dois
/// caminhos de apply de lambda inline. Retorna os padrões tipados na mesma
/// ordem dos argumentos.
pub(crate) fn check_patterns(
    patterns: &[Spanned<Pattern>],
    param_tys: &[Ty],
    enum_registry: &EnumRegistry,
    env: &mut TypeEnv,
) -> InferResult<Vec<Spanned<TypedPattern>>> {
    let mut typed_patterns: Vec<Spanned<TypedPattern>> = Vec::with_capacity(patterns.len());
    for (i, pat) in patterns.iter().enumerate() {
        let typed_pat = patterns::check_pattern(pat, &param_tys[i], enum_registry, env)?;
        typed_patterns.push(typed_pat);
    }
    Ok(typed_patterns)
}

/// Processa with bindings (açúcar → let chain) num escopo filho.
///
/// Infere o valor de cada binding, registra no `env` e coleta os bindings
/// tipados. Compartilhado por `infer_named_function`, `infer_lambda`, e pelos
/// dois caminhos de apply de lambda inline.
pub(crate) fn process_with_bindings(
    wbs: &[WithBinding],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<Vec<TypedWithBinding>> {
    let mut typed_with_bindings: Vec<TypedWithBinding> = Vec::new();
    for wb in wbs {
        let typed_value = infer_expr(&wb.value.node, &wb.value.span, env, ctx, false)?;
        let val_ty = typed_value.ty.clone();
        env.define(&wb.name, val_ty);
        typed_with_bindings.push(TypedWithBinding {
            name: wb.name.clone(),
            value: Spanned::new(typed_value, wb.value.span),
        });
    }
    Ok(typed_with_bindings)
}
