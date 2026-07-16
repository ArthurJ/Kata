//! Funções auxiliares de conversão de tipos (TypeExpr → Ty).
//!
//! Funções self-contained usadas pelo pass de resolution:
//! - `resolve_type_expr`: converte `TypeExpr` → `Ty` usando `TypeEnv`
//! - `infer_payload_ty_from_pred`: infere tipo do payload a partir do predicado
//! - `is_type_param_name`: verifica se um nome é um type param (UPPER_CASE)
//! - `collect_type_params`: coleta type params de uma assinatura resolvida

use kata_ast::{Expr, TypeExpr};
use kata_core::{InterfaceRegistry, PrimTy, Ty, TypeEnv};

/// Converte TypeExpr → Ty usando TypeEnv para resolver nomes.
///
/// Se `name` é uma interface registrada no `InterfaceRegistry`, produz
/// `Ty::Interface(name)` em vez de `Ty::Struct(name)`.
pub(crate) fn resolve_type_expr(
    expr: &TypeExpr,
    env: &TypeEnv,
    iface_reg: &InterfaceRegistry,
) -> Ty {
    match expr {
        TypeExpr::Named(name) => {
            // Tenta resolver no TypeEnv
            if let Some(ty) = env.lookup(name) {
                ty.clone()
            } else {
                // Tipos conhecidos do prelude
                match name.as_str() {
                    "Int" => Ty::Prim(PrimTy::Int),
                    "Float" => Ty::Prim(PrimTy::Float),
                    "Text" => Ty::Prim(PrimTy::Text),
                    "Rational" => Ty::Prim(PrimTy::Rational),
                    "Boolean" => Ty::Sum("Boolean".into()),
                    "Unit" => Ty::Unit,
                    _ => {
                        // Se é uma interface registrada, produz Ty::Interface.
                        if iface_reg.get_interface(name).is_some() {
                            Ty::Interface(name.clone())
                        } else if is_type_param_name(name) {
                            // Fase 5: UPPER_CASE sem :: é type param (ex: T, E, A).
                            Ty::Var(name.clone())
                        } else {
                            Ty::Struct(name.clone()) // fallback: tipo declarado pelo usuário
                        }
                    }
                }
            }
        }
        TypeExpr::Unit => Ty::Unit,
        TypeExpr::Grouping(inner) => resolve_type_expr(&inner.node, env, iface_reg),
        TypeExpr::Tuple(elements) => {
            let tys: Vec<Ty> = elements
                .iter()
                .map(|t| resolve_type_expr(&t.node, env, iface_reg))
                .collect();
            Ty::Tuple(tys)
        }
        TypeExpr::Func { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env, iface_reg))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env, iface_reg);
            Ty::Function(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, params } => {
            // Fase 6: Result::(Int, Text) → resolve params → Ty::Generic("Result", [Int, Text]).
            // Se o enum é genérico no EnumRegistry, produz Ty::Generic.
            // Se não é genérico (fallback), produz Ty::Sum como antes.
            let resolved_params: Vec<Ty> = params
                .iter()
                .map(|p| resolve_type_expr(&p.node, env, iface_reg))
                .collect();
            // Fio 8: tipos intrínsecos de coleção — List::(A), Array::(A), Range::(A).
            // São variants de Ty, não Ty::Generic. O codegen precisa do layout.
            // O parser sempre produz pelo menos 1 param em ParamApp, então
            // .expect() aqui é uma asserção de invariant, não um path de erro.
            match name.as_str() {
                "List" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("List::(A) exige exatamente 1 param");
                    Ty::List(Box::new(elem))
                }
                "Array" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("Array::(A) exige exatamente 1 param");
                    Ty::Array(Box::new(elem))
                }
                "Range" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("Range::(A) exige exatamente 1 param");
                    Ty::Range(Box::new(elem))
                }
                _ => {
                    // Tenta resolver como Ty::Var se o param é um nome que não está no TypeEnv
                    // (ex: "T" em Result::(T, E) dentro de uma declaração de função genérica).
                    Ty::Generic(name.clone(), resolved_params)
                }
            }
        }
        // Fio 7: Self é resolvido na Fase 2 (resolution de implements).
        // Por ora, mapeia para Ty::Var("Self") como placeholder.
        TypeExpr::SelfRef => Ty::Var("Self".into()),
    }
}

/// Infere o tipo do payload a partir do predicado da variante.
///
/// `Magreza(< _ 18.5)` → predicado `Apply { Ident("<"), [Hole, FloatLit("18.5")] }`
/// → o tipo do payload é o tipo do literal (`Float`).
///
/// Suporta predicados no formato `op _ literal` (Apply com callee Ident e args [Hole, literal]).
pub(crate) fn infer_payload_ty_from_pred(expr: &Expr) -> Option<Ty> {
    if let Expr::Apply { callee, args } = expr {
        // callee deve ser Ident (operador)
        if matches!(callee.node, Expr::Ident { .. }) {
            // args[0] deve ser Hole, args[1] deve ser literal
            if args.len() == 2 && matches!(args[0].node, Expr::Hole) {
                return match &args[1].node {
                    Expr::IntLit { .. } => Some(Ty::Prim(PrimTy::Int)),
                    Expr::FloatLit { .. } => Some(Ty::Prim(PrimTy::Float)),
                    Expr::TextLit { .. } => Some(Ty::Prim(PrimTy::Text)),
                    _ => None,
                };
            }
        }
    }
    None
}

/// Fase 5: verifica se um nome é um type param.
///
/// Convenção: UPPER_CASE (todas as letras maiúsculas, pelo menos 1 char).
/// `T`, `E`, `A` → true. `Int`, `Complex`, `NUM` → false (tem minúsculas).
/// `Self` → false (não é type param genérico, é placeholder de interface).
pub(crate) fn is_type_param_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_uppercase()) && !name.is_empty() && name != "Self"
}

/// Fase 5: coleta type params de uma assinatura resolvida.
///
/// Percorre param_types e return_type recursivamente buscando `Ty::Var(name)`
/// onde `name` é UPPER_CASE. Recursa em `Ty::Generic` args. Remove duplicatas,
/// preservando ordem de primeira ocorrência.
pub(crate) fn collect_type_params(param_types: &[Ty], return_type: &Ty) -> Vec<String> {
    fn collect_into(ty: &Ty, result: &mut Vec<String>) {
        match ty {
            Ty::Var(name) if is_type_param_name(name) && !result.contains(name) => {
                result.push(name.clone());
            }
            Ty::Generic(_, args) => {
                for arg in args {
                    collect_into(arg, result);
                }
            }
            _ => {}
        }
    }
    let mut result: Vec<String> = Vec::new();
    for ty in param_types {
        collect_into(ty, &mut result);
    }
    collect_into(return_type, &mut result);
    result
}
