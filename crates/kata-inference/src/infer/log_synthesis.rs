//! Síntese de logging `@log` — processa template e produz expressão tipada.
//!
//! O resolution extrai `@log{msg: "...", when: "enter"/"exit", ...}` e produz
//! `LogSpec`. O inference chama `synthesize_log_spec` que:
//! 1. Parseia o template `msg` extraindo placeholders `{expr}`.
//! 2. Constrói `Expr::Ident(name)` para cada placeholder.
//! 3. Constrói a tupla de args e o template com `{}` no lugar de `{expr}`.
//! 4. Chama `infer_format` para produzir a cadeia de `text_replace_first`.
//! 5. Retorna `TypedLogSpec` com a expressão tipada da mensagem.

use kata_ast::{DotIndex, Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedLogSpec};

use super::expr::InferCtx;
use super::format_synthesis::infer_format;
use super::helpers::InferResult;

/// Nomes das variantes de LogLevel e suas tags.
const LOG_LEVEL_TAGS: &[(&str, i64)] = &[("Debug", 0), ("Info", 1), ("Warn", 2), ("Error", 3)];

/// Sintetiza `TypedLogSpec` a partir de `LogSpec` do resolution.
///
/// Processa o template `msg`, extrai placeholders, chama `infer_format`,
/// e retorna o spec tipado pronto para o codegen.
pub(crate) fn synthesize_log_spec(
    log: &kata_resolution::LogSpec,
    param_names: &[String],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<TypedLogSpec> {
    // Parseia o template: extrai placeholders `{expr}`, `{{`, `}}`.
    let (template, placeholders) =
        parse_template(&log.msg).map_err(|e| MiddleError::TypeMismatch {
            expected: "template válido: {expr}, {{ escapa {".into(),
            found: e,
            span: Span::synthetic().into(),
        })?;

    // Constrói Expr::Ident para cada placeholder.
    // MVP: só Ident simples. Se o placeholder contém `.`, é FieldAccess.
    let mut args = Vec::new();
    for ph in &placeholders {
        let expr = parse_placeholder(ph).map_err(|e| MiddleError::TypeMismatch {
            expected: "placeholder válido: {expr} ou {expr.field}".into(),
            found: e,
            span: Span::synthetic().into(),
        })?;
        args.push(Spanned::new(expr, Span::synthetic()));
    }

    // Constrói a chamada para `infer_format`:
    // format "template {} {}" (arg1, arg2)
    // infer_format espera args = [template_expr, tuple_expr]
    let template_expr = Expr::TextLit { text: template };
    let tuple_expr = if args.is_empty() {
        Expr::Unit
    } else if args.len() == 1 {
        // Grouping de 1 elemento — infer_format espera Tuple ou Grouping(Tuple)
        Expr::Grouping {
            inner: Box::new(Spanned::new(
                Expr::Tuple { elements: args },
                Span::synthetic(),
            )),
        }
    } else {
        Expr::Tuple { elements: args }
    };

    let format_args = vec![
        Spanned::new(template_expr, Span::synthetic()),
        Spanned::new(tuple_expr, Span::synthetic()),
    ];

    // Chama infer_format para produzir a cadeia de text_replace_first.
    let (msg_ty, msg_kind, _effect) = infer_format(
        &Spanned::new(Expr::Unit, Span::synthetic()),
        &format_args,
        &Span::synthetic(),
        env,
        ctx,
    )?;

    if msg_ty != Ty::text() {
        return Err(MiddleError::TypeMismatch {
            expected: "Text".into(),
            found: format!("{msg_ty}"),
            span: Span::synthetic().into(),
        });
    }

    // Resolve level: nome da variante → tag.
    let level = log.level.as_deref().unwrap_or("Info");
    let level_tag = LOG_LEVEL_TAGS
        .iter()
        .find(|(name, _)| *name == level)
        .map(|(_, tag)| *tag)
        .ok_or_else(|| MiddleError::TypeMismatch {
            expected: "LogLevel variant (Debug, Info, Warn, Error)".into(),
            found: level.to_string(),
            span: Span::synthetic().into(),
        })?;

    // Validação: se when: "enter", placeholders só podem referenciar params.
    if log.when == "enter" {
        for ph in &placeholders {
            let name = ph.split('.').next().unwrap_or(ph);
            if !param_names.contains(&name.to_string()) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!(
                        "when: \"enter\" só pode referenciar params [{param_names:?}], mas \"{name}\" não é param"
                    ),
                    found: format!("placeholder {{{ph}}} referencia var do corpo"),
                    span: Span::synthetic().into(),
                });
            }
        }
    }

    let msg_expr = Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty: msg_ty,
            tail_pos: false,
            escape: kata_core::escape::EscapeTarget::Local,
            effect: _effect,
            kind: msg_kind,
        },
        Span::synthetic(),
    );

    if log.when == "enter" {
        Ok(TypedLogSpec::Enter {
            msg_expr,
            topic: log.topic.clone(),
            policy: log.policy.clone(),
            level: level_tag,
        })
    } else {
        // when: "exit" (já validado no resolution)
        Ok(TypedLogSpec::Exit {
            msg_expr,
            topic: log.topic.clone(),
            policy: log.policy.clone(),
            level: level_tag,
        })
    }
}

/// Resultado do parse de template: (template_com_placeholders, lista_de_exprs).
///
/// `"processando {x}, resultado: {y}"` → `("processando {}, resultado: {}", ["x", "y"])`
/// `"literal {{ escapado }}"` → `("literal { escapado }", [])`
fn parse_template(msg: &str) -> Result<(String, Vec<String>), String> {
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
/// MVP: só `Ident` simples. Se contém `.`, constrói `FieldAccess`.
/// `{x}` → `Expr::Ident("x")`
/// `{foo.bar}` → `Expr::FieldAccess { target: Ident("foo"), field: "bar" }`
fn parse_placeholder(ph: &str) -> Result<Expr, String> {
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
