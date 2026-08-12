//! Validações de constness para `constant` — verificadas na inferência.
//!
//! Um `constant` deve ser:
//! - **Serializável**: não pode ser lambda (Function não é serializável em
//!   compile-time, PRD §3.7). O usuário deve usar named function.
//! - **Comptime-available**: o value não pode depender de valores runtime
//!   (parâmetros, var de Action, I/O). Na inferência (pré-pass 2a), o
//!   `type_env` contém apenas bindings de módulo (constants anteriores e
//!   funções nomeadas) — qualquer Ident que não está no `type_env` não é
//!   comptime-available.
//! - **Puro**: não pode conter ActionCall, Fork, ChannelSend, etc.

use kata_core::ty::Ty;
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

/// Faz peel de Grouping e TypeAscription para verificar se o value
/// subjacente é uma Lambda. Se for, retorna o tipo (Function) da lambda
/// para construir a mensagem de erro com a assinatura esperada.
pub(crate) fn peel_to_lambda_ty(expr: &TypedExpr) -> Option<Ty> {
    match &expr.kind {
        TypedExprKind::Lambda { .. } => Some(expr.ty.clone()),
        TypedExprKind::Grouping { inner } => peel_to_lambda_ty(&inner.node),
        TypedExprKind::TypeAscription { expr: inner, .. } => peel_to_lambda_ty(&inner.node),
        _ => None,
    }
}

/// Formata um `Ty::Function` como assinatura Kata (`Int Int => Int`).
fn format_function_sig(ty: &Ty) -> String {
    if let Ty::Function(params, ret) = ty {
        let params_str = params
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        format!("{params_str} => {}", ret.display())
    } else {
        ty.display().to_string()
    }
}

/// Verifica se o value de um `constant` é uma lambda. Se for, retorna
/// `MiddleError::ConstantLambda` com a assinatura esperada.
pub(crate) fn check_constant_lambda(
    name: &str,
    value: &TypedExpr,
    span: kata_ast::Span,
) -> Result<(), MiddleError> {
    if let Some(ty) = peel_to_lambda_ty(value) {
        let sig = format_function_sig(&ty);
        return Err(MiddleError::ConstantLambda {
            name: name.to_string(),
            sig,
            span: span.into(),
        });
    }
    Ok(())
}
