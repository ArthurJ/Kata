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

/// Dados de uma chamada de construção de variante — o callee e argumentos.
///
/// Agrupa os 5 parâmetros que descrevem a chamada (`Enum::Variant(args)`),
/// reduzindo a aridade de `infer_variant_construct` de 8 para 4.
pub(crate) struct VariantCall<'a> {
    pub enum_name: &'a str,
    pub variant: &'a str,
    pub module_path: Option<&'a [String]>,
    pub args: &'a [Spanned<Expr>],
    pub span: &'a Span,
}

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
    call: &VariantCall,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    expected_ty: Option<&Ty>,
) -> InferResult<(Ty, TypedExprKind)> {
    use kata_core::ty::Ty;

    let enum_name = call.enum_name;
    let variant = call.variant;
    let module_path = call.module_path;
    let args = call.args;
    let span = call.span;

    // Resolve origin from module_path for qualified lookups.
    let origin: Option<&str> = if let Some(path) = module_path
        && let Some(first) = path.first()
    {
        Some(first.as_str())
    } else {
        None
    };

    // Verifica que o enum e a variante existem.
    let variant_exists = if let Some(o) = origin {
        ctx.enum_registry
            .is_variant_with_origin(o, enum_name, variant)
    } else {
        ctx.enum_registry.is_variant(enum_name, variant)
    };
    if !variant_exists {
        return Err(MiddleError::UnboundName {
            name: format!("{}::{}", enum_name, variant),
            span: (*span).into(),
        });
    }

    // Variante constante: não aceita argumentos — o valor é fixo.
    let fixed = if let Some(o) = origin {
        ctx.enum_registry
            .fixed_value_with_origin(o, enum_name, variant)
    } else {
        ctx.enum_registry.fixed_value(enum_name, variant)
    };
    if fixed.is_some() {
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
    let payload_ty = if let Some(o) = origin {
        ctx.enum_registry
            .payload_ty_with_origin(o, enum_name, variant)
    } else {
        ctx.enum_registry.payload_ty(enum_name, variant)
    }
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
    let is_generic = if let Some(o) = origin {
        ctx.enum_registry.is_generic_with_origin(o, enum_name)
    } else {
        ctx.enum_registry.is_generic(enum_name)
    };
    if is_generic {
        let type_params = if let Some(o) = origin {
            ctx.enum_registry.type_params_of_with_origin(o, enum_name)
        } else {
            ctx.enum_registry.type_params_of(enum_name)
        }
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

        // Preenche type params não-inferidos pelo payload usando expected_ty.
        // Inferência bidirecional top-down: se o contexto (assinatura da função,
        // hint de retorno) conhece o tipo completo, os params que a variante não
        // menciona são preenchidos pelo expected.
        if let Some(Ty::Generic(exp_name, exp_args)) = expected_ty
            && exp_name == enum_name
            && exp_args.len() == type_args.len()
        {
            for (i, arg) in type_args.iter_mut().enumerate() {
                if matches!(arg, Ty::Var(_)) {
                    *arg = exp_args[i].clone();
                }
            }
        }

        // Preenche type params não-inferidos com defaults do EnumRegistry.
        // Ex: Result com default E|Text. Se E ainda é Var("E") após hint,
        // e o enum tem default para E, usa o default.
        let defaults = if let Some(o) = origin {
            ctx.enum_registry.defaults_of_with_origin(o, enum_name)
        } else {
            ctx.enum_registry.defaults_of(enum_name)
        };
        if let Some(defaults) = defaults {
            for (i, arg) in type_args.iter_mut().enumerate() {
                if matches!(arg, Ty::Var(_))
                    && let Some(Some(default_ty)) = defaults.get(i)
                {
                    *arg = default_ty.clone();
                }
            }
        }

        let result_ty = Ty::Generic(enum_name.to_string(), type_args);
        let tag = if let Some(o) = origin {
            ctx.enum_registry
                .variant_index_with_origin(o, enum_name, variant)
        } else {
            ctx.enum_registry.variant_index(enum_name, variant)
        }
        .unwrap_or(0);
        return Ok((
            result_ty,
            TypedExprKind::VariantConstruct {
                enum_name: enum_name.to_string(),
                variant: variant.to_string(),
                payload: Box::new(Spanned::new(typed_arg, args[0].span)),
                tag,
                module_path: module_path.map(|p| p.to_vec()),
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

    let tag = if let Some(o) = origin {
        ctx.enum_registry
            .variant_index_with_origin(o, enum_name, variant)
    } else {
        ctx.enum_registry.variant_index(enum_name, variant)
    }
    .unwrap_or(0);
    Ok((
        Ty::Sum(enum_name.to_string()),
        TypedExprKind::VariantConstruct {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            payload: Box::new(Spanned::new(typed_arg, args[0].span)),
            tag,
            module_path: module_path.map(|p| p.to_vec()),
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
