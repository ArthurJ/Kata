//! Helpers compartilhados dos testes de desugar — verificação de que a AST
//! pós-desugar não contém `Expr::Hole` ou `Expr::Pipe` em qualquer profundidade.
//!
//! Extrato de `desugar_test.rs` por responsabilidade: o traversal recursivo
//! sobre `Expr` é uma unidade coesa (uma árvore, dois predicados) e reusável
//! por outros test files que queiram validar invariantes do desugar.

use kata_ast::{Expr, Spanned};

/// Verifica que a AST não contém nenhum `Expr::Hole` em qualquer profundidade.
pub fn assert_no_holes(expr: &Spanned<Expr>) {
    match &expr.node {
        Expr::Hole => panic!("AST contém Hole após desugar: {expr:?}"),
        Expr::Apply { callee, args } => {
            assert_no_holes(callee);
            args.iter().for_each(assert_no_holes);
        }
        Expr::Let { value, .. } | Expr::LetDestruct { value, .. } => assert_no_holes(value),
        Expr::Lambda {
            body,
            guards,
            with_bindings,
            ..
        } => {
            assert_no_holes(body);
            guards.iter().for_each(|g| {
                if let Some(c) = &g.condition {
                    assert_no_holes(c);
                }
                assert_no_holes(&g.body);
            });
            with_bindings.iter().for_each(|w| assert_no_holes(&w.value));
        }
        Expr::Match { scrutinee, arms } => {
            assert_no_holes(scrutinee);
            arms.iter().for_each(|arm| {
                if let Some(g) = &arm.guard {
                    assert_no_holes(g);
                }
                assert_no_holes(&arm.body);
            });
        }
        Expr::TypeAscription { expr, .. } => assert_no_holes(expr),
        Expr::Grouping { inner } => assert_no_holes(inner),
        Expr::Tuple { elements } => elements.iter().for_each(assert_no_holes),
        Expr::Pipe { lhs: _, rhs: _ } => {
            // Pipe também não deve existir após desugar
            panic!("AST contém Pipe após desugar: {expr:?}");
        }
        Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::Ident { .. }
        | Expr::Unit
        | Expr::VariantQual { .. }
        | Expr::Break
        | Expr::Continue => {}
        // Novos nós: recursão nos filhos
        Expr::ActionCall { args, .. } => assert_no_holes(args),
        Expr::Return(inner) => assert_no_holes(inner),
        Expr::Loop { body } => body.iter().for_each(assert_no_holes),
        Expr::Var { value, .. } => assert_no_holes(value),
        Expr::Reassign { value, .. } => assert_no_holes(value),
        Expr::Question(inner) => assert_no_holes(inner),
        Expr::PipeFallback { lhs, rhs } => {
            assert_no_holes(lhs);
            assert_no_holes(rhs);
        }
        Expr::DotAccess { expr, .. } => assert_no_holes(expr),
        Expr::Spread => {}
        // Coleções: recursão nos elementos
        Expr::ListLit { elements } | Expr::ArrayLit { elements } => {
            elements.iter().for_each(assert_no_holes)
        }
        Expr::RangeLit {
            start, step, end, ..
        } => {
            assert_no_holes(start);
            assert_no_holes(step);
            assert_no_holes(end);
        }
        // ForIn e In
        Expr::ForIn { iterable, body, .. } => {
            assert_no_holes(iterable);
            body.iter().for_each(assert_no_holes);
        }
        Expr::In { item, collection } => {
            assert_no_holes(item);
            assert_no_holes(collection);
        }
        // Nós CSP não contêm holes, recursam nos filhos
        Expr::ChannelSend { channel, value } => {
            assert_no_holes(channel);
            assert_no_holes(value);
        }
        Expr::ChannelRecv { channel, .. } => {
            assert_no_holes(channel);
        }
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                assert_no_holes(&arm.channel);
                assert_no_holes(&arm.body);
            }
            if let Some(ms) = timeout_ms {
                assert_no_holes(ms);
            }
            if let Some(b) = timeout_body {
                assert_no_holes(b);
            }
        }
    }
}

/// Verifica que a AST não contém nenhum `Expr::Pipe` em qualquer profundidade.
pub fn assert_no_pipes(expr: &Spanned<Expr>) {
    match &expr.node {
        Expr::Pipe { lhs: _, rhs: _ } => {
            panic!("AST contém Pipe após desugar: {expr:?}");
        }
        Expr::Apply { callee, args } => {
            assert_no_pipes(callee);
            args.iter().for_each(assert_no_pipes);
        }
        Expr::Let { value, .. } | Expr::LetDestruct { value, .. } => assert_no_pipes(value),
        Expr::Lambda {
            body,
            guards,
            with_bindings,
            ..
        } => {
            assert_no_pipes(body);
            guards.iter().for_each(|g| {
                if let Some(c) = &g.condition {
                    assert_no_pipes(c);
                }
                assert_no_pipes(&g.body);
            });
            with_bindings.iter().for_each(|w| assert_no_pipes(&w.value));
        }
        Expr::Match { scrutinee, arms } => {
            assert_no_pipes(scrutinee);
            arms.iter().for_each(|arm| {
                if let Some(g) = &arm.guard {
                    assert_no_pipes(g);
                }
                assert_no_pipes(&arm.body);
            });
        }
        Expr::TypeAscription { expr, .. } => assert_no_pipes(expr),
        Expr::Grouping { inner } => assert_no_pipes(inner),
        Expr::Tuple { elements } => elements.iter().for_each(assert_no_pipes),
        Expr::Hole
        | Expr::IntLit { .. }
        | Expr::FloatLit { .. }
        | Expr::TextLit { .. }
        | Expr::Ident { .. }
        | Expr::Unit
        | Expr::VariantQual { .. }
        | Expr::Break
        | Expr::Continue => {}
        // Novos nós: recursão nos filhos
        Expr::ActionCall { args, .. } => assert_no_pipes(args),
        Expr::Return(inner) => assert_no_pipes(inner),
        Expr::Loop { body } => body.iter().for_each(assert_no_pipes),
        Expr::Var { value, .. } => assert_no_pipes(value),
        Expr::Reassign { value, .. } => assert_no_pipes(value),
        Expr::Question(inner) => assert_no_pipes(inner),
        Expr::PipeFallback { lhs, rhs } => {
            assert_no_pipes(lhs);
            assert_no_pipes(rhs);
        }
        Expr::DotAccess { expr, .. } => assert_no_pipes(expr),
        Expr::Spread => {}
        // Coleções: recursão nos elementos
        Expr::ListLit { elements } | Expr::ArrayLit { elements } => {
            elements.iter().for_each(assert_no_pipes)
        }
        Expr::RangeLit {
            start, step, end, ..
        } => {
            assert_no_pipes(start);
            assert_no_pipes(step);
            assert_no_pipes(end);
        }
        // ForIn e In
        Expr::ForIn { iterable, body, .. } => {
            assert_no_pipes(iterable);
            body.iter().for_each(assert_no_pipes);
        }
        Expr::In { item, collection } => {
            assert_no_pipes(item);
            assert_no_pipes(collection);
        }
        // Nós CSP não contêm pipes, recursam nos filhos
        Expr::ChannelSend { channel, value } => {
            assert_no_pipes(channel);
            assert_no_pipes(value);
        }
        Expr::ChannelRecv { channel, .. } => {
            assert_no_pipes(channel);
        }
        Expr::Select {
            arms,
            timeout_ms,
            timeout_body,
        } => {
            for arm in arms {
                assert_no_pipes(&arm.channel);
                assert_no_pipes(&arm.body);
            }
            if let Some(ms) = timeout_ms {
                assert_no_pipes(ms);
            }
            if let Some(b) = timeout_body {
                assert_no_pipes(b);
            }
        }
    }
}
