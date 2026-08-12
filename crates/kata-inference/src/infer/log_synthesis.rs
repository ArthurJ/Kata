//! Síntese de logging `@log` — processa template e produz expressão tipada.
//!
//! O resolution extrai `@log{msg: "...", when: "enter"/"exit", ...}` e produz
//! `LogSpec`. O inference chama `synthesize_log_spec` que:
//! 1. Parseia o template `msg` extraindo placeholders `{expr}`.
//! 2. Constrói `Expr::Ident(name)` para cada placeholder.
//! 3. Constrói a tupla de args e o template com `{}` no lugar de `{expr}`.
//! 4. Chama `infer_format` para produzir a cadeia de `text_replace_first`.
//! 5. Retorna `TypedLogSpec` com a expressão tipada da mensagem.

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedLogSpec};

use super::expr::InferCtx;
use super::format_synthesis::infer_format;
use super::helpers::InferResult;
use super::log_template::{log_level_name, parse_placeholder, parse_template};

/// Nomes das variantes de LogLevel e suas tags.
const LOG_LEVEL_TAGS: &[(&str, i64)] = &[("Debug", 0), ("Info", 1), ("Warn", 2), ("Error", 3)];

/// Sintetiza `Vec<TypedLogSpec>` a partir de múltiplos `LogSpec` do resolution.
///
/// Cada `LogSpec` é processado independentemente: parsear template, chamar
/// `infer_format`, produzir `TypedLogSpec`. Retorna um `Vec` com um
/// `TypedLogSpec` por `LogSpec` de entrada.
pub(crate) fn synthesize_log_specs(
    logs: &[kata_resolution::LogSpec],
    param_names: &[String],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<Vec<TypedLogSpec>> {
    let mut specs = Vec::new();
    for log in logs {
        specs.push(synthesize_log_spec(log, param_names, env, ctx)?);
    }
    Ok(specs)
}

/// Sintetiza um único `TypedLogSpec` a partir de `LogSpec` do resolution.
///
/// Processa o template `msg`, extrai placeholders, chama `infer_format`,
/// e retorna o spec tipado pronto para o codegen.
pub(crate) fn synthesize_log_spec(
    log: &kata_resolution::LogSpec,
    param_names: &[String],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<TypedLogSpec> {
    // Resolve level: nome da variante → tag (precisa antes do loop
    // de placeholders para resolver {log_level} como TextLit).
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

    // Parseia o template: extrai placeholders `{expr}`, `{{`, `}}`.
    let (template, placeholders) =
        parse_template(&log.msg).map_err(|e| MiddleError::TypeMismatch {
            expected: "template válido: {expr}, {{ escapa {".into(),
            found: e,
            span: Span::synthetic().into(),
        })?;

    // Constrói Expr para cada placeholder.
    // {log_level} → TextLit com a string do level (variável sintética).
    // Outros → Ident ou FieldAccess via parse_placeholder.
    let mut args = Vec::new();
    for ph in &placeholders {
        if ph == "log_level" {
            // {log_level} → TextLit com a string do level.
            // Resolvido aqui (não passa pelo escopo — é variável sintética).
            let level_name = log_level_name(level_tag);
            args.push(Spanned::new(
                Expr::TextLit {
                    text: level_name.to_string(),
                },
                Span::synthetic(),
            ));
        } else {
            let expr = parse_placeholder(ph).map_err(|e| MiddleError::TypeMismatch {
                expected: "placeholder válido: {expr} ou {expr.field}".into(),
                found: e,
                span: Span::synthetic().into(),
            })?;
            args.push(Spanned::new(expr, Span::synthetic()));
        }
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
    let (msg_ty, msg_kind) = infer_format(
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

    // Validação: se when: "enter", placeholders só podem referenciar params
    // (exceção: {log_level} é variável sintética, sempre válida).
    if log.when == "enter" {
        for ph in &placeholders {
            if ph == "log_level" {
                continue;
            }
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
            kind: msg_kind,
        },
        Span::synthetic(),
    );

    // Resolve file: se Some(name), inferir.
    // `file` pode ser: (1) variável File no escopo, ou (2) action 0-ary que
    // retorna File (ex: `stdout` em `import stdio.(stdout)` — `stdout!()` → File).
    // Se o nome está no DispatchTable como action, geramos ActionCall; senão, Ident.
    let file_expr = if let Some(file_name) = &log.file {
        let file_ast = if ctx.table.has_function(file_name) {
            // Action 0-ary: stdout!() → Expr::ActionCall { callee, args: () }
            Expr::ActionCall {
                callee: file_name.clone(),
                args: Box::new(Spanned::new(Expr::Unit, Span::synthetic())),
            }
        } else {
            Expr::Ident {
                name: file_name.clone(),
            }
        };
        let typed = super::expr::infer_expr(&file_ast, &Span::synthetic(), env, ctx, false)?;
        if typed.ty != Ty::File {
            return Err(MiddleError::TypeMismatch {
                expected: "File".into(),
                found: format!("{}", typed.ty),
                span: Span::synthetic().into(),
            });
        }
        Some(Spanned::new(typed, Span::synthetic()))
    } else {
        None
    };

    if log.when == "enter" {
        Ok(TypedLogSpec::Enter {
            msg_expr,
            topic: log.topic.clone(),
            file: file_expr,
            policy: log.policy.clone(),
            level: level_tag,
        })
    } else {
        // when: "exit" (já validado no resolution)
        Ok(TypedLogSpec::Exit {
            msg_expr,
            topic: log.topic.clone(),
            file: file_expr,
            policy: log.policy.clone(),
            level: level_tag,
        })
    }
}
