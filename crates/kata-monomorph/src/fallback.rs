//! Pass final de fallback — substitui Closures `show v` inválidas por
//! `TextLit("?")`.
//!
//! Após o fixpoint da monomorphização, pode haver Closures com
//! `ffi_symbol: None` cujo `arg_type` é `Ty::Var(_)` não resolvido (ex: braço
//! `Err` de `show_Result` quando só `Result::Ok` aparece). O braço nunca
//! executa em runtime, mas o codegen precisa de um nó válido — substitui por
//! `TextLit("?")`.

use kata_ast::Spanned;
use kata_core::ty::Ty;
use kata_inference::{FusedStage, TypedExpr, TypedExprKind, TypedReadMode, TypedSelectArm};

use crate::MonoModule;

/// Percorre todas as funções, actions, pre_entry e entry do módulo
/// monomorfizado, substituindo Closures `show v` (ffi_symbol: None, arg
/// Ty::Var não resolvido) por TextLit("?").
pub(crate) fn fallback_unresolved_show(mono: &mut MonoModule) {
    for func in &mut mono.functions {
        for clause in &mut func.clauses {
            fallback_in_expr(&mut clause.body);
            for guard in &mut clause.guards {
                if let Some(ref mut cond) = guard.condition {
                    fallback_in_expr(cond);
                }
                fallback_in_expr(&mut guard.body);
            }
            for wb in &mut clause.with_bindings {
                fallback_in_expr(&mut wb.value);
            }
            for pre in &mut clause.synthetic_pre {
                fallback_in_expr(pre);
            }
            for post in &mut clause.synthetic_post {
                fallback_in_expr(post);
            }
        }
    }
    for action in &mut mono.actions {
        for stmt in &mut action.body {
            fallback_in_expr(stmt);
        }
    }
    for expr in &mut mono.pre_entry {
        fallback_in_expr(expr);
    }
    fallback_in_expr(&mut mono.entry);
}

/// Recursão sobre um `Spanned<TypedExpr>` aplicando o fallback.
fn fallback_in_expr(expr_span: &mut Spanned<TypedExpr>) {
    let expr = &mut expr_span.node;
    match &mut expr.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            for arg in args.iter_mut() {
                fallback_in_expr(arg);
            }
            if ffi_symbol.is_none()
                && matches!(&callee.node.kind, TypedExprKind::Ident { name } if name == "show")
            {
                // Lista vazia com tipo do elemento não-resolvido (InferVar) →
                // "[]" — caso base da recursão de show de List. O braço Nil
                // imprime "[]" sem tocar no tipo do elemento.
                if args.iter().all(|a| {
                    matches!(&a.node.ty, Ty::List(inner) if matches!(inner.as_ref(), Ty::InferVar(_)))
                }) {
                    expr.kind = TypedExprKind::TextLit {
                        text: "[]".to_string(),
                    };
                    expr.ty = Ty::text();
                } else if args
                    .iter()
                    .all(|a| matches!(a.node.ty, Ty::Var(_) | Ty::InferVar(_)))
                {
                    expr.kind = TypedExprKind::TextLit {
                        text: "?".to_string(),
                    };
                    expr.ty = Ty::text();
                }
            }
        }
        TypedExprKind::TypeAscription { expr: inner, .. }
        | TypedExprKind::Grouping { inner }
        | TypedExprKind::Return(inner) => {
            fallback_in_expr(inner);
        }
        TypedExprKind::Tuple { elements }
        | TypedExprKind::StructConstruct {
            values: elements, ..
        } => {
            for elem in elements.iter_mut() {
                fallback_in_expr(elem);
            }
        }
        TypedExprKind::FieldAccess { expr: inner, .. }
        | TypedExprKind::IndexAccess { expr: inner, .. } => {
            fallback_in_expr(inner);
        }
        TypedExprKind::Let { value, .. }
        | TypedExprKind::LetDestruct { value, .. }
        | TypedExprKind::Var { value, .. }
        | TypedExprKind::Reassign { value, .. } => {
            fallback_in_expr(value);
        }
        TypedExprKind::Lambda { clauses, .. } => {
            for clause in clauses.iter_mut() {
                fallback_in_expr(&mut clause.body);
                for guard in &mut clause.guards {
                    if let Some(ref mut cond) = guard.condition {
                        fallback_in_expr(cond);
                    }
                    fallback_in_expr(&mut guard.body);
                }
                for wb in &mut clause.with_bindings {
                    fallback_in_expr(&mut wb.value);
                }
                for pre in &mut clause.synthetic_pre {
                    fallback_in_expr(pre);
                }
                for post in &mut clause.synthetic_post {
                    fallback_in_expr(post);
                }
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            fallback_in_expr(scrutinee);
            for arm in arms.iter_mut() {
                if let Some(ref mut guard) = arm.guard {
                    fallback_in_expr(guard);
                }
                fallback_in_expr(&mut arm.body);
            }
        }
        TypedExprKind::ActionCall { args, .. } => {
            fallback_in_expr(args);
        }
        TypedExprKind::TypeOf { expr } => {
            fallback_in_expr(expr);
        }
        TypedExprKind::Loop { body } => {
            for stmt in body.iter_mut() {
                fallback_in_expr(stmt);
            }
        }
        TypedExprKind::VariantConstruct { payload, .. } => {
            fallback_in_expr(payload);
        }
        TypedExprKind::ListLit { elements } | TypedExprKind::ArrayLit { elements } => {
            for el in elements.iter_mut() {
                fallback_in_expr(el);
            }
        }
        TypedExprKind::DictLit { entries, .. } => {
            for (key, val) in entries.iter_mut() {
                fallback_in_expr(key);
                fallback_in_expr(val);
            }
        }
        TypedExprKind::SetLit { elements, .. } => {
            for el in elements.iter_mut() {
                fallback_in_expr(el);
            }
        }
        TypedExprKind::RangeLit {
            start, step, end, ..
        } => {
            fallback_in_expr(start);
            fallback_in_expr(step);
            fallback_in_expr(end);
        }
        TypedExprKind::ForIn { iterable, body, .. } => {
            fallback_in_expr(iterable);
            for stmt in body.iter_mut() {
                fallback_in_expr(stmt);
            }
        }
        TypedExprKind::In { item, collection } => {
            fallback_in_expr(item);
            fallback_in_expr(collection);
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
            fallback_in_expr(callback);
            fallback_in_expr(collection);
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            ..
        } => {
            fallback_in_expr(callback);
            fallback_in_expr(initial);
            fallback_in_expr(collection);
        }
        TypedExprKind::FusedStream { stages, source, .. } => {
            fallback_in_expr(source);
            for stage in stages {
                let cb = match stage {
                    FusedStage::Filter { callback, .. } | FusedStage::Map { callback, .. } => {
                        callback
                    }
                };
                fallback_in_expr(cb);
            }
        }
        TypedExprKind::ChannelSend { channel, value } => {
            fallback_in_expr(channel);
            fallback_in_expr(value);
        }
        TypedExprKind::ChannelRecv { channel, .. } => {
            fallback_in_expr(channel);
        }
        TypedExprKind::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                match arm {
                    TypedSelectArm::Channel { channel, body, .. } => {
                        fallback_in_expr(channel);
                        fallback_in_expr(body);
                    }
                    TypedSelectArm::IoRead {
                        handle_expr,
                        read_mode,
                        body,
                        ..
                    } => {
                        fallback_in_expr(handle_expr);
                        if let TypedReadMode::Chunk(chunk_size_expr) = read_mode {
                            fallback_in_expr(chunk_size_expr);
                        }
                        fallback_in_expr(body);
                    }
                }
            }
            if let Some(tm) = timeout_ms {
                fallback_in_expr(tm);
            }
            if let Some(tb) = timeout_body {
                fallback_in_expr(tb);
            }
        }
        TypedExprKind::Fork {
            action_expr, args, ..
        } => {
            fallback_in_expr(action_expr);
            fallback_in_expr(args);
        }
        TypedExprKind::Spawn {
            action_expr, args, ..
        } => {
            fallback_in_expr(action_expr);
            fallback_in_expr(args);
        }
        TypedExprKind::ReceiverFactoryCall { factory, .. } => {
            fallback_in_expr(factory);
        }
        // Folhas — sem sub-expressões.
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::BytesLit { .. }
        | TypedExprKind::Unit
        | TypedExprKind::Ident { .. }
        | TypedExprKind::VariantQual { .. }
        | TypedExprKind::Break
        | TypedExprKind::Continue
        | TypedExprKind::ChannelCreate { .. } => {}

        // HeapSnapshot — folha.
        TypedExprKind::HeapSnapshot { .. } => {}
        // Block — recursão em cada stmt.
        TypedExprKind::Block { stmts } => {
            for stmt in stmts {
                fallback_in_expr(stmt);
            }
        }
        // ConstantBinding — recursão no value.
        TypedExprKind::ConstantBinding { value, .. } => {
            fallback_in_expr(value);
        }
    }
}
