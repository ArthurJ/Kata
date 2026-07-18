//! Prelude — carrega `stdlib/core.kata` como módulo Kata normal.
//!
//! Substitui o catálogo hardcoded por parse+resolve do prelude
//! em Kata. O arquivo `stdlib/core.kata` é embutido no binário via
//! `include_str!` e processado pelo mesmo pipeline lex→parse→resolve
//! que qualquer módulo do usuário.

use kata_lexer::lex;
use kata_parser::parse;

use crate::resolve;

/// Código fonte do prelude, embutido no binário em tempo de compilação.
const PRELUDE_SOURCE: &str = include_str!("../../../stdlib/core.kata");

/// Carrega o prelude (lex → parse → resolve) e retorna o ResolvedModule.
///
/// O driver chama isto antes de resolver o módulo do usuário.
/// O ResolvedModule resultante contém:
/// - TypeEnv com tipos primitivos (Int, Float, Text, Rational, Boolean, Unit)
/// - EnumRegistry com Boolean, Result, Optional
/// - InterfaceRegistry com EQ, ORD, NUM, SHOW + implementações
/// - Signatures com todos os operadores e funções FFI
pub fn load_prelude() -> Result<crate::ResolvedModule, Vec<crate::ResolveError>> {
    let tokens = lex(PRELUDE_SOURCE).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("prelude lex error: {e:?}"),
        }]
    })?;
    let module = parse(tokens).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("prelude parse error: {e:?}"),
        }]
    })?;
    resolve(&module)
}
