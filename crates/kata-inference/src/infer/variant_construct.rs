//! Construção de variantes de enum e expansão de spread.
//!
//! Funções extraídas de `apply.rs`:
//! - `infer_variant_construct`: infere `Apply(VariantQual, [arg])` — construção de Sum
//! - `expand_spread`: expande `$` spread em argumentos de Apply

use kata_ast::{Expr, Span, Spanned};
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::TypedExprKind;

use super::expr::{InferCtx, infer_expr};
use super::helpers::InferResult;

/// Infere `Apply(VariantQual("Enum", "Variant"), [arg])` —
/// construção de Sum com payload.
///
/// Verifica que a variante existe no EnumRegistry, que tem payload,
/// e que o tipo do argumento é compatível com o tipo do payload.
/// Produz `TypedExprKind::VariantConstruct { enum_name, variant, payload }`.
///
/// Se o enum é genérico, o payload_ty pode ser `Ty::Var("T")`.
/// Nesse caso, unifica `Ty::Var("T")` com `arg.ty` → binding `T = arg.ty`.
/// Produz `Ty::Generic(enum_name, type_args)` onde type_args são os
/// type params instanciados (não-inferidos ficam como `Ty::Var`).
pub(crate) fn infer_variant_construct(
    enum_name: &str,
    variant: &str,
    args: &[Spanned<Expr>],
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<(Ty, TypedExprKind)> {
    use kata_core::ty::Ty;

    // Verifica que o enum e a variante existem.
    if !ctx.enum_registry.is_variant(enum_name, variant) {
        return Err(MiddleError::UnboundName {
            name: format!("{}::{}", enum_name, variant),
            span: (*span).into(),
        });
    }

    // Variante constante: não aceita argumentos — o valor é fixo.
    if ctx.enum_registry.fixed_value(enum_name, variant).is_some() {
        return Err(MiddleError::TypeMismatch {
            expected: format!(
                "{}::{} (variante constante — não aceita argumentos)",
                enum_name, variant
            ),
            found: format!("{}::{} tem valor fixo — use sem args", enum_name, variant),
            span: (*span).into(),
        });
    }

    // Verifica que a variante tem payload.
    let payload_ty = ctx
        .enum_registry
        .payload_ty(enum_name, variant)
        .ok_or_else(|| MiddleError::TypeMismatch {
            expected: "variante com payload".into(),
            found: format!("{}::{} é unitária", enum_name, variant),
            span: (*span).into(),
        })?;

    // Exatamente 1 argumento.
    if args.len() != 1 {
        return Err(MiddleError::ArityMismatch {
            expected: 1,
            found: args.len(),
            span: (*span).into(),
        });
    }

    // Infere o argumento (tail_pos = false — é computação local).
    let typed_arg = infer_expr(&args[0].node, &args[0].span, env, ctx, false)?;

    // Unificação com Ty::Var.
    if ctx.enum_registry.is_generic(enum_name) {
        let type_params = ctx
            .enum_registry
            .type_params_of(enum_name)
            .expect("is_generic true implica type_params_of Some");

        // Unifica payload_ty (que pode ser Ty::Var) com typed_arg.ty.
        let arg_ty = &typed_arg.ty;
        let mut type_args: Vec<Ty> = Vec::with_capacity(type_params.len());

        for param_name in type_params {
            // Se o payload_ty é Ty::Var(param_name), o arg fornece o tipo concreto.
            if payload_ty == &Ty::Var(param_name.to_string()) {
                type_args.push(arg_ty.clone());
            } else {
                // Type param não-inferido por esta variante — mantém como Ty::Var.
                type_args.push(Ty::Var(param_name.to_string()));
            }
        }

        // Verifica compatibilidade: se payload_ty é Ty::Var, aceita qualquer tipo.
        // Se payload_ty é concreto (não deveria acontecer em enum genérico, mas
        // pode se o payload não usa o type param), compara estruturalmente.
        let compatible = match payload_ty {
            Ty::Var(_) => true,
            _ => payload_ty == arg_ty,
        };
        if !compatible {
            return Err(MiddleError::TypeMismatch {
                expected: format!("{}", payload_ty),
                found: format!("{}", typed_arg.ty),
                span: args[0].span.into(),
            });
        }

        let result_ty = Ty::Generic(enum_name.to_string(), type_args);
        return Ok((
            result_ty,
            TypedExprKind::VariantConstruct {
                enum_name: enum_name.to_string(),
                variant: variant.to_string(),
                payload: Box::new(Spanned::new(typed_arg, args[0].span)),
                tag: ctx
                    .enum_registry
                    .variant_index(enum_name, variant)
                    .unwrap_or(0),
            },
        ));
    }

    // Enum não-genérico — comparação estrutural.
    if typed_arg.ty != *payload_ty {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", payload_ty),
            found: format!("{}", typed_arg.ty),
            span: args[0].span.into(),
        });
    }

    Ok((
        Ty::Sum(enum_name.to_string()),
        TypedExprKind::VariantConstruct {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            payload: Box::new(Spanned::new(typed_arg, args[0].span)),
            tag: ctx
                .enum_registry
                .variant_index(enum_name, variant)
                .unwrap_or(0),
        },
    ))
}

/// Expande `$` spread em argumentos de Apply.
///
/// `f $ (a, b)` → `f a b`. Se um arg é `Ident("$")`, o próximo arg deve ser
/// `Expr::Tuple` — substitui ambos (`$` + `Tuple`) pelos elementos individuais.
/// Se `$` não é seguido por tupla → `SpreadRequiresTuple` error.
pub(crate) fn expand_spread(
    args: &[Spanned<Expr>],
    _span: &kata_ast::Span,
) -> Result<Vec<Spanned<Expr>>, MiddleError> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < args.len() {
        // Verifica se é `Ident("$")`
        if let Expr::Ident { name } = &args[i].node
            && name == "$"
        {
            // Próximo arg deve ser Tuple
            if i + 1 >= args.len() {
                return Err(MiddleError::UnboundName {
                    name: "$ spread requires a following tuple".into(),
                    span: args[i].span.into(),
                });
            }
            match &args[i + 1].node {
                Expr::Tuple { elements } => {
                    result.extend(elements.iter().cloned());
                }
                Expr::Grouping { inner } => {
                    if let Expr::Tuple { elements } = &inner.node {
                        result.extend(elements.iter().cloned());
                    } else {
                        return Err(MiddleError::TypeMismatch {
                            expected: "Tuple".into(),
                            found: format!("{:?}", inner.node),
                            span: args[i + 1].span.into(),
                        });
                    }
                }
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: "Tuple after $".into(),
                        found: format!("{:?}", args[i + 1].node),
                        span: args[i + 1].span.into(),
                    });
                }
            }
            i += 2; // pula $ e a tupla
            continue;
        }
        result.push(args[i].clone());
        i += 1;
    }
    Ok(result)
}
