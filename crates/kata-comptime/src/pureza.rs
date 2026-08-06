//! Verificação de pureza — uma expressão é pura se não contém efeitos colaterais.
//!
//! Uma expressão `@comptime` deve ser pura: não pode conter `ActionCall`,
//! `Fork`, `ChannelSend`, `ChannelRecv`, `Select`, ou qualquer nó que produza
//! efeitos observáveis. Funções puras podem chamar outras funções puras;
//! a verificação é transitiva (mas para a Fase 1, verificamos apenas a
//! expressão direta — a transitividade será adicionada quando o call graph
//! de funções puras for implementado).

use kata_inference::{TypedExpr, TypedExprKind};

use crate::ComptimeError;

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
pub(crate) fn check_purity(expr: &TypedExpr) -> Result<(), ComptimeError> {
    check_purity_inner(expr)
}

fn check_purity_inner(expr: &TypedExpr) -> Result<(), ComptimeError> {
    match &expr.kind {
        // Nós impuros — falham imediatamente.
        TypedExprKind::ActionCall { callee, .. } => Err(ComptimeError::Impure {
            reason: format!("contém ActionCall `{callee}`"),
        }),
        TypedExprKind::Fork { action_name, .. } => Err(ComptimeError::Impure {
            reason: format!("contém Fork `{action_name}`"),
        }),
        TypedExprKind::Spawn { action_name, .. } => Err(ComptimeError::Impure {
            reason: format!("contém Spawn `{action_name}`"),
        }),
        TypedExprKind::ChannelSend { .. } => Err(ComptimeError::Impure {
            reason: "contém ChannelSend".into(),
        }),
        TypedExprKind::ChannelRecv { .. } => Err(ComptimeError::Impure {
            reason: "contém ChannelRecv".into(),
        }),
        TypedExprKind::Select { .. } => Err(ComptimeError::Impure {
            reason: "contém Select".into(),
        }),
        TypedExprKind::ChannelCreate { .. } => Err(ComptimeError::Impure {
            reason: "contém ChannelCreate".into(),
        }),
        TypedExprKind::ReceiverFactoryCall { .. } => Err(ComptimeError::Impure {
            reason: "contém ReceiverFactoryCall".into(),
        }),
        TypedExprKind::Var { .. } => Err(ComptimeError::Impure {
            reason: "contém Var (binding mutável)".into(),
        }),
        TypedExprKind::Reassign { .. } => Err(ComptimeError::Impure {
            reason: "contém Reassign".into(),
        }),
        TypedExprKind::Return(_) => Err(ComptimeError::Impure {
            reason: "contém Return".into(),
        }),
        TypedExprKind::Loop { .. } => Err(ComptimeError::Impure {
            reason: "contém Loop".into(),
        }),
        TypedExprKind::Break => Err(ComptimeError::Impure {
            reason: "contém Break".into(),
        }),
        TypedExprKind::Continue => Err(ComptimeError::Impure {
            reason: "contém Continue".into(),
        }),
        TypedExprKind::ForIn { .. } => Err(ComptimeError::Impure {
            reason: "contém ForIn".into(),
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
            check_purity_inner(&callee.node)?;
            for arg in args {
                check_purity_inner(&arg.node)?;
            }
            Ok(())
        }
        TypedExprKind::TypeAscription { expr, .. } => check_purity_inner(&expr.node),
        TypedExprKind::TypeOf { expr } => check_purity_inner(&expr.node),
        TypedExprKind::Grouping { inner } => check_purity_inner(&inner.node),
        TypedExprKind::Tuple { elements } => {
            for el in elements {
                check_purity_inner(&el.node)?;
            }
            Ok(())
        }
        TypedExprKind::StructConstruct { values, .. } => {
            for v in values {
                check_purity_inner(&v.node)?;
            }
            Ok(())
        }
        TypedExprKind::FieldAccess { expr, .. } => check_purity_inner(&expr.node),
        TypedExprKind::IndexAccess { expr, .. } => check_purity_inner(&expr.node),
        TypedExprKind::Let { value, .. } => check_purity_inner(&value.node),
        TypedExprKind::LetDestruct {
            value, bindings, ..
        } => {
            check_purity_inner(&value.node)?;
            for (_, b) in bindings {
                check_purity_inner(&b.node)?;
            }
            Ok(())
        }
        TypedExprKind::VariantConstruct { payload, .. } => check_purity_inner(&payload.node),
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses {
                for guard in &clause.guards {
                    if let Some(cond) = &guard.condition {
                        check_purity_inner(&cond.node)?;
                    }
                    check_purity_inner(&guard.body.node)?;
                }
                for wb in &clause.with_bindings {
                    check_purity_inner(&wb.value.node)?;
                }
                if clause.guards.is_empty() {
                    check_purity_inner(&clause.body.node)?;
                }
            }
            Ok(())
        }
        TypedExprKind::Match { scrutinee, arms } => {
            check_purity_inner(&scrutinee.node)?;
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_purity_inner(&g.node)?;
                }
                check_purity_inner(&arm.body.node)?;
            }
            Ok(())
        }
        TypedExprKind::Comptime { expr } => check_purity_inner(&expr.node),
        // HeapSnapshot — puro (já avaliado, é um literal).
        TypedExprKind::HeapSnapshot { .. } => Ok(()),
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements {
                check_purity_inner(&el.node)?;
            }
            Ok(())
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (k, v) in entries {
                check_purity_inner(&k.node)?;
                check_purity_inner(&v.node)?;
            }
            Ok(())
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements {
                check_purity_inner(&el.node)?;
            }
            Ok(())
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            check_purity_inner(&start.node)?;
            check_purity_inner(&step.node)?;
            check_purity_inner(&end.node)?;
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
            check_purity_inner(&callback.node)?;
            check_purity_inner(&collection.node)?;
            Ok(())
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            check_purity_inner(&callback.node)?;
            check_purity_inner(&initial.node)?;
            check_purity_inner(&collection.node)?;
            Ok(())
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            check_purity_inner(&source.node)?;
            for stage in stages {
                let cb = match stage {
                    kata_inference::FusedStage::Filter { callback, .. }
                    | kata_inference::FusedStage::Map { callback, .. } => callback,
                };
                check_purity_inner(&cb.node)?;
            }
            Ok(())
        }
        TypedExprKind::In { item, collection } => {
            check_purity_inner(&item.node)?;
            check_purity_inner(&collection.node)?;
            Ok(())
        }
        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                check_purity_inner(&stmt.node)?;
            }
            Ok(())
        }
    }
}
