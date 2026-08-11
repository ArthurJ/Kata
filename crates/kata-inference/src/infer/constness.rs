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

use kata_core::ty::{Ty, TypeEnv};
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

/// Verifica se uma expressão é comptime-available **no contexto da
/// inferência** (pré-pass 2a). Neste ponto, o `dispatch_table` já tem
/// todas as funções nomeadas registradas (passo 1), e o `type_env`
/// contém apenas constants anteriores (processadas em iterações
/// anteriores do loop 2a).
///
/// Diferente do `is_comptime_available` do comptime pass (que usa
/// `comptime_bindings` com dataflow dinâmico), esta versão é mais simples:
/// um Ident é comptime-available se é uma função nomeada no
/// `dispatch_table` (não-action) ou uma constant anterior no `type_env`.
/// Casos que só aparecem em runtime (parâmetros, var) não existem em
/// nenhum dos dois.
pub(crate) fn is_consttime_available_at_infer(
    expr: &TypedExpr,
    type_env: &TypeEnv,
    dispatch_table: &kata_core::dispatch::DispatchTable,
) -> bool {
    check_at_infer(expr, type_env, dispatch_table)
}

fn check_at_infer(
    expr: &TypedExpr,
    type_env: &TypeEnv,
    dispatch_table: &kata_core::dispatch::DispatchTable,
) -> bool {
    match &expr.kind {
        // Literais — sempre comptime-available.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::HeapSnapshot { .. } => true,

        // Ident — comptime-available se é uma função nomeada no
        // dispatch_table (não-action) ou uma constant anterior no type_env.
        TypedExprKind::Ident { name } => {
            // Função nomeada no dispatch_table (não-action)?
            if let Some(overloads) = dispatch_table.get_overloads(name) {
                if overloads.iter().any(|oi| !oi.is_action) {
                    return true;
                }
            }
            // Constant anterior no type_env?
            if type_env.lookup(name).is_some() {
                return true;
            }
            false
        }

        // Closure (chamada de função) — comptime se callee e args são.
        TypedExprKind::Closure { callee, args, .. } => {
            check_at_infer(&callee.node, type_env, dispatch_table)
                && args
                    .iter()
                    .all(|a| check_at_infer(&a.node, type_env, dispatch_table))
        }

        // TypeAscription — comptime se inner é.
        TypedExprKind::TypeAscription { expr, .. } => {
            check_at_infer(&expr.node, type_env, dispatch_table)
        }

        // Grouping — transparente.
        TypedExprKind::Grouping { inner } => check_at_infer(&inner.node, type_env, dispatch_table),

        // Tuple — comptime se todos elementos são.
        TypedExprKind::Tuple { elements } => elements
            .iter()
            .all(|e| check_at_infer(&e.node, type_env, dispatch_table)),

        // StructConstruct — comptime se todos valores são.
        TypedExprKind::StructConstruct { values, .. } => values
            .iter()
            .all(|v| check_at_infer(&v.node, type_env, dispatch_table)),

        // VariantConstruct — comptime se payload é.
        TypedExprKind::VariantConstruct { payload, .. } => {
            check_at_infer(&payload.node, type_env, dispatch_table)
        }

        // ListLit, ArrayLit — comptime se todos elementos são.
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => elements
            .iter()
            .all(|e| check_at_infer(&e.node, type_env, dispatch_table)),

        // RangeLit — comptime se start, step, end são.
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            check_at_infer(&start.node, type_env, dispatch_table)
                && check_at_infer(&step.node, type_env, dispatch_table)
                && check_at_infer(&end.node, type_env, dispatch_table)
        }

        // Match — comptime se scrutinee e todos arms body são.
        TypedExprKind::Match { scrutinee, arms } => {
            check_at_infer(&scrutinee.node, type_env, dispatch_table)
                && arms.iter().all(|arm| {
                    let guard_ok = match &arm.guard {
                        Some(g) => check_at_infer(&g.node, type_env, dispatch_table),
                        None => true,
                    };
                    guard_ok && check_at_infer(&arm.body.node, type_env, dispatch_table)
                })
        }

        // FieldAccess — comptime se expr é.
        TypedExprKind::FieldAccess { expr, .. } => {
            check_at_infer(&expr.node, type_env, dispatch_table)
        }

        // IndexAccess — comptime se expr é.
        TypedExprKind::IndexAccess { expr, .. } => {
            check_at_infer(&expr.node, type_env, dispatch_table)
        }

        // DictLit — comptime se todas as chaves e valores são.
        TypedExprKind::DictLit { entries, .. } => entries.iter().all(|(k, v)| {
            check_at_infer(&k.node, type_env, dispatch_table)
                && check_at_infer(&v.node, type_env, dispatch_table)
        }),

        // SetLit — comptime se todos elementos são.
        TypedExprKind::SetLit { elements, .. } => elements
            .iter()
            .all(|e| check_at_infer(&e.node, type_env, dispatch_table)),

        // Everything else — NÃO comptime-available por padrão.
        _ => false,
    }
}

/// Verifica que uma expressão é pura (sem efeitos colaterais).
///
/// Percorre a TAST procurando nós impuros:
/// - `ActionCall` — chama uma Action (impura por definição)
/// - `Fork` — spawn de fiber (impuro)
/// - `ChannelSend` — envio por canal (impuro)
/// - `ChannelRecv` — recebimento de canal (impuro)
/// - `Select` — select de canais (impuro)
/// - `ChannelCreate` — criação de canal (impuro)
/// - `ReceiverFactoryCall` — pede receiver (impuro)
/// - `Var` — binding mutável (impuro em Action)
/// - `Reassign` — reatribuição (impuro)
/// - `Return` — early return (impuro em Action)
/// - `Loop`/`Break`/`Continue` — controle de fluxo de Action
/// - `ForIn` — iteração (impuro em Action)
pub(crate) fn check_purity(
    name: &str,
    expr: &TypedExpr,
    span: kata_ast::Span,
) -> Result<(), MiddleError> {
    check_purity_inner(name, expr, span)
}

fn check_purity_inner(
    name: &str,
    expr: &TypedExpr,
    span: kata_ast::Span,
) -> Result<(), MiddleError> {
    match &expr.kind {
        // Nós impuros — falham imediatamente.
        TypedExprKind::ActionCall { callee, .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: format!("contém ActionCall `{callee}`"),
            span: span.into(),
        }),
        TypedExprKind::Fork { action_name, .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: format!("contém Fork `{action_name}`"),
            span: span.into(),
        }),
        TypedExprKind::Spawn { action_name, .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: format!("contém Spawn `{action_name}`"),
            span: span.into(),
        }),
        TypedExprKind::ChannelSend { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém ChannelSend".into(),
            span: span.into(),
        }),
        TypedExprKind::ChannelRecv { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém ChannelRecv".into(),
            span: span.into(),
        }),
        TypedExprKind::Select { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Select".into(),
            span: span.into(),
        }),
        TypedExprKind::ChannelCreate { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém ChannelCreate".into(),
            span: span.into(),
        }),
        TypedExprKind::ReceiverFactoryCall { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém ReceiverFactoryCall".into(),
            span: span.into(),
        }),
        TypedExprKind::Var { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Var (binding mutável)".into(),
            span: span.into(),
        }),
        TypedExprKind::Reassign { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Reassign".into(),
            span: span.into(),
        }),
        TypedExprKind::Return(_) => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Return".into(),
            span: span.into(),
        }),
        TypedExprKind::Loop { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Loop".into(),
            span: span.into(),
        }),
        TypedExprKind::Break => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Break".into(),
            span: span.into(),
        }),
        TypedExprKind::Continue => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém Continue".into(),
            span: span.into(),
        }),
        TypedExprKind::ForIn { .. } => Err(MiddleError::ImpureConstant {
            name: name.to_string(),
            reason: "contém ForIn".into(),
            span: span.into(),
        }),

        // Nós puros — recursão nos filhos.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::BytesLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. } => Ok(()),

        TypedExprKind::Closure { callee, args, .. } => {
            check_purity_inner(name, &callee.node, span)?;
            for arg in args {
                check_purity_inner(name, &arg.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::TypeAscription { expr, .. } => check_purity_inner(name, &expr.node, span),
        TypedExprKind::TypeOf { expr } => check_purity_inner(name, &expr.node, span),
        TypedExprKind::Grouping { inner } => check_purity_inner(name, &inner.node, span),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                check_purity_inner(name, &el.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for v in values {
                check_purity_inner(name, &v.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::FieldAccess { expr, .. } => check_purity_inner(name, &expr.node, span),
        TypedExprKind::IndexAccess { expr, .. } => check_purity_inner(name, &expr.node, span),
        TypedExprKind::Let { value, .. } => check_purity_inner(name, &value.node, span),
        TypedExprKind::LetDestruct {
            value, bindings, ..
        } => {
            check_purity_inner(name, &value.node, span)?;
            for (_, b) in bindings {
                check_purity_inner(name, &b.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::VariantConstruct { payload, .. } => {
            check_purity_inner(name, &payload.node, span)
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                for guard in &clause.guards {
                    if let Some(cond) = &guard.condition {
                        check_purity_inner(name, &cond.node, span)?;
                    }
                    check_purity_inner(name, &guard.body.node, span)?;
                }
                for wb in &clause.with_bindings {
                    check_purity_inner(name, &wb.value.node, span)?;
                }
                if clause.guards.is_empty() {
                    check_purity_inner(name, &clause.body.node, span)?;
                }
            }
            Ok(())
        }
        TypedExprKind::Match { scrutinee, arms } => {
            check_purity_inner(name, &scrutinee.node, span)?;
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_purity_inner(name, &g.node, span)?;
                }
                check_purity_inner(name, &arm.body.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::HeapSnapshot { .. } => Ok(()),
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                check_purity_inner(name, &el.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (k, v) in entries {
                check_purity_inner(name, &k.node, span)?;
                check_purity_inner(name, &v.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                check_purity_inner(name, &el.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            check_purity_inner(name, &start.node, span)?;
            check_purity_inner(name, &step.node, span)?;
            check_purity_inner(name, &end.node, span)?;
            Ok(())
        }
        TypedExprKind::Map {
            callback,
            collection,
            ..
        }
        | TypedExprKind::Filter {
            callback,
            collection,
            ..
        } => {
            check_purity_inner(name, &callback.node, span)?;
            check_purity_inner(name, &collection.node, span)?;
            Ok(())
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            check_purity_inner(name, &callback.node, span)?;
            check_purity_inner(name, &initial.node, span)?;
            check_purity_inner(name, &collection.node, span)?;
            Ok(())
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            check_purity_inner(name, &source.node, span)?;
            for stage in stages {
                let cb = match stage {
                    crate::FusedStage::Filter { callback, .. }
                    | crate::FusedStage::Map { callback, .. } => callback,
                };
                check_purity_inner(name, &cb.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::In { item, collection } => {
            check_purity_inner(name, &item.node, span)?;
            check_purity_inner(name, &collection.node, span)?;
            Ok(())
        }
        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                check_purity_inner(name, &stmt.node, span)?;
            }
            Ok(())
        }
        TypedExprKind::ConstantBinding { value, .. } => check_purity_inner(name, &value.node, span),
    }
}
