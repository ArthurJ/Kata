//! AST de dados puros — sem lógica, sem dependência do compilador.
//!
//! Define [`Span`], [`Spanned`], [`Expr`], [`Pattern`], [`TypeExpr`] e
//! estruturas de dados que o lexer/parser produzem e o typeck consome.

// Implementação vem no Fio 1.