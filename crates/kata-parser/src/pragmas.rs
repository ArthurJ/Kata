//! Pragmas — parse_pragmas: reconhece `#!<token> <payload>` do lexer
//! e produz nós do AST (`DiagnosticControl`, `TestSpec`, `UnknownPragma`).
//!
//! Tokens conhecidos do compilador (sem prefixo): `allow`, `warn`, `deny`,
//! `test`, `deprecated`, `must_use`. Tokens externos exigem prefixo
//! obrigatório `#!<prefixo>-<resto>`. Token sem prefixo não reconhecido
//! → erro.

use kata_ast::{
    DiagnosticControl, DiagnosticLevel, Pragma, TestSpec, UnknownPragma,
};
use kata_diagnostics::FrontendError;
use kata_ast::Span;

use crate::Parser;

/// Conjunto fechado de tokens do compilador (sem prefixo).
const COMPILER_TOKENS: &[&str] = &["allow", "warn", "deny", "test", "deprecated", "must_use"];

impl Parser {
    /// Coleta pragmas `#!` que aparecem antes de uma declaração
    /// (start-of-line). O lexer já produziu `Token::Pragma { token, raw }`.
    ///
    /// Retorna a lista de pragmas parseados. Consome todos os
    /// `Token::Pragma` consecutivos, pulando `StmtSep` entre eles.
    pub(crate) fn parse_pragmas(&mut self) -> Result<Vec<Pragma>, FrontendError> {
        let mut pragmas = Vec::new();
        loop {
            // Skip statement separators between stacked pragmas
            while matches!(self.peek(), kata_ast::Token::StmtSep) {
                self.advance();
            }
            let (token, raw, span) = match self.peek() {
                kata_ast::Token::Pragma { token, raw } => {
                    let t = token.clone();
                    let r = raw.clone();
                    let s = self.peek_span();
                    self.advance();
                    (t, r, s)
                }
                _ => break,
            };
            let pragma = self.dispatch_pragma(&token, &raw, span)?;
            pragmas.push(pragma);
        }
        Ok(pragmas)
    }

    /// Faz dispatch por token: conhecido → structurado, externo com
    /// prefixo → `UnknownPragma`, sem prefixo e desconhecido → erro.
    fn dispatch_pragma(
        &mut self,
        token: &str,
        raw: &str,
        span: Span,
    ) -> Result<Pragma, FrontendError> {
        match token {
            "allow" => Ok(Pragma::DiagnosticControl(DiagnosticControl {
                level: DiagnosticLevel::Allow,
                code: raw.to_string(),
                span,
            })),
            "warn" => Ok(Pragma::DiagnosticControl(DiagnosticControl {
                level: DiagnosticLevel::Warn,
                code: raw.to_string(),
                span,
            })),
            "deny" => Ok(Pragma::DiagnosticControl(DiagnosticControl {
                level: DiagnosticLevel::Deny,
                code: raw.to_string(),
                span,
            })),
            "test" => {
                let spec = self.parse_test_spec(raw, span)?;
                Ok(Pragma::TestSpec(spec))
            }
            // deprecated, must_use: aceitos no conjunto fechado mas
            // payload ainda não definido — preserva como UnknownPragma
            // temporariamente (implementação futura).
            "deprecated" | "must_use" => {
                // Para Fase 1, preserva como UnknownPragma com prefixo
                // vazio (token do compilador, não externo).
                Ok(Pragma::UnknownPragma(UnknownPragma {
                    token: token.to_string(),
                    prefix: String::new(),
                    raw: raw.to_string(),
                    span,
                }))
            }
            _ => {
                // Token não está no conjunto fechado. Verifica se tem
                // prefixo (contém `-`).
                if let Some(dash_pos) = token.find('-') {
                    let prefix = token[..dash_pos].to_string();
                    if prefix.is_empty() {
                        // `-` no início do token — inválido
                        return Err(FrontendError::UnexpectedToken {
                            expected: format!(
                                "pragma token with non-empty prefix (got `{token}`)"
                            ),
                            found: format!("#!{token} {raw}"),
                            span: span.into(),
                        });
                    }
                    Ok(Pragma::UnknownPragma(UnknownPragma {
                        token: token.to_string(),
                        prefix,
                        raw: raw.to_string(),
                        span,
                    }))
                } else {
                    // Sem prefixo e não é token do compilador — erro
                    Err(FrontendError::UnexpectedToken {
                        expected: format!(
                            "known pragma ({}) or external pragma with prefix (e.g. `#!mytool-{token}`)",
                            COMPILER_TOKENS.join(", ")
                        ),
                        found: format!("#!{token}"),
                        span: span.into(),
                    })
                }
            }
        }
    }

    /// Parse do payload de `#!test`: `("desc")` ou `{desc, args, timeout}`.
    /// `raw` é o texto bruto após `#!test` (já sem o token).
    /// Para a Fase 1, faz parsing simples: extrai desc de `("desc")` ou
    /// `{desc: "desc", ...}`. O parsing completo de args/timeout vem na
    /// Fase 3 (migração @test → #!test).
    fn parse_test_spec(&mut self, raw: &str, span: Span) -> Result<TestSpec, FrontendError> {
        // Fase 1: parsing minimal — extrai descrição de ("desc") ou
        // {desc: "desc", ...}. Args e timeout ficam vazios/default.
        // O payload real (args, timeout) será parseado na Fase 3 quando
        // a migração de @test for feita.
        let trimmed = raw.trim();

        // Forma curta: ("desc")
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            let inner = &trimmed[1..trimmed.len() - 1];
            let desc = inner.trim().trim_matches('"').to_string();
            return Ok(TestSpec {
                desc,
                args: Vec::new(),
                timeout: None,
                span,
            });
        }

        // Forma longa: {desc: "desc", args: ..., timeout: ...}
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            let inner = &trimmed[1..trimmed.len() - 1];
            // Parsing minimal: extrai desc
            let desc = extract_field(inner, "desc")
                .trim_matches('"')
                .to_string();
            let timeout = extract_field(inner, "timeout")
                .parse::<u64>()
                .ok();
            return Ok(TestSpec {
                desc,
                args: Vec::new(),
                timeout,
                span,
            });
        }

        // Sem parênteses/chaves — desc é o raw inteiro
        Ok(TestSpec {
            desc: trimmed.to_string(),
            args: Vec::new(),
            timeout: None,
            span,
        })
    }
}

/// Extrai o valor de um campo `key: value` de um payload de diretiva.
/// Busca linear simples — suficiente para Fase 1.
fn extract_field(payload: &str, key: &str) -> String {
    let pattern = format!("{key}:");
    if let Some(pos) = payload.find(&pattern) {
        let after_key = &payload[pos + pattern.len()..];
        // Encontra o próximo `,` ou fim do payload
        let end = after_key.find(',').unwrap_or(after_key.len());
        after_key[..end].trim().to_string()
    } else {
        String::new()
    }
}