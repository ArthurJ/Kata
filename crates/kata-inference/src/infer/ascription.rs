//! Inferência de `TypeAscription` (`expr::Type`).
//!
//! Extraído de `expr.rs` — responsabilidade: lidar com ascription-refined
//! (validação compile-time de predicados), ascription-construção
//! (Tuple→StructConstruct), e rebaixamento de literais (Int→Float, etc.).

use kata_ast::{Expr, Span, Spanned, TypeExpr};
use kata_core::StructKey;
use kata_core::escape::EscapeTarget;
use kata_core::ty::{PrimTy, Ty, TypeEnv};
use kata_diagnostics::MiddleError;

use crate::typed::{TypedExpr, TypedExprKind};

use super::expr::{InferCtx, infer_expr_hinted};
use super::helpers::InferResult;
use kata_resolution::resolve_type_expr;

/// Infere uma `TypeAscription` — `expr::Type`.
///
/// Cenários:
/// 1. **Grouped ascription** `((expr))::Type` — barreira de hint (sem hint).
/// 2. **Ascription-refined** `5::PositiveInt` — valida predicados em compile-time.
/// 3. **Ascription-construção** `(a, b)::Pessoa` — Tuple→StructConstruct.
/// 4. **Rebaixamento** `42::Float` — IntLit→Float, etc.
pub(crate) fn infer_type_ascription(
    expr: &Spanned<Expr>,
    ty: &Spanned<TypeExpr>,
    span: &Span,
    env: &mut TypeEnv,
    ctx: &InferCtx,
    tail_pos: bool,
    _hint: Option<&Ty>,
) -> InferResult<TypedExpr> {
    let target_ty = resolve_type_expr(
        &ty.node,
        env,
        ctx.interface_registry,
        ctx.struct_registry,
        None,
    );

    // Grouped ascription `((expr))::Type` — barreira de hint.
    // Se expr é Grouping(Grouping(inner2)), o grouping duplo bloqueia
    // a propagação do hint. Inferir inner2 sem hint (None), depois
    // validar contra target_ty normalmente.
    let (inner, _is_grouped) = match &expr.node {
        Expr::Grouping { inner: g1 } => {
            if let Expr::Grouping { inner: g2 } = &g1.node {
                // ((expr))::Type — barreira: sem hint
                let typed = infer_expr_hinted(&g2.node, &g2.span, env, ctx, false, None)?;
                (typed, true)
            } else {
                // (expr)::Type — propaga hint normalmente.
                // Mesma lógica de família polimórfica do branch _ abaixo.
                let is_family_target = matches!(
                    &resolve_type_expr(&ty.node, env, ctx.interface_registry, ctx.struct_registry, None),
                    Ty::Struct(StructKey::Family(n)) | Ty::Struct(StructKey::Plain(n))
                        if ctx.type_graph.is_family(n)
                );
                let hint = if is_family_target {
                    None
                } else {
                    Some(&target_ty)
                };
                let typed = infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, hint)?;
                (typed, false)
            }
        }
        _ => {
            // expr::Type — propaga hint normalmente.
            // Mas se target_ty é família polimórfica (Family/Plain com
            // instâncias), não propagar como hint — o inner será promovido
            // a Instance depois. Passar Family como hint causa erro em
            // actions como `(rational 3)::NonZero` (rational não retorna
            // Family). Para literals como `3::NonZero`, o hint é ignorado
            // (IntLit não usa hint de tipo struct).
            let is_family_target = matches!(
                &resolve_type_expr(&ty.node, env, ctx.interface_registry, ctx.struct_registry, None),
                Ty::Struct(StructKey::Family(n)) | Ty::Struct(StructKey::Plain(n))
                    if ctx.type_graph.is_family(n)
            );
            let hint = if is_family_target {
                None
            } else {
                Some(&target_ty)
            };
            let typed = infer_expr_hinted(&expr.node, &expr.span, env, ctx, false, hint)?;
            (typed, false)
        }
    };

    // Família polimórfica → instância concreta.
    // Se target_ty é Family ou Plain(family_name) e a família tem instâncias
    // (is_instance_of), e o inner tem tipo primitivo concreto, promove
    // target_ty para Instance(family_name, concrete). Ex: `3::NonZero`
    // com inner :: Int → Ty::Struct(Instance("NonZero", "Int")).
    //
    // `resolve_type_expr` produz `Family("NonZero")` para `Named("NonZero")`
    // quando NonZero é família registrada, e `Plain("NonZero")` quando ainda
    // não foi registrada (pass0a). Ambos precisam ser tratados aqui.
    let target_ty = if let Ty::Struct(StructKey::Family(family_name))
    | Ty::Struct(StructKey::Plain(family_name)) = &target_ty
    {
        if ctx.type_graph.is_family(family_name) {
            let concrete = match &inner.ty {
                Ty::Prim(PrimTy::Int) => "Int",
                Ty::Prim(PrimTy::Float) => "Float",
                Ty::Prim(PrimTy::Rational) => "Rational",
                Ty::Prim(PrimTy::Text) => "Text",
                _ => "",
            };
            if !concrete.is_empty()
                && ctx
                    .struct_registry
                    .get_instance(family_name, concrete)
                    .is_some()
            {
                Ty::Struct(StructKey::Instance(
                    family_name.clone(),
                    concrete.to_string(),
                ))
            } else {
                target_ty
            }
        } else {
            target_ty
        }
    } else {
        target_ty
    };

    // Downcast Instance de família polimórfica → tipo base.
    // `i::Int` onde `i :: NonZero::Int` (Instance("NonZero", "Int")).
    // A instância tem alias_of = "Int" no StructRegistry. No-op em runtime
    // (mesmos bits). Deve vir antes do downcast refined/alias genérico
    // porque struct_registry.get("NonZero") retorna a família (sem
    // alias_of), não a instância específica.
    if let Ty::Struct(StructKey::Instance(family, concrete)) = &inner.ty
        && let Some(inst_info) = ctx.struct_registry.get_instance(family, concrete)
    {
        let base_name = inst_info.alias_of.as_deref().unwrap_or(concrete);
        let base_matches = match (&target_ty, base_name) {
            (Ty::Prim(PrimTy::Int), "Int") => true,
            (Ty::Prim(PrimTy::Float), "Float") => true,
            (Ty::Prim(PrimTy::Rational), "Rational") => true,
            (Ty::Prim(PrimTy::Text), "Text") => true,
            (Ty::Struct(key), b) if key.name() == b => true,
            _ => false,
        };
        if base_matches {
            return Ok(TypedExpr {
                span: *span,
                ty: target_ty.clone(),
                tail_pos,
                escape: if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Caller
                },
                kind: TypedExprKind::TypeAscription {
                    expr: Box::new(Spanned::new(inner, expr.span)),
                    target_ty,
                    pending_predicates: Vec::new(),
                },
            });
        }
    }

    // Downcast refined/alias→base — `a::Int` onde `a :: PositiveInt`
    // ou `x::Float` onde `x :: Altura` (alias de Float) ou
    // `p::Float` onde `p :: Peso` (alias de PositiveFloat que é refined de Float).
    // O refined/alias é alias do base no layout (mesmos bits). No-op em runtime.
    // Válido quando target_ty aparece em algum ponto da cadeia de alias_of.
    // Deve vir ANTES do ascription-refined, senão `p::Float` entra no path
    // de refined-validation que exige literal e falha para variáveis.
    if let Ty::Struct(ref key) = inner.ty
        && let Some(struct_info) = ctx.struct_registry.get(key.name())
        && (struct_info.alias_of.is_some() || struct_info.predicates.is_some())
    {
        // Percorre a cadeia de alias_of recursivamente.
        let mut current = key.name().to_string();
        let mut found_match = false;
        while let Some(info) = ctx.struct_registry.get(&current) {
            let base_name = match &info.alias_of {
                Some(b) => b.clone(),
                None => break,
            };
            let base_matches = match (&target_ty, base_name.as_str()) {
                (Ty::Prim(PrimTy::Int), "Int") => true,
                (Ty::Prim(PrimTy::Float), "Float") => true,
                (Ty::Prim(PrimTy::Rational), "Rational") => true,
                (Ty::Prim(PrimTy::Text), "Text") => true,
                (Ty::Struct(key), b) if key.name() == b => true,
                _ => false,
            };
            if base_matches {
                found_match = true;
                break;
            }
            current = base_name;
        }
        if found_match {
            return Ok(TypedExpr {
                span: *span,
                ty: target_ty.clone(),
                tail_pos,
                escape: if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Caller
                },
                kind: TypedExprKind::TypeAscription {
                    expr: Box::new(Spanned::new(inner, expr.span)),
                    target_ty,
                    pending_predicates: Vec::new(),
                },
            });
        }
    }

    // Ascription-refined — `5::PositiveInt` valida predicados
    // em compile-time. Se target é um tipo refined (StructInfo com
    // predicates) e expr é literal, avalia cada predicado via
    // const_eval. Se todos passam → TypeAscription com target_ty.
    if let Ty::Struct(ref key) = target_ty
        && let Some(struct_info) = ctx.struct_registry.get(key.name())
        && struct_info.predicates.is_some()
    {
        // Refined type — expr deve ser literal numérico OU path conditions
        // podem provar o predicado sobre não-literais (refinement propagation).
        // Grouping é transparente: `(5)::NonZero` e `((-5))::NonZero` devem
        // ser aceitos — const_eval e typed_expr_to_const_val já desembrulham
        // Grouping recursivamente.
        let is_literal = is_literal_expr(&inner);
        // Gate conservador duplo (débito 1):
        // (a) sem path conditions e não-literal → construtor;
        // (b) não-literal que referencia `var` → construtor. O Z3 não
        //     tem material sound sobre mutáveis: nem provar (var vira
        //     variável livre e "prova" com valores arbitrários), nem
        //     refutar (P28: fact stale rejeita ascription correta).
        if !is_literal && ctx.path_conditions.borrow().is_empty() {
            // Sem path conditions e não-literal: não há como provar em
            // compile-time. Usar construtor falível (ex: PositiveInt n).
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "literal para ascription refined {key_name} \
                     (use construtor para expr não-literal)",
                    key_name = key.name(),
                ),
                found: format!("{:?}", inner.kind),
                span: expr.span.into(),
            });
        }
        if !is_literal
            && ctx
                .path_conditions
                .borrow()
                .references_mutable(&inner, ctx.table)
        {
            // Não-literal sobre `var`: conservadoramente rejeitada.
            // Os facts do escopo podem até ser legítimos (sobre lets),
            // mas o var não é semeado — provar ou refutar com ele como
            // variável livre é insound nas duas direções.
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "literal para ascription refined {key_name} \
                     (var é opaco ao Z3 — use construtor para expr sobre var)",
                    key_name = key.name(),
                ),
                found: format!("{:?}", inner.kind),
                span: expr.span.into(),
            });
        }
        if !is_literal {
            // Não-literal com path conditions: tenta provar via Z3.
            // Se Z3 não decide (Unknown), fallback para pending (comptime).
            // Continua para o loop de predicados abaixo, onde
            // const_eval_predicate retornará None e try_prove_with_path_conditions
            // será tentado.
        }

        // Busca os predicados em refined_decls.
        let refined_decl = ctx
            .refined_decls
            .iter()
            .find(|rd| rd.name == key.name())
            .ok_or_else(|| MiddleError::TypeMismatch {
                expected: format!("RefinedDeclInfo para {}", key.name()),
                found: "não encontrado em refined_decls".into(),
                span: expr.span.into(),
            })?;

        // Avalia cada predicado sobre o literal.
        let mut pending: Vec<Spanned<TypedExpr>> = Vec::new();
        for (i, pred) in refined_decl.predicates.iter().enumerate() {
            match super::const_eval::const_eval_predicate(pred, expr) {
                Some(true) => {} // predicado satisfeito
                Some(false) => {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("predicado {i} de {} satisfeito", key.name()),
                        found: "predicado falhou para valor".to_string(),
                        span: expr.span.into(),
                    });
                }
                None => {
                    // Predicado complexo — não avaliável localmente pelo
                    // const_eval. Substitui Hole pelo valor, tipa via
                    // infer_expr_hinted, e tenta provar com path conditions
                    // (Z3) antes de adicionar como pending.
                    let substituted = super::const_eval::substitute_hole(pred, expr);
                    let typed_pred = infer_expr_hinted(
                        &substituted.node,
                        &substituted.span,
                        env,
                        ctx,
                        false,
                        Some(&Ty::Sum("Boolean".to_string())),
                    )?;

                    // Tenta provar com path conditions (Z3).
                    match super::path_conditions::try_prove_with_path_conditions(
                        &typed_pred,
                        &ctx.path_conditions.borrow(),
                        ctx.inline_fns,
                    ) {
                        Some(true) => {} // provado satisfeito pelas path conditions
                        Some(false) => {
                            return Err(MiddleError::TypeMismatch {
                                expected: format!("predicado {i} de {} satisfeito", key.name()),
                                found: "predicado refutado pelas path conditions".to_string(),
                                span: expr.span.into(),
                            });
                        }
                        None => {
                            // Z3 não decidiu — fallback conservador: pending
                            // para comptime pass validar via jit_eval.
                            pending.push(Spanned::new(typed_pred, substituted.span));
                        }
                    }
                }
            }
        }

        // Todos os predicados passaram (ou são pending) — produz TypeAscription.
        return Ok(TypedExpr {
            span: *span,
            ty: target_ty.clone(),
            tail_pos,
            escape: if ctx.ret_ty.is_some() {
                if tail_pos {
                    EscapeTarget::Caller
                } else {
                    EscapeTarget::Local
                }
            } else {
                EscapeTarget::Caller
            },
            kind: TypedExprKind::TypeAscription {
                expr: Box::new(Spanned::new(inner, expr.span)),
                target_ty,
                pending_predicates: pending,
            },
        });
    }

    // Ascription-construção — `(a, b)::Pessoa` → StructConstruct.
    // Se inner é Tuple e target é Struct, e o shape bate (mesmo nº de
    // elementos, tipos compatíveis), produz StructConstruct.
    if let Ty::Struct(ref key) = target_ty
        && let TypedExprKind::Tuple { elements } = &inner.kind
        && let Some(struct_info) = ctx.struct_registry.get(key.name())
        && !struct_info.fields.is_empty()
        && struct_info.alias_of.is_none()
    {
        // Shape check: mesmo número de elementos
        if elements.len() != struct_info.fields.len() {
            return Err(MiddleError::TypeMismatch {
                expected: format!(
                    "Struct {} with {} fields",
                    key.name(),
                    struct_info.fields.len()
                ),
                found: format!("Tuple with {} elements", elements.len()),
                span: expr.span.into(),
            });
        }
        // Verifica tipos compatíveis
        let mut shape_ok = true;
        for (elem, field) in elements.iter().zip(struct_info.fields.iter()) {
            if elem.node.ty != field.ty {
                shape_ok = false;
                break;
            }
        }
        if shape_ok {
            let values = elements
                .iter()
                .map(|e| Spanned::new(e.node.clone(), e.span))
                .collect();
            return Ok(TypedExpr {
                span: *span,
                ty: target_ty.clone(),
                tail_pos,
                escape: if ctx.ret_ty.is_some() {
                    if tail_pos {
                        EscapeTarget::Caller
                    } else {
                        EscapeTarget::Local
                    }
                } else {
                    EscapeTarget::Caller
                },
                kind: TypedExprKind::StructConstruct {
                    struct_name: key.name().to_string(),
                    values,
                },
            });
        }
        // Shape mismatch (tipos incompatíveis) → error
        return Err(MiddleError::TypeMismatch {
            expected: format!(
                "Struct {} fields [{}]",
                key.name(),
                struct_info
                    .fields
                    .iter()
                    .map(|f| f.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            found: format!(
                "Tuple elements [{}]",
                elements
                    .iter()
                    .map(|e| e.node.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            span: expr.span.into(),
        });
    }

    let rebaixa_ok = match (&inner.kind, &target_ty) {
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Int)) => true,
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Float)) => true,
        (TypedExprKind::IntLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
        (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Float)) => true,
        (TypedExprKind::FloatLit { .. }, Ty::Prim(PrimTy::Rational)) => true,
        (TypedExprKind::TextLit { .. }, Ty::Prim(PrimTy::Text)) => true,
        _ if inner.ty == target_ty => true,
        _ => false,
    };

    if !rebaixa_ok {
        return Err(MiddleError::TypeMismatch {
            expected: format!("{}", target_ty),
            found: format!("{}", inner.ty),
            span: expr.span.into(),
        });
    }

    Ok(TypedExpr {
        span: *span,
        ty: target_ty.clone(),
        tail_pos,
        escape: if ctx.ret_ty.is_some() {
            if tail_pos {
                EscapeTarget::Caller
            } else {
                EscapeTarget::Local
            }
        } else {
            EscapeTarget::Caller
        },
        kind: TypedExprKind::TypeAscription {
            expr: Box::new(Spanned::new(inner, expr.span)),
            target_ty,
            pending_predicates: Vec::new(),
        },
    })
}

/// Verifica se um `TypedExpr` é um literal (ou Grouping sobre literal).
///
/// Equivalente ao `matches!` anterior, mas recursivo sobre `Grouping` —
/// `(5)::NonZero` e `((-5))::PositiveInt` são literais transparentemente
/// embrulhados. Consistente com `eval_const` e `typed_expr_to_const_val`
/// (ambos desembrulham Grouping).
fn is_literal_expr(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Grouping { inner } => is_literal_expr(&inner.node),
        TypedExprKind::IntLit { .. }
        | TypedExprKind::FloatLit { .. }
        | TypedExprKind::TextLit { .. }
        | TypedExprKind::ListLit { .. }
        | TypedExprKind::ArrayLit { .. }
        | TypedExprKind::SetLit { .. }
        | TypedExprKind::DictLit { .. } => true,
        _ => false,
    }
}
