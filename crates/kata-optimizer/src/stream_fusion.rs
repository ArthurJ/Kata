//! Stream Fusion — DoD 60.
//!
//! Detecta composições de map/filter e reescreve em um único `FusedStream`,
//! eliminando coleções intermediárias.
//!
//! Exemplo:
//! ```text
//! map (+ 10 _) (filter (> _ 5) [1 8 3 9])
//! ```
//! Transforma em:
//! ```text
//! FusedStream {
//!     stages: [Filter { (> _ 5) }, Map { (+ 10 _) }],
//!     source: [1 8 3 9],
//! }
//! ```
//! O codegen gera um único loop que itera a fonte, aplica o predicado,
//! e se passa aplica a transformação, sem materializar a lista intermediária.
//!
//! Padrões detectados:
//! - `Map(f, Filter(g, src))` → FusedStream [Filter(g), Map(f)] source=src
//! - `Map(f, Map(g, src))`    → FusedStream [Map(g), Map(f)] source=src
//! - `Filter(g, Map(f, src))` → FusedStream [Map(f), Filter(g)] source=src
//! - `Filter(g, Filter(h, src))` → FusedStream [Filter(h), Filter(g)] source=src
//! - `Map(f, Map(g, Filter(h, src)))` → FusedStream [Filter(h), Map(g), Map(f)] source=src
//!
//! Limitação: Fold não é fundido (sempre consome, não produz lista).
//! `Fold(f, init, Map(g, src))` NÃO é reescrito — Fold não retorna lista.

use kata_ast::{Span, Spanned};
use kata_core::escape::EscapeTarget;
use kata_core::ty::Ty;
use kata_inference::{FusedStage, TypedExpr, TypedExprKind, TypedModule};

/// Resultado da tentativa de fusão de um nó.
/// `Fused` contém os stages acumulados e a coleção fonte.
/// `NotFused` contém o nó original (não é fundível).
#[allow(clippy::large_enum_variant)]
enum FusionResult {
    /// O nó foi fundido em um pipeline de stages.
    /// Os stages estão em ordem de aplicação (primeiro stage aplicado primeiro).
    /// `source` é a coleção fonte (o nó mais interno).
    Fused {
        stages: Vec<FusedStage>,
        source: Box<Spanned<TypedExpr>>,
        coll_ty: Ty,
        source_elem_ty: Ty,
        result_elem_ty: Ty,
        ret_ty: Ty,
    },
    /// O nó não é fundível — manter como está.
    NotFused,
}

/// Remove wrapper `Grouping` (parênteses) de uma expressão, recursivamente.
fn unwrap_grouping(expr: &Spanned<TypedExpr>) -> &Spanned<TypedExpr> {
    match &expr.node.kind {
        TypedExprKind::Grouping { inner } => unwrap_grouping(inner),
        _ => expr,
    }
}

/// Tenta fundir um nó TAST em um pipeline de stages.
///
/// Se o nó é `Map { collection: <fundível> }`, primeiro funde o collection,
/// depois adiciona `Map` como stage final.
/// Se o nó é `Filter { collection: <fundível> }`, mesmo padrão.
/// Se o nó é `Map/Filter { collection: <não-fundível> }`, não funde (a coleção
/// intermediária já existe por outro motivo).
fn try_fuse(expr: &Spanned<TypedExpr>) -> FusionResult {
    // Grouping (parênteses) é transparente — desempacota antes de casar.
    let expr = unwrap_grouping(expr);
    match &expr.node.kind {
        TypedExprKind::Map {
            callback,
            collection,
            ret_ty,
            ..
        } => {
            // Desempacota Grouping do collection antes de processar.
            let collection = unwrap_grouping(collection);
            // Tenta fundir o collection primeiro.
            match try_fuse(collection) {
                FusionResult::Fused {
                    mut stages,
                    source,
                    coll_ty: inner_coll_ty,
                    source_elem_ty,
                    result_elem_ty: inner_result_elem_ty,
                    ret_ty: _,
                } => {
                    // Adiciona Map como stage final.
                    // input_elem_ty = inner_result_elem_ty (tipo que o collection produziu).
                    // output_elem_ty = ret_ty elemento (tipo que Map produz).
                    // ret_ty do Map é List(B) — B é o tipo de retorno do callback.
                    let cb_ret = match &callback.node.ty {
                        Ty::Function(_, ret) => (**ret).clone(),
                        _ => return FusionResult::NotFused,
                    };
                    stages.push(FusedStage::Map {
                        callback: Box::new((**callback).clone()),
                        input_elem_ty: inner_result_elem_ty.clone(),
                        output_elem_ty: cb_ret.clone(),
                    });
                    FusionResult::Fused {
                        stages,
                        source,
                        coll_ty: inner_coll_ty,
                        source_elem_ty,
                        result_elem_ty: cb_ret,
                        ret_ty: ret_ty.clone(),
                    }
                }
                FusionResult::NotFused => {
                    // O collection não é fundível por si só.
                    // Mas se for um Map/Filter simples (não aninhado), podemos
                    // fundir: criar FusedStream com 1 stage.
                    if let Some(stage) = expr_to_stage(&collection.node) {
                        // O collection é Map/Filter sobre uma coleção fonte.
                        // Extrair a fonte.
                        let (source, src_coll_ty, src_elem_ty) = match &collection.node.kind {
                            TypedExprKind::Map {
                                collection: inner_src,
                                coll_ty,
                                elem_ty,
                                ..
                            }
                            | TypedExprKind::Filter {
                                collection: inner_src,
                                coll_ty,
                                elem_ty,
                                ..
                            } => (inner_src.clone(), coll_ty.clone(), elem_ty.clone()),
                            _ => return FusionResult::NotFused,
                        };
                        // O stage produz um tipo — para Map é o cb_ret, para Filter é elem_ty.
                        let stage_output_ty = stage_output_elem_ty(&stage);
                        let cb_ret = match &callback.node.ty {
                            Ty::Function(_, ret) => (**ret).clone(),
                            _ => return FusionResult::NotFused,
                        };
                        let stages = vec![
                            stage,
                            FusedStage::Map {
                                callback: Box::new((**callback).clone()),
                                input_elem_ty: stage_output_ty,
                                output_elem_ty: cb_ret.clone(),
                            },
                        ];
                        FusionResult::Fused {
                            stages,
                            source,
                            coll_ty: src_coll_ty,
                            source_elem_ty: src_elem_ty,
                            result_elem_ty: cb_ret,
                            ret_ty: ret_ty.clone(),
                        }
                    } else {
                        FusionResult::NotFused
                    }
                }
            }
        }
        TypedExprKind::Filter {
            callback,
            collection,
            ret_ty,
            ..
        } => {
            // Tenta fundir o collection primeiro.
            // Desempacota Grouping (parênteses) transparentes.
            let collection = unwrap_grouping(collection);
            match try_fuse(collection) {
                FusionResult::Fused {
                    mut stages,
                    source,
                    coll_ty: inner_coll_ty,
                    source_elem_ty,
                    result_elem_ty: inner_result_elem_ty,
                    ret_ty: _,
                } => {
                    // Adiciona Filter como stage final.
                    // input_elem_ty = inner_result_elem_ty.
                    stages.push(FusedStage::Filter {
                        callback: Box::new((**callback).clone()),
                        input_elem_ty: inner_result_elem_ty.clone(),
                    });
                    // Filter não muda o tipo do elemento.
                    FusionResult::Fused {
                        stages,
                        source,
                        coll_ty: inner_coll_ty,
                        source_elem_ty,
                        result_elem_ty: inner_result_elem_ty,
                        ret_ty: ret_ty.clone(),
                    }
                }
                FusionResult::NotFused => {
                    // O collection não é fundível por si só.
                    // Se for um Map/Filter simples, criar FusedStream com 1 stage.
                    if let Some(stage) = expr_to_stage(&collection.node) {
                        let (source, src_coll_ty, src_elem_ty) = match &collection.node.kind {
                            TypedExprKind::Map {
                                collection: inner_src,
                                coll_ty,
                                elem_ty,
                                ..
                            }
                            | TypedExprKind::Filter {
                                collection: inner_src,
                                coll_ty,
                                elem_ty,
                                ..
                            } => (inner_src.clone(), coll_ty.clone(), elem_ty.clone()),
                            _ => return FusionResult::NotFused,
                        };
                        let stage_output_ty = stage_output_elem_ty(&stage);
                        let stages = vec![
                            stage,
                            FusedStage::Filter {
                                callback: Box::new((**callback).clone()),
                                input_elem_ty: stage_output_ty.clone(),
                            },
                        ];
                        // Filter retorna List(elem_ty) — o tipo do elemento não muda.
                        FusionResult::Fused {
                            stages,
                            source,
                            coll_ty: src_coll_ty,
                            source_elem_ty: src_elem_ty,
                            result_elem_ty: stage_output_ty,
                            ret_ty: ret_ty.clone(),
                        }
                    } else {
                        FusionResult::NotFused
                    }
                }
            }
        }
        _ => FusionResult::NotFused,
    }
}

/// Converte um nó Map/Filter em um FusedStage.
/// Retorna None se o nó não é Map nem Filter.
fn expr_to_stage(expr: &TypedExpr) -> Option<FusedStage> {
    match &expr.kind {
        TypedExprKind::Map {
            callback, elem_ty, ..
        } => {
            // ret_ty é List(B). B é o tipo de retorno do callback.
            let cb_ret = match &callback.node.ty {
                Ty::Function(_, ret) => (**ret).clone(),
                _ => return None,
            };
            Some(FusedStage::Map {
                callback: Box::new((**callback).clone()),
                input_elem_ty: elem_ty.clone(),
                output_elem_ty: cb_ret,
            })
        }
        TypedExprKind::Filter {
            callback, elem_ty, ..
        } => Some(FusedStage::Filter {
            callback: Box::new((**callback).clone()),
            input_elem_ty: elem_ty.clone(),
        }),
        _ => None,
    }
}

/// Extrai o tipo de elemento que um stage produz.
fn stage_output_elem_ty(stage: &FusedStage) -> Ty {
    match stage {
        FusedStage::Filter { input_elem_ty, .. } => input_elem_ty.clone(),
        FusedStage::Map { output_elem_ty, .. } => output_elem_ty.clone(),
    }
}

/// Cria um `Spanned<TypedExpr>` sintético.
fn syn_expr(kind: TypedExprKind, ty: Ty) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty,
            tail_pos: false,
            escape: EscapeTarget::Local,
            kind,
        },
        Span::synthetic(),
    )
}

/// Percorre a TAST recursivamente, substituindo composições de Map/Filter
/// por FusedStream.
fn fuse_expr(expr: &Spanned<TypedExpr>) -> Spanned<TypedExpr> {
    // Primeiro recursa nas sub-expressões.
    let fused_kind = match &expr.node.kind {
        TypedExprKind::Map {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => {
            let fused_callback = fuse_expr(callback);
            let fused_collection = fuse_expr(collection);
            // Tenta fundir este Map com o collection fundido.
            let reconstructed = syn_expr(
                TypedExprKind::Map {
                    callback: Box::new(fused_callback.clone()),
                    collection: Box::new(fused_collection.clone()),
                    coll_ty: coll_ty.clone(),
                    elem_ty: elem_ty.clone(),
                    ret_ty: ret_ty.clone(),
                },
                expr.node.ty.clone(),
            );
            match try_fuse(&reconstructed) {
                FusionResult::Fused {
                    stages,
                    source,
                    coll_ty: f_coll_ty,
                    source_elem_ty,
                    result_elem_ty,
                    ret_ty: f_ret_ty,
                } => {
                    // Cria o FusedStream.
                    TypedExprKind::FusedStream {
                        stages,
                        source,
                        coll_ty: f_coll_ty,
                        source_elem_ty,
                        result_elem_ty,
                        ret_ty: f_ret_ty,
                    }
                }
                FusionResult::NotFused => {
                    // Não fundível — manter o Map reconstruído.
                    return reconstructed;
                }
            }
        }
        TypedExprKind::Filter {
            callback,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => {
            let fused_callback = fuse_expr(callback);
            let fused_collection = fuse_expr(collection);
            let reconstructed = syn_expr(
                TypedExprKind::Filter {
                    callback: Box::new(fused_callback.clone()),
                    collection: Box::new(fused_collection.clone()),
                    coll_ty: coll_ty.clone(),
                    elem_ty: elem_ty.clone(),
                    ret_ty: ret_ty.clone(),
                },
                expr.node.ty.clone(),
            );
            match try_fuse(&reconstructed) {
                FusionResult::Fused {
                    stages,
                    source,
                    coll_ty: f_coll_ty,
                    source_elem_ty,
                    result_elem_ty,
                    ret_ty: f_ret_ty,
                } => TypedExprKind::FusedStream {
                    stages,
                    source,
                    coll_ty: f_coll_ty,
                    source_elem_ty,
                    result_elem_ty,
                    ret_ty: f_ret_ty,
                },
                FusionResult::NotFused => {
                    return reconstructed;
                }
            }
        }
        TypedExprKind::Fold {
            callback,
            initial,
            collection,
            coll_ty,
            elem_ty,
            ret_ty,
        } => {
            // Fold não é fundido, mas suas sub-expressões podem conter fusões.
            TypedExprKind::Fold {
                callback: Box::new(fuse_expr(callback)),
                initial: Box::new(fuse_expr(initial)),
                collection: Box::new(fuse_expr(collection)),
                coll_ty: coll_ty.clone(),
                elem_ty: elem_ty.clone(),
                ret_ty: ret_ty.clone(),
            }
        }
        // Outros nós: recursar genericamente seria ideal, mas para o escopo
        // do DoD 60, só precisamos fundir Map/Filter que aparecem como entry
        // point ou dentro de outros Map/Filter/Fold. A recursão acima já
        // cobre o caso comum. Outros nós são preservados inalterados.
        _ => return expr.clone(),
    };

    syn_expr(fused_kind, expr.node.ty.clone())
}

/// Pass de stream fusion — percorre o módulo e funde composições de Map/Filter.
pub(crate) fn stream_fusion_pass(typed: &mut TypedModule) {
    // Funda o entry point.
    let fused_entry = fuse_expr(&typed.entry);
    typed.entry = fused_entry;

    // Funda o body de cada função.
    let mut new_functions = Vec::new();
    for func in &typed.functions {
        let mut new_func = func.clone();
        new_func.clauses = func
            .clauses
            .iter()
            .map(|clause| {
                let mut new_clause = clause.clone();
                new_clause.body = fuse_expr(&clause.body);
                new_clause
            })
            .collect();
        new_functions.push(new_func);
    }
    typed.functions = new_functions;
}
