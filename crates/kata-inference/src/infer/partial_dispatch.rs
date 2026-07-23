//! DoD 27: Partial dispatch para inferência de holes.
//!
//! Tenta inferir tipos dos parâmetros do lambda via partial dispatch.
//! Quando o body do lambda é `Apply(Ident(name), args)` e algum arg é um
//! `Ident` cujo nome corresponde a um parâmetro do lambda, chama
//! `resolve_partial` com `None` nessas posições e tipos concretos nas
//! demais. Se resolve único, retorna os tipos extraídos.

use kata_ast::{Expr, Pattern, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::interface_registry::InterfaceRegistry;
use kata_core::ty::{Ty, TypeEnv};

use super::helpers::peel_grouping_expr;
use kata_resolution::resolve_type_expr;

/// Tenta inferir tipos dos parâmetros do lambda via partial dispatch.
///
/// Retorna `Vec<Ty>` (vazio se não aplicável ou ambíguo). A ordem corresponde
/// aos `patterns` do lambda.
pub(crate) fn try_partial_dispatch(
    patterns: &[Spanned<Pattern>],
    body: &Spanned<Expr>,
    env: &TypeEnv,
    table: &DispatchTable,
    iface_reg: &InterfaceRegistry,
) -> Vec<Ty> {
    // Só funciona com 1+ patterns Ident (holes desugared viram lambda com 1 param).
    let param_names: Vec<&str> = patterns
        .iter()
        .filter_map(|p| match &p.node {
            Pattern::Ident(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    if param_names.is_empty() {
        return Vec::new();
    }

    // Extrai o Apply do body (ignora Grouping).
    let body_core = peel_grouping_expr(&body.node);

    let (callee_name, args) = match body_core {
        Expr::Apply { callee, args } => {
            let name = match &callee.node {
                Expr::Ident { name } => name.clone(),
                _ => return Vec::new(),
            };
            (name, args)
        }
        _ => return Vec::new(),
    };

    // A função precisa estar no DispatchTable.
    if !table.has_function(&callee_name) {
        return Vec::new();
    }

    // Constrói lista de Option<Ty> por posição de arg.
    // None = arg é um parâmetro do lambda (hole → Ident com nome do pattern).
    // Some(ty) = arg é um literal, ident conhecido no env, ascription, etc.
    //
    // Caso especial: TypeAscription { expr: Ident(name), ty } onde name é um
    // parâmetro do lambda — é um hole com ascription (DoD 28). A posição é
    // None (hole) mas o tipo do parâmetro é extraído da ascription diretamente.
    let mut partial_args: Vec<Option<Ty>> = Vec::with_capacity(args.len());
    let mut ascription_hints: Vec<Option<Ty>> = vec![None; patterns.len()];
    for arg in args.iter() {
        let arg_core = peel_grouping_expr(&arg.node);
        match arg_core {
            Expr::Ident { name } => {
                if param_names.contains(&name.as_str()) {
                    // É um parâmetro do lambda — posição ausente (hole)
                    partial_args.push(None);
                } else if let Some(ty) = env.lookup(name) {
                    // Ident conhecido no escopo — usa seu tipo
                    partial_args.push(Some(ty.clone()));
                } else {
                    // Ident desconhecido — não podemos inferir
                    return Vec::new();
                }
            }
            Expr::TypeAscription { expr: inner, ty } => {
                // Verifica se o inner é um Ident que é parâmetro do lambda (hole com ascription).
                let inner_core = peel_grouping_expr(&inner.node);
                #[allow(clippy::collapsible_if)]
                if let Expr::Ident { name } = inner_core {
                    if param_names.contains(&name.as_str()) {
                        // Hole com ascription: None no dispatch + hint direto da ascription
                        partial_args.push(None);
                        let resolved = resolve_type_expr(&ty.node, env, iface_reg);
                        for (pat_idx, pat) in patterns.iter().enumerate() {
                            if let Pattern::Ident(pat_name) = &pat.node {
                                if pat_name == name {
                                    ascription_hints[pat_idx] = Some(resolved.clone());
                                }
                            }
                        }
                        continue;
                    }
                }
                // Ascription em arg que não é parâmetro do lambda — usa tipo da ascription
                let resolved = resolve_type_expr(&ty.node, env, iface_reg);
                partial_args.push(Some(resolved));
            }
            Expr::IntLit { .. } => partial_args.push(Some(Ty::int())),
            Expr::FloatLit { .. } => partial_args.push(Some(Ty::float())),
            Expr::TextLit { .. } => partial_args.push(Some(Ty::text())),
            Expr::Unit => partial_args.push(Some(Ty::Unit)),
            _ => return Vec::new(), // tipo complexo — não tenta
        }
    }

    // Tenta resolve_partial.
    let result = match table.resolve_partial(&callee_name, &partial_args, iface_reg) {
        Ok(r) => r,
        Err(_) => {
            // Se resolve_partial falha, mas há ascription_hints, usa eles diretamente.
            // Isto cobre o caso `+ _::Int _::Float` onde não há overload [Int, Float]
            // mas ascription_hints tem os tipos. O typeck do body vai falhar com
            // NoOverload, o que é o comportamento correto.
            if ascription_hints.iter().all(|h| h.is_some()) {
                return ascription_hints
                    .into_iter()
                    .map(|h| h.expect("checked above"))
                    .collect();
            }
            return Vec::new();
        }
    };

    // Mapeia hole_types de volta para os parâmetros do lambda.
    // hole_types[i] = Some(ty) significa que a posição i era ausente (hole).
    // O arg na posição i é Ident(name) ou TypeAscription(Ident(name), ty) onde
    // name é um parâmetro do lambda.
    let mut hints = vec![None; patterns.len()];
    for (arg_idx, hole_ty) in result.hole_types.iter().enumerate() {
        if let Some(ty) = hole_ty {
            // arg na posição arg_idx era um hole → é um parâmetro do lambda
            let arg_core = peel_grouping_expr(&args[arg_idx].node);
            let arg_name = match arg_core {
                Expr::Ident { name } => name,
                Expr::TypeAscription { expr: inner, .. } => match peel_grouping_expr(&inner.node) {
                    Expr::Ident { name } => name,
                    _ => continue,
                },
                _ => continue,
            };
            // Encontra o índice do parâmetro com este nome
            for (pat_idx, pat) in patterns.iter().enumerate() {
                #[allow(clippy::collapsible_if)]
                if let Pattern::Ident(pat_name) = &pat.node {
                    if pat_name == arg_name {
                        hints[pat_idx] = Some(ty.clone());
                    }
                }
            }
        }
    }

    // Mescla ascription_hints: se partial dispatch resolveu uma posição, usa
    // o resultado do dispatch. Se não resolveu mas a ascription forneceu tipo,
    // usa o tipo da ascription.
    for i in 0..hints.len() {
        if hints[i].is_none() {
            hints[i] = ascription_hints[i].clone();
        }
    }

    // Só retorna se todos os parâmetros receberam tipos
    if hints.iter().all(|h| h.is_some()) {
        hints
            .into_iter()
            .map(|h| h.expect("checked above"))
            .collect()
    } else {
        Vec::new()
    }
}
