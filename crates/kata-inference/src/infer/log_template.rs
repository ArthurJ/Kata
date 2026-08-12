//! Parsing de template de log — extraído de `log_synthesis.rs`.
//!
//! Funções compartilhadas entre `log_synthesis.rs` (diretiva `@log`) e
//! `log_builtins.rs` (builtin `log!()`). Ambos precisam parsear templates
//! com placeholders `{expr}` e escape `{{`/`}}`.

use kata_ast::{Expr, Span, Spanned};

/// Resultado do parse de template: (template_com_placeholders, lista_de_exprs).
///
/// `"processando {x}, resultado: {y}"` → `("processando {}, resultado: {}", ["x", "y"])`
/// `"literal {{ escapado }}"` → `("literal { escapado }", [])`
pub(crate) fn parse_template(msg: &str) -> Result<(String, Vec<String>), String> {
    let mut template = String::new();
    let mut placeholders = Vec::new();
    let chars: Vec<char> = msg.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '{' => {
                if i + 1 < chars.len() && chars[i + 1] == '{' {
                    // {{ → { literal
                    template.push('{');
                    i += 2;
                } else {
                    // {expr} → placeholder
                    let mut expr = String::new();
                    i += 1; // consome {
                    while i < chars.len() && chars[i] != '}' {
                        expr.push(chars[i]);
                        i += 1;
                    }
                    if i >= chars.len() {
                        return Err(format!("template: {{ sem }} correspondente em \"{msg}\""));
                    }
                    i += 1; // consome }
                    let expr = expr.trim().to_string();
                    if expr.is_empty() {
                        return Err(format!("template: placeholder vazio {{}} em \"{msg}\""));
                    }
                    template.push_str("{}");
                    placeholders.push(expr);
                }
            }
            '}' => {
                if i + 1 < chars.len() && chars[i + 1] == '}' {
                    // }} → } literal
                    template.push('}');
                    i += 2;
                } else {
                    // } sem {{ correspondente — erro
                    return Err(format!("template: }} sem {{ correspondente em \"{msg}\""));
                }
            }
            c => {
                template.push(c);
                i += 1;
            }
        }
    }

    Ok((template, placeholders))
}

/// Constrói `Expr` a partir de um placeholder string.
///
/// MVP: só `Ident` simples. Se contém `.`, constrói `DotAccess`.
/// `{x}` → `Expr::Ident("x")`
/// `{foo.bar}` → `Expr::DotAccess { expr: Ident("foo"), index: Field("bar") }`
pub(crate) fn parse_placeholder(ph: &str) -> Result<Expr, String> {
    if ph.contains('.') {
        let parts: Vec<&str> = ph.splitn(2, '.').collect();
        if parts.len() != 2 || parts[1].is_empty() {
            return Err(format!("placeholder inválido: {ph}"));
        }
        Ok(Expr::DotAccess {
            expr: Box::new(Spanned::new(
                Expr::Ident {
                    name: parts[0].trim().to_string(),
                },
                Span::synthetic(),
            )),
            index: kata_ast::DotIndex::Field(parts[1].trim().to_string()),
        })
    } else {
        Ok(Expr::Ident {
            name: ph.to_string(),
        })
    }
}

/// Mapeia tag i64 de LogLevel para string legível.
///
/// Usado por `log!()` e `@log` para resolver `{log_level}`.
/// Tag 0 → "Debug", 1 → "Info", 2 → "Warn", 3 → "Error".
pub(crate) fn log_level_name(tag: i64) -> &'static str {
    match tag {
        0 => "Debug",
        1 => "Info",
        2 => "Warn",
        3 => "Error",
        _ => "Unknown",
    }
}
