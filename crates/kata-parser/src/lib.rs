//! Parser recursive-descent, prefix-only (sem Pratt parsing).
//!
//! Consome `Vec<(Token, Span)>` e produz `Spanned<Expr>` (AST plana).
//! A notação prefixa elimina precedência de operadores — `+`, `soma`,
//! `fatorial` são todos identificadores tratados identicamente.

// Implementação vem no Fio 1.