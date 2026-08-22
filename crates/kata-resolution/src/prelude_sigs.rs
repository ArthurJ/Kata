//! Prelude — carrega `stdlib/core.kata` + `stdlib/core_internals.kata`.
//!
//! Substitui o catálogo hardcoded por parse+resolve do prelude
//! em Kata. Os arquivos da stdlib são embutidos no binário via
//! `include_str!` e processados pelo mesmo pipeline lex→parse→resolve
//! que qualquer módulo do usuário.
//!
//! `core_internals` contém funções internas (divisão unchecked) que o
//! core importa mas não deveriam ser visíveis para o usuário. Hoje, o
//! merge manual no `load_prelude` traz tudo — quando o import implícito
//! via `stdlib/mod.kata` substituir este arquivo, `filter_exports` vai
//! esconder `core_internals` automaticamente. (Ver TODO.)

use kata_lexer::lex;
use kata_parser::parse;

use crate::{merge_two, resolve_with_origin};

/// Código fonte do prelude, embutido no binário em tempo de compilação.
const PRELUDE_SOURCE: &str = include_str!("../../../stdlib/core.kata");

/// Código fonte do módulo interno (divisão unchecked), embutido no binário.
const INTERNALS_SOURCE: &str = include_str!("../../../stdlib/core_internals.kata");

/// Carrega o prelude (lex → parse → resolve) e retorna o ResolvedModule.
///
/// O driver chama isto antes de resolver o módulo do usuário.
/// O ResolvedModule resultante contém:
/// - TypeEnv com tipos primitivos (Int, Float, Text, Rational, Boolean, Unit)
/// - EnumRegistry com Boolean, Result, Optional
/// - InterfaceRegistry com EQ, ORD, NUM, SHOW + implementações
/// - Signatures com todos os operadores e funções FFI
///
/// O prelude é carregado com origin `"core"` para que tipos do prelude
/// (ex: `Result`) coexistam no EnumRegistry com tipos do usuário de mesmo
/// nome (origin `"__local__"`). A qualificação `core.Result::Err` resolve
/// o enum do prelude; `Result::Err` sem qualificar é ambíguo quando o
/// usuário faz shadowing.
///
/// `core_internals` é resolvido separadamente com origin `"core_internals"`
/// e mergeado antes do core. As funções internas (`bi_div`, `f_div`,
/// `rat_div`) ficam disponíveis no prelude merged para que os corpos `div`
/// do core as encontrem no DispatchTable.
pub fn load_prelude() -> Result<crate::ResolvedModule, Vec<crate::ResolveError>> {
    // ── core_internals ──
    let internals_tokens = lex(INTERNALS_SOURCE).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("core_internals lex error: {e}"),
        }]
    })?;
    let internals_module = parse(internals_tokens).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("core_internals parse error: {e}"),
        }]
    })?;
    let internals = resolve_with_origin(&internals_module, "core_internals")?;

    // ── core ──
    let core_tokens = lex(PRELUDE_SOURCE).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("prelude lex error: {e}"),
        }]
    })?;
    let core_module = parse(core_tokens).map_err(|e| {
        vec![crate::ResolveError::UnknownFfi {
            name: format!("prelude parse error: {e}"),
        }]
    })?;
    let core = resolve_with_origin(&core_module, "core")?;

    // Merge: internals como "prelude", core como "user".
    // merge_two coloca signatures do internals primeiro, depois do core.
    // O DispatchTable do core encontra bi_div/f_div/rat_div via merge.
    Ok(merge_two(internals, core))
}
