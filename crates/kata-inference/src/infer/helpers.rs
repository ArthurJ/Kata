//! Helpers compartilhados entre os submódulos de inferência.
//!
//! Funções utilitárias usadas por múltiplos submódulos do `infer/`:
//! conversão de erros, peeling de Grouping, populate do DispatchTable
//! e span fallback. A resolução de `TypeExpr` → `Ty` usa
//! [`kata_resolution::resolve_type_expr`].

use kata_ast::{Expr, Item, Pattern, Span, Spanned, WithBinding};
use kata_core::dispatch::{DispatchError, DispatchTable, OverloadInfo};
use kata_core::enum_registry::EnumRegistry;
use kata_core::ty::{Ty, TypeEnv};
use kata_diagnostics::{MiddleError, MietteSpan};
use kata_resolution::Signature;

use crate::patterns;
use crate::typed::{TypedExpr, TypedExprKind, TypedPattern, TypedWithBinding};

use super::expr::{InferCtx, infer_expr};

/// Erro de inferência — wrapped `MiddleError` (carrega Span).
pub(crate) type InferResult<T> = Result<T, MiddleError>;

/// Popula o DispatchTable a partir das assinaturas do ResolvedModule.
pub(crate) fn populate_dispatch_table(signatures: &[Signature]) -> DispatchTable {
    let mut table = DispatchTable::new();
    for sig in signatures {
        let ffi_symbol = sig.ffi_symbol.clone();
        let associative_neutral = sig.associative_neutral;

        table.insert(OverloadInfo {
            name: sig.name.clone(),
            params: sig.param_types.clone(),
            ret: sig.return_type.clone(),
            ffi_symbol,
            is_action: sig.is_action,
            is_generic: false,
            is_constructor: false,
            associative_neutral,
            type_params: sig.type_params.clone(),
            substitutions: None,
            param_names: vec![],
            param_defaults: vec![],
        });

        // Marca comutativa quando a assinatura tem @commutative.
        if sig.is_commutative {
            table.mark_commutative(&sig.name);
        }
    }
    table
}

/// Span do último item ou sintético se módulo vazio.
pub(crate) fn item_span_or_synthetic(items: &[Spanned<Item>]) -> MietteSpan {
    items
        .last()
        .map(|i| i.span.into())
        .unwrap_or(MietteSpan(Span::synthetic()))
}

/// Converte `DispatchError` em `MiddleError` para diagnóstico.
pub(crate) fn dispatch_to_middle_error(err: DispatchError, span: Span) -> MiddleError {
    match err {
        DispatchError::FunctionNotFound { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::TypeMismatch { name, .. } => MiddleError::NoOverload {
            name,
            span: span.into(),
        },
        DispatchError::AmbiguousDispatch { name, .. } => MiddleError::AmbiguousDispatch {
            name,
            span: span.into(),
        },
    }
}

/// Remove camadas de `Expr::Grouping` — retorna a expressão interna.
pub(crate) fn peel_grouping_expr(expr: &kata_ast::Expr) -> &kata_ast::Expr {
    match expr {
        kata_ast::Expr::Grouping { inner } => peel_grouping_expr(&inner.node),
        _ => expr,
    }
}

/// Casa uma lista de padrões contra tipos esperados, registrando bindings no escopo.
///
/// Extrai o loop comum a `infer_named_function`, `infer_lambda`, e aos dois
/// caminhos de apply de lambda inline. Retorna os padrões tipados na mesma
/// ordem dos argumentos.
pub(crate) fn check_patterns(
    patterns: &[Spanned<Pattern>],
    param_tys: &[Ty],
    enum_registry: &EnumRegistry,
    env: &mut TypeEnv,
    iface_registry: &kata_core::InterfaceRegistry,
    struct_registry: &kata_core::struct_registry::StructRegistry,
) -> InferResult<Vec<Spanned<TypedPattern>>> {
    let mut typed_patterns: Vec<Spanned<TypedPattern>> = Vec::with_capacity(patterns.len());
    for (i, pat) in patterns.iter().enumerate() {
        let typed_pat = patterns::check_pattern(
            pat,
            &param_tys[i],
            enum_registry,
            env,
            iface_registry,
            struct_registry,
        )?;
        typed_patterns.push(typed_pat);
    }
    Ok(typed_patterns)
}

/// Processa with bindings (açúcar → let chain) num escopo filho.
///
/// Infere o valor de cada binding, registra no `env` e coleta os bindings
/// tipados. Compartilhado por `infer_named_function`, `infer_lambda`, e pelos
/// dois caminhos de apply de lambda inline.
pub(crate) fn process_with_bindings(
    wbs: &[WithBinding],
    env: &mut TypeEnv,
    ctx: &InferCtx,
) -> InferResult<Vec<TypedWithBinding>> {
    let mut typed_with_bindings: Vec<TypedWithBinding> = Vec::new();
    for wb in wbs {
        let typed_value = infer_expr(&wb.value.node, &wb.value.span, env, ctx, false)?;
        let val_ty = typed_value.ty.clone();
        env.define(&wb.name, val_ty, "__local__");
        typed_with_bindings.push(TypedWithBinding {
            name: wb.name.clone(),
            value: Spanned::new(typed_value, wb.value.span),
        });
    }
    Ok(typed_with_bindings)
}

/// Mapeia chaves de DictLit → nomes de params da função/action e reordena para Tuple.
///
/// Extrai os nomes dos params do DispatchTable (OverloadInfo.param_names).
/// Cada chave do Dict deve ser `TextLit` cujo valor corresponde a um nome de param.
/// Reordena os valores na ordem posicional dos params e produz `TypedExprKind::Tuple`.
///
/// Funciona para funções puras e actions — qualquer função com `param_names`
/// não-vazios. O caller (infer_apply ou infer_action_call) identifica se o
/// DictLit é o único arg de um Apply/ActionCall e chama esta função.
///
/// Erros:
/// - Função/action sem params nomeados → "use chamada posicional"
/// - Chave que não é TextLit → "args nomeados exigem chaves literais de Text"
/// - Chave não corresponde a nenhum param → "parâmetro `X` não existe na função `f`"
/// - Param faltante → "parâmetro `X` não foi fornecido"
pub(crate) fn reorder_dict_args_to_tuple(
    callee: &str,
    entries: &[(Spanned<TypedExpr>, Spanned<TypedExpr>)],
    typed_args: &TypedExpr,
    ctx: &InferCtx,
    env: &mut TypeEnv,
    span: Span,
) -> InferResult<TypedExpr> {
    // Busca a função/action no DispatchTable para obter os nomes dos params.
    let overloads = ctx.table.get_overloads(callee).ok_or_else(|| {
        kata_diagnostics::MiddleError::UnboundName {
            suggestion: None,
            name: format!("Função `{callee}` não declarada"),
            span: span.into(),
        }
    })?;

    // Encontra o overload com param_names não-vazios. Pode ser função pura
    // ou action — o critério é ter nomes, não o flag is_action.
    let overload = overloads
        .iter()
        .find(|o| !o.param_names.is_empty())
        .ok_or_else(|| kata_diagnostics::MiddleError::TypeMismatch {
            expected: format!("`{callee}` com params nomeados para chamada via Dict"),
            found: format!(
                "`{callee}` não tem params nomeados — use chamada posicional {callee} ...)"
            ),
            span: span.into(),
        })?;

    let param_names: &[Option<String>] = &overload.param_names;
    let param_defaults: &[Option<Spanned<Expr>>] = &overload.param_defaults;

    // Constrói mapa nome → índice posicional.
    let name_to_idx: std::collections::HashMap<&str, usize> = param_names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| name.as_ref().map(|n| (n.as_str(), i)))
        .collect();

    // Valida e mapeia cada entrada do Dict.
    let mut reordered: Vec<Option<Spanned<TypedExpr>>> = vec![None; param_names.len()];

    for (key_expr, val_expr) in entries {
        // Chave deve ser TextLit.
        let key_name = match &key_expr.node.kind {
            TypedExprKind::TextLit { text } => text.clone(),
            _ => {
                return Err(kata_diagnostics::MiddleError::TypeMismatch {
                    expected: "chave literal de Text".into(),
                    found: "expressão como chave".into(),
                    span: key_expr.span.into(),
                });
            }
        };

        // Chave deve corresponder a um param.
        let idx = *name_to_idx.get(key_name.as_str()).ok_or_else(|| {
            kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("parâmetro de `{callee}`"),
                found: format!("`{key_name}` não é parâmetro de `{callee}`"),
                span: key_expr.span.into(),
            }
        })?;

        if reordered[idx].is_some() {
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("parâmetro `{key_name}` fornecido uma vez"),
                found: format!("parâmetro `{key_name}` duplicado"),
                span: key_expr.span.into(),
            });
        }

        reordered[idx] = Some(val_expr.clone());
    }

    // Preenche faltantes: usa default se houver, erro se obrigatório.
    let missing_indices: Vec<usize> = reordered
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| if slot.is_none() { Some(i) } else { None })
        .collect();

    for i in missing_indices {
        if let Some(default_expr) = param_defaults.get(i).and_then(|d| d.as_ref()) {
            let typed_default =
                infer_expr(&default_expr.node, &default_expr.span, env, ctx, false)?;
            reordered[i] = Some(Spanned::new(typed_default, default_expr.span));
        } else {
            let name = param_names[i].as_deref().unwrap_or("?");
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("parâmetro `{name}` de `{callee}`"),
                found: "parâmetro não fornecido".into(),
                span: span.into(),
            });
        }
    }

    // Produz Tuple com valores reordenados.
    let elements: Vec<Spanned<TypedExpr>> = reordered
        .into_iter()
        .map(|s| s.expect("todos elementos reordenados são Some"))
        .collect();
    let tys: Vec<Ty> = elements.iter().map(|e| e.node.ty.clone()).collect();

    Ok(TypedExpr {
        ty: Ty::Tuple(tys),
        kind: TypedExprKind::Tuple { elements },
        span: typed_args.span,
        tail_pos: typed_args.tail_pos,
        escape: typed_args.escape,
    })
}

/// Preenche defaults em chamada posicional de action.
///
/// Se a tupla de args tem menos elementos que o número de params da action,
/// e os params faltantes têm default no template, infere os defaults e anexa.
/// Se um param faltante é obrigatório (`_`), retorna erro.
///
/// Se o número de args já bate com o número de params, ou a action não tem
/// params nomeados (função pura chamada via `!` — não deveria acontecer),
/// retorna `typed_args` inalterado.
pub(crate) fn fill_positional_defaults(
    callee: &str,
    typed_args: &TypedExpr,
    ctx: &InferCtx,
    env: &mut TypeEnv,
    span: Span,
) -> InferResult<TypedExpr> {
    // Extrai elementos da tupla (se for Tuple).
    let elements = match &typed_args.kind {
        TypedExprKind::Tuple { elements } => elements.clone(),
        TypedExprKind::Unit => Vec::new(),
        _ => return Ok(typed_args.clone()), // não-tupla: não tenta preencher
    };

    // Busca overloads da action no DispatchTable.
    let overloads = match ctx.table.get_overloads(callee) {
        Some(o) => o,
        None => return Ok(typed_args.clone()), // não encontrado: deixa dispatch falhar
    };

    // Encontra o overload com param_names não-vazios (é uma action com params).
    let overload = match overloads.iter().find(|o| !o.param_names.is_empty()) {
        Some(o) => o,
        None => return Ok(typed_args.clone()), // sem params nomeados: nada a preencher
    };

    let param_count = overload.param_names.len();
    let arg_count = elements.len();

    // Args >= params: nada a preencher.
    if arg_count >= param_count {
        return Ok(typed_args.clone());
    }

    // Params faltantes: do índice arg_count até param_count.
    let mut new_elements = elements;
    for i in arg_count..param_count {
        if let Some(default_expr) = overload.param_defaults.get(i).and_then(|d| d.as_ref()) {
            let typed_default =
                infer_expr(&default_expr.node, &default_expr.span, env, ctx, false)?;
            new_elements.push(Spanned::new(typed_default, default_expr.span));
        } else {
            // Param obrigatório faltante em chamada posicional.
            let name = overload
                .param_names
                .get(i)
                .and_then(|n| n.as_deref())
                .unwrap_or("?");
            return Err(kata_diagnostics::MiddleError::TypeMismatch {
                expected: format!("parâmetro `{name}` de `{callee}`"),
                found: "parâmetro não fornecido".into(),
                span: span.into(),
            });
        }
    }

    let tys: Vec<Ty> = new_elements.iter().map(|e| e.node.ty.clone()).collect();
    Ok(TypedExpr {
        ty: Ty::Tuple(tys),
        kind: TypedExprKind::Tuple {
            elements: new_elements,
        },
        span: typed_args.span,
        tail_pos: typed_args.tail_pos,
        escape: typed_args.escape,
    })
}
