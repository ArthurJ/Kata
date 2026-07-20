//! Inferência de Actions — extrai `infer_action` de `mod.rs`.
//!
//! Produz `TypedAction` a partir de `ActionDef`: infere cada statement do
//! body em sequência, valida o tipo de retorno, tipa args de `@test` specs,
//! e sintetiza `@log` spec. Separado de `mod.rs` por ser self-contained
//! (só recebe `ActionDef`, `InferCtx`, `TypeEnv`).

use kata_ast::Spanned;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::desugar;
use crate::typed::{TypedAction, TypedExpr, TypedExprKind, TypedTestSpec};

use super::expr::{InferCtx, fits_return, infer_expr};
use super::helpers::InferResult;
use super::log_synthesis;

/// Infere uma Action — produz `TypedAction` a partir de `ActionDef`.
///
/// O body é uma sequência de statements (`ActionStmt`). Cada statement é
/// inferido em sequência no mesmo escopo. O último statement **sem `;`** é
/// o retorno implícito — verifica tipo contra `ret_ty`. O último statement
/// **com `;`** retorna `Unit` — se `ret_ty` não for `Unit`, é um erro.
/// Após um `return`, statements subsequentes são unreachable — paramos.
pub(crate) fn infer_action(
    action_def: &kata_resolution::ActionDef,
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InferResult<TypedAction> {
    let param_types = &action_def.param_types;
    let ret_ty = &action_def.return_type;

    // Cria escopo para a Action com o type_env do módulo como parent.
    // Isso permite que o body da Action acesse tipos do prelude (Result, Optional, etc).
    let mut action_env = TypeEnv::with_parent(module_type_env.clone());

    // Define parâmetros no escopo.
    // `__param_N` é o nome interno usado pelo codegen (def_var __param_N).
    // O nome nomeado (`x`) é um alias apontando para o mesmo tipo.
    for (i, ty) in param_types.iter().enumerate() {
        action_env.define(&format!("__param_{i}"), ty.clone());
    }
    // Define nomes nomeados dos params (forma nomeada `x::Tipo`) no escopo,
    // além de `__param_N`. O codegen mapeia `x` → `__param_N` via o typeck.
    for (i, opt_name) in action_def.param_names.iter().enumerate() {
        if let Some(name) = opt_name
            && let Some(ty) = param_types.get(i)
        {
            action_env.define(name, ty.clone());
        }
    }

    // Infere cada statement do body em sequência.
    // O último statement sem `;` é o retorno implícito (tail_pos = true).
    // O último statement com `;` retorna Unit (tail_pos = false).
    // Após um `return`, statements subsequentes são unreachable — paramos.
    let mut typed_body: Vec<Spanned<TypedExpr>> = Vec::new();
    let n = action_def.body.len();
    for (i, stmt) in action_def.body.iter().enumerate() {
        let is_last = i == n - 1;
        // Se o último statement tem `;`, não é retorno implícito.
        let tail_pos = is_last && !stmt.has_semicolon;
        let desugared = desugar::desugar(&stmt.expr);
        let typed = infer_expr(
            &desugared.node,
            &desugared.span,
            &mut action_env,
            ctx,
            tail_pos,
        )?;
        let is_return = matches!(typed.kind, TypedExprKind::Return(_));
        typed_body.push(Spanned::new(typed, stmt.expr.span));
        if is_return {
            break; // statements após return são unreachable
        }
    }

    // Verifica que o último statement produz o tipo esperado.
    // Se o body terminou com `return`, o tipo já foi validado em infer_return.
    // Senão, o último statement é o retorno implícito (ou Unit se tinha `;`).
    if let Some(last) = typed_body.last()
        && !matches!(last.node.kind, TypedExprKind::Return(_))
    {
        let actual_ty = &last.node.ty;
        // Se o último statement tinha `;`, o retorno é Unit.
        let expected_ty = if action_def.body.last().is_some_and(|s| s.has_semicolon) {
            &Ty::Unit
        } else {
            ret_ty
        };
        if !fits_return(actual_ty, expected_ty) {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{expected_ty:?}"),
                found: format!("{actual_ty:?}"),
                span: last.span.into(),
            });
        }
    }

    // Tipa os args de cada @test spec (Expr → TypedExpr).
    // infer_expr pode falhar (type mismatch nos args) — propaga o erro.
    let mut typed_tests: Vec<TypedTestSpec> = Vec::new();
    for spec in &action_def.tests {
        let typed_args = if let Some(args_expr) = &spec.args {
            let desugared = desugar::desugar(args_expr);
            let typed = infer_expr(
                &desugared.node,
                &desugared.span,
                &mut action_env,
                ctx,
                false,
            )?;
            Some(Spanned::new(typed, args_expr.span))
        } else {
            None
        };
        typed_tests.push(TypedTestSpec {
            desc: spec.desc.clone(),
            args: typed_args,
            timeout: spec.timeout,
            expects: spec.expects.clone(),
        });
    }

    // Sintetiza log spec se a action tem @log.
    // Params nomeados (forma `x::Tipo`) ficam disponíveis para o template
    // do `when: "enter"` via param_names. `when: "exit"` infere o template no
    // escopo da action (params + vars do corpo).
    let log = if let Some(log_spec) = &action_def.log {
        let param_names: Vec<String> = action_def
            .param_names
            .iter()
            .filter_map(|n| n.clone())
            .collect();
        Some(log_synthesis::synthesize_log_spec(
            log_spec,
            &param_names,
            &mut action_env,
            ctx,
        )?)
    } else {
        None
    };

    Ok(TypedAction {
        name: action_def.name.clone(),
        param_types: param_types.clone(),
        param_names: action_def.param_names.clone(),
        ret_ty: ret_ty.clone(),
        body: typed_body,
        tests: typed_tests,
        log,
    })
}
