//! Pattern checking e verificação de exaustividade.
//!
//! `check_pattern` converte um `Pattern` da AST em `TypedPattern`,
//! resolvendo `Ident("True")` → `Variant` via `EnumRegistry`, e definindo
//! bindings no `TypeEnv`.
//!
//! `check_exhaustiveness` verifica se os braços cobrem todas as variantes
//! de um `Sum`, ou exige `otherwise` para tipos infinitos.

use kata_ast::{Pattern, Span, Spanned};
use kata_core::enum_registry::EnumRegistry;
use kata_core::escape::EscapeTarget;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::MiddleError;
use kata_resolution::resolve_type_expr;

use crate::typed::{TypedExpr, TypedExprKind, TypedPattern};

/// Tipo de erro de inferência — alias para `Result<T, MiddleError>`.
pub(crate) type PatternResult<T> = Result<T, MiddleError>;

/// Verifica um pattern contra o tipo do scrutinee, produzindo `TypedPattern`
/// e definindo bindings no `env`.
///
/// Resolução de variantes sem qualificação: `Pattern::Ident("True")` em
/// scrutinee `Ty::Sum("Boolean")` é resolvido para `TypedPattern::Variant`
/// se `True` é variante de `Boolean` no `EnumRegistry`. Caso contrário,
/// é tratado como binding (`Ident`).
pub(crate) fn check_pattern(
    pat: &Spanned<Pattern>,
    scrutinee_ty: &Ty,
    enum_registry: &EnumRegistry,
    env: &mut TypeEnv,
    iface_registry: &kata_core::InterfaceRegistry,
) -> PatternResult<Spanned<TypedPattern>> {
    let typed = check_pattern_inner(
        &pat.node,
        scrutinee_ty,
        enum_registry,
        env,
        &pat.span,
        iface_registry,
    )?;
    Ok(Spanned::new(typed, pat.span))
}

fn check_pattern_inner(
    pat: &Pattern,
    scrutinee_ty: &Ty,
    enum_registry: &EnumRegistry,
    env: &mut TypeEnv,
    span: &Span,
    iface_registry: &kata_core::InterfaceRegistry,
) -> PatternResult<TypedPattern> {
    match pat {
        // ── Ident: pode ser binding ou variante sem qualificação ──
        Pattern::Ident(name) => {
            // Se o scrutinee é Sum ou Generic e o nome é variante desse enum, resolve.
            let enum_name: Option<&str> = match scrutinee_ty {
                Ty::Sum(enum_name) => Some(enum_name),
                Ty::Generic(enum_name, _) => Some(enum_name),
                _ => None,
            };
            if let Some(enum_name) = enum_name
                && enum_registry.is_variant(enum_name, name)
            {
                return Ok(TypedPattern::Variant {
                    enum_name: enum_name.to_string(),
                    variant: name.clone(),
                    sub_patterns: None,
                    tag: enum_registry.variant_index(enum_name, name).unwrap_or(0),
                });
            }
            // Caso contrário, é binding. Define no escopo.
            env.define(name, scrutinee_ty.clone(), "__local__");
            Ok(TypedPattern::Ident {
                name: name.clone(),
                ty: scrutinee_ty.clone(),
            })
        }

        // ── TypedIdent: `x::Type` — binding com type annotation ──
        // O parser produziu TypedIdent porque `snake_case::PascalCase` foi
        // disambiguado como type annotation (não Enum::Variant).
        // O typeck resolve o TypeExpr → Ty e define o binding com o tipo
        // anotado. Se o scrutinee tem tipo conhecido e difere do anotado,
        // é erro. Se o scrutinee é InferVar, o tipo anotado ajuda a inferir.
        Pattern::TypedIdent { name, ty } => {
            let annotated_ty = resolve_type_expr(&ty.node, env, iface_registry);
            // Valida compatibilidade com o scrutinee.
            match scrutinee_ty {
                Ty::InferVar(_) => {
                    // Scrutinee não tem tipo ainda — o annotation define.
                    // TODO: unificar InferVar com annotated_ty quando
                    // tivermos constraint solving. Por ora, aceitar.
                }
                _ => {
                    if !pattern_type_compatible(&annotated_ty, scrutinee_ty) {
                        return Err(MiddleError::TypeMismatch {
                            expected: format!("{}", annotated_ty),
                            found: format!("{}", scrutinee_ty),
                            span: (*span).into(),
                        });
                    }
                }
            }
            env.define(name, annotated_ty.clone(), "__local__");
            Ok(TypedPattern::Ident {
                name: name.clone(),
                ty: annotated_ty,
            })
        }

        // ── Wildcard: aceita qualquer tipo, não liga nome ──
        Pattern::Wildcard => Ok(TypedPattern::Wildcard),

        // ── Literal: verifica tipo do literal contra scrutinee ──
        Pattern::Literal(expr) => {
            // O literal precisa ser do mesmo tipo que o scrutinee.
            // Inferimosos o tipo da expr literal e comparamos.
            let literal_ty = literal_expr_ty(&expr.node, scrutinee_ty);
            if !pattern_type_compatible(&literal_ty, scrutinee_ty) {
                return Err(MiddleError::TypeMismatch {
                    expected: format!("{}", scrutinee_ty),
                    found: format!("{}", literal_ty),
                    span: (*span).into(),
                });
            }
            // Constrói TypedExpr para o literal.
            let typed_expr = TypedExpr {
                span: expr.span,
                ty: literal_ty,
                tail_pos: false,
                escape: EscapeTarget::Local,
                kind: literal_to_typed_kind(&expr.node),
            };
            Ok(TypedPattern::Literal {
                value: Spanned::new(typed_expr, expr.span),
            })
        }

        // ── Variant: verifica que enum e variante existem ──
        Pattern::Variant {
            enum_name,
            variant,
            payload,
        } => {
            // Variant desqualificada (enum_name vazio, produzido por parse_match_pattern):
            // resolver o enum_name via EnumRegistry do scrutinee.
            // `Ok v` em match sobre Result → enum_name="Result".
            let enum_name: &str = if enum_name.is_empty() {
                let scrutinee_enum = match scrutinee_ty {
                    Ty::Sum(s) => Some(s.as_str()),
                    Ty::Generic(s, _) => Some(s.as_str()),
                    _ => None,
                };
                match scrutinee_enum {
                    Some(s) if enum_registry.is_variant(s, variant) => s,
                    _ => {
                        return Err(MiddleError::UnboundName {
                                    suggestion: None,
                            name: format!(
                                "variante desqualificada `{}` — scrutinee {} não tem essa variante",
                                variant, scrutinee_ty
                            ),
                            span: (*span).into(),
                        });
                    }
                }
            } else {
                enum_name
            };
            // Verifica que o enum existe e tem a variante.
            if !enum_registry.is_variant(enum_name, variant) {
                return Err(MiddleError::UnboundName {
                            suggestion: None,
                    name: format!("{}::{}", enum_name, variant),
                    span: (*span).into(),
                });
            }
            // Verifica que o scrutinee é o enum esperado.
            // Aceita Ty::Sum (não-genérico) ou Ty::Generic (instanciado).
            let type_args: Vec<Ty> = match scrutinee_ty {
                Ty::Sum(s) if s == enum_name => Vec::new(),
                Ty::Generic(s, args) if s == enum_name => args.clone(),
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{}", scrutinee_ty),
                        found: format!("Sum({}) or Generic({})", enum_name, enum_name),
                        span: (*span).into(),
                    });
                }
            };
            //  /6: resolver sub-patterns do payload.
            let sub_patterns = if let Some(sub_pats) = payload {
                // Variante com payload — precisa ter tipo de payload no EnumRegistry.
                let payload_ty = enum_registry
                    .payload_ty(enum_name, variant)
                    .ok_or_else(|| MiddleError::TypeMismatch {
                        expected: "variante com payload".into(),
                        found: format!("{}::{} sem payload", enum_name, variant),
                        span: (*span).into(),
                    })?;
                // Se o enum é genérico, instancia o payload_ty.
                let effective_payload_ty = if enum_registry.is_generic(enum_name) {
                    enum_registry
                        .instantiate_variant(enum_name, variant, &type_args)
                        .unwrap_or_else(|| payload_ty.clone())
                } else {
                    payload_ty.clone()
                };
                // 1 sub-pattern por variante.
                if sub_pats.len() != 1 {
                    return Err(MiddleError::ArityMismatch {
                        expected: 1,
                        found: sub_pats.len(),
                        span: (*span).into(),
                        hint: Some(
                            "variantes de enum carregam exatamente 1 valor associado — use (valor) ou remova os parênteses extras".into(),
                        ),
                    });
                }
                let mut typed_subs = Vec::with_capacity(sub_pats.len());
                for sub_pat in sub_pats {
                    let typed = check_pattern_inner(
                        &sub_pat.node,
                        &effective_payload_ty,
                        enum_registry,
                        env,
                        &sub_pat.span,
                        iface_registry,
                    )?;
                    typed_subs.push(Spanned::new(typed, sub_pat.span));
                }
                Some(typed_subs)
            } else {
                // Sem sub-pattern — verifica que a variante é unitária.
                if enum_registry.payload_ty(enum_name, variant).is_some() {
                    return Err(MiddleError::TypeMismatch {
                        expected: "variante sem payload".into(),
                        found: format!("{}::{} tem payload", enum_name, variant),
                        span: (*span).into(),
                    });
                }
                None
            };
            Ok(TypedPattern::Variant {
                enum_name: enum_name.to_string(),
                variant: variant.clone(),
                sub_patterns,
                tag: enum_registry.variant_index(enum_name, variant).unwrap_or(0),
            })
        }

        // ── Tuple: verifica cada sub-pattern contra sub-tipo ──
        Pattern::Tuple(elements) => {
            let element_tys = match scrutinee_ty {
                Ty::Tuple(tys) => tys,
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{}", scrutinee_ty),
                        found: "tupla".into(),
                        span: (*span).into(),
                    });
                }
            };
            if elements.len() != element_tys.len() {
                return Err(MiddleError::ArityMismatch {
                    expected: element_tys.len(),
                    found: elements.len(),
                    span: (*span).into(),
                    hint: Some(format!(
                        "a tupla tem {expected} elemento(s) — forneça {expected} valor(es) no padrão",
                        expected = element_tys.len()
                    )),
                });
            }
            let mut typed_elements = Vec::with_capacity(elements.len());
            for (pat, ty) in elements.iter().zip(element_tys.iter()) {
                let typed = check_pattern_inner(
                    &pat.node,
                    ty,
                    enum_registry,
                    env,
                    &pat.span,
                    iface_registry,
                )?;
                typed_elements.push(Spanned::new(typed, pat.span));
            }
            Ok(TypedPattern::Tuple {
                elements: typed_elements,
            })
        }

        // ── Cons pattern — match em Ty::List(A) ──
        Pattern::Cons { head, tail } => {
            // Scrutinee deve ser Ty::List(A).
            let elem_ty = match scrutinee_ty {
                Ty::List(elem) => elem.as_ref().clone(),
                _ => {
                    return Err(MiddleError::TypeMismatch {
                        expected: "List(A) para pattern Cons [h : t]".into(),
                        found: format!("{scrutinee_ty:?}"),
                        span: (*span).into(),
                    });
                }
            };
            // head: A, tail: List(A)
            let typed_head = check_pattern_inner(
                &head.node,
                &elem_ty,
                enum_registry,
                env,
                &head.span,
                iface_registry,
            )?;
            let tail_ty = Ty::List(Box::new(elem_ty));
            let typed_tail = check_pattern_inner(
                &tail.node,
                &tail_ty,
                enum_registry,
                env,
                &tail.span,
                iface_registry,
            )?;
            Ok(TypedPattern::Cons {
                head: Box::new(Spanned::new(typed_head, head.span)),
                tail: Box::new(Spanned::new(typed_tail, tail.span)),
            })
        }

        // ── Nil pattern — match em Ty::List(A), testa val == 0 ──
        Pattern::Nil => match scrutinee_ty {
            Ty::List(_) => Ok(TypedPattern::Nil),
            _ => Err(MiddleError::TypeMismatch {
                expected: "List(A) para pattern Nil []".into(),
                found: format!("{scrutinee_ty:?}"),
                span: (*span).into(),
            }),
        },
    }
}

/// Determina o tipo de uma expressão literal em pattern.
fn literal_expr_ty(expr: &kata_ast::Expr, scrutinee_ty: &Ty) -> Ty {
    match expr {
        kata_ast::Expr::IntLit { .. } => Ty::int(),
        kata_ast::Expr::FloatLit { .. } => Ty::float(),
        kata_ast::Expr::TextLit { .. } => Ty::text(),
        kata_ast::Expr::Unit => Ty::Unit,
        // Demais expressões em pattern literal não são esperadas.
        _ => scrutinee_ty.clone(),
    }
}

/// Constrói `TypedExprKind` para um literal em pattern.
fn literal_to_typed_kind(expr: &kata_ast::Expr) -> TypedExprKind {
    match expr {
        kata_ast::Expr::IntLit { text } => TypedExprKind::IntLit { text: text.clone() },
        kata_ast::Expr::FloatLit { text } => TypedExprKind::FloatLit { text: text.clone() },
        kata_ast::Expr::TextLit { text } => TypedExprKind::TextLit { text: text.clone() },
        kata_ast::Expr::Unit => TypedExprKind::Unit,
        _ => TypedExprKind::Unit, // fallback — não deveria acontecer
    }
}

/// Verifica compatibilidade de tipo entre literal e scrutinee.
/// Int compatível com Int, Float com Float, etc. Ascription de literal
/// (Int→Float) não se aplica em patterns — o literal deve ser do mesmo tipo.
fn pattern_type_compatible(literal_ty: &Ty, scrutinee_ty: &Ty) -> bool {
    literal_ty == scrutinee_ty
}

/// Verifica exaustividade de braços de match contra o tipo do scrutinee.
///
/// - `Ty::Sum(name)`: coleta variantes cobertas. Se todas cobertas → exaustivo.
///   Se não → `NonExhaustiveMatch` com variantes faltantes.
/// - `Ty::Prim`, `Ty::Unit`, `Ty::Struct`, `Ty::Tuple`: tipos infinitos → exige
///   `otherwise` ou `Wildcard`. Retorna `MissingOtherwise` se não há fallback.
/// - `Ty::Function`, `Ty::InferVar`: não faz sentido fazer match → type error.
///
/// `has_otherwise` indica se algum braço é `otherwise` (pattern None) ou
/// `Wildcard`. Esses cobrem qualquer valor.
pub(crate) fn check_exhaustiveness(
    covered_variants: &[String],
    scrutinee_ty: &Ty,
    has_otherwise: bool,
    enum_registry: &EnumRegistry,
    span: &Span,
) -> PatternResult<()> {
    if has_otherwise {
        // otherwise/wildcard cobre tudo — sempre exaustivo.
        return Ok(());
    }

    match scrutinee_ty {
        Ty::Sum(enum_name) | Ty::Generic(enum_name, _) => {
            let all_variants = enum_registry.variants_of(enum_name);
            if all_variants.is_empty() {
                // Enum desconhecido — não há variantes para cobrir.
                // Trata como tipo infinito: exige otherwise (já checado acima).
                return Err(MiddleError::MissingOtherwise {
                    span: (*span).into(),
                });
            }
            // Coleta variantes não cobertas.
            let missing: Vec<String> = all_variants
                .iter()
                .filter(|v| !covered_variants.iter().any(|c| c == *v))
                .map(|v| v.to_string())
                .collect();
            if missing.is_empty() {
                Ok(()) // exaustivo
            } else {
                Err(MiddleError::NonExhaustiveMatch {
                    missing: missing.clone(),
                    span: (*span).into(),
                    hint: Some(format!(
                        "variantes faltantes: {}. Adicione um caso para cada uma ou use `otherwise:` como fallback",
                        missing.join(", ")
                    )),
                })
            }
        }
        Ty::List(_) => {
            // List tem duas variantes virtuais: Cons e Nil.
            // Se ambas cobertas → exaustivo. Senão → exige otherwise.
            let list_variants = ["Cons", "Nil"];
            let missing: Vec<String> = list_variants
                .iter()
                .filter(|v| !covered_variants.iter().any(|c| c == *v))
                .map(|v| v.to_string())
                .collect();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(MiddleError::MissingOtherwise {
                    span: (*span).into(),
                })
            }
        }
        Ty::Prim(_)
        | Ty::Unit
        | Ty::Struct(_)
        | Ty::Tuple(_)
        | Ty::Array(_)
        | Ty::Range(_)
        | Ty::Dict(_, _)
        | Ty::Set(_)
        | Ty::Sender(_)
        | Ty::Receiver(_)
        | Ty::ReceiverFactory(_) => {
            // Tipos infinitos exigem otherwise/wildcard.
            Err(MiddleError::MissingOtherwise {
                span: (*span).into(),
            })
        }
        Ty::Function(_, _) => Err(MiddleError::TypeMismatch {
            expected: "tipo não-função para match".into(),
            found: "função".into(),
            span: (*span).into(),
        }),
        Ty::Action(_, _) => Err(MiddleError::TypeMismatch {
            expected: "tipo não-action para match".into(),
            found: "action".into(),
            span: (*span).into(),
        }),
        Ty::InferVar(_) => Err(MiddleError::TypeMismatch {
            expected: "tipo concreto para match".into(),
            found: "variável de inferência".into(),
            span: (*span).into(),
        }),
        Ty::Var(_) => Err(MiddleError::TypeMismatch {
            expected: "tipo concreto para match".into(),
            found: "variável de tipo".into(),
            span: (*span).into(),
        }),
        Ty::Interface(_) => Err(MiddleError::TypeMismatch {
            expected: "tipo concreto para match".into(),
            found: "interface".into(),
            span: (*span).into(),
        }),
        // Byte e Bytes: tipos finitos mas infinitos em valores (0-255 / sequências).
        // Match em Byte ou Bytes exige otherwise/wildcard — como Int, Float, Text.
        Ty::Byte | Ty::Bytes | Ty::File | Ty::Socket | Ty::OverloadSet { .. } => {
            Err(MiddleError::MissingOtherwise {
                span: (*span).into(),
            })
        }
    }
}
