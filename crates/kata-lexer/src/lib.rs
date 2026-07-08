//! Analisador léxico indent-sensitive.
//!
//! Converte texto fonte em `Vec<(Token, Span)>`. Emite tokens sintéticos
//! INDENT/DEDENT para o parser tratar blocos por indentação.

// Implementação vem no Fio 1.
