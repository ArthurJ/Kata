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
pub fn resolve_type_expr(expr: &TypeExpr, env: &TypeEnv, iface_reg: &InterfaceRegistry) -> Ty {
    match expr {
        TypeExpr::Named(name) => {
            // Verifica ambiguidade primeiro — se o nome está marcado como
            // ambíguo (conflito de origin entre imports), o usuário deve
            // qualificar com `module.Type`.
            if env.is_ambiguous(name) {
                // Tenta encontrar um binding local (origin do módulo atual).
                // Se existe, local shadowa imports — não é ambíguo.
                // Se não existe local, é ambíguo de fato.
                let has_local = env
                    .lookup_binding(name)
                    .is_some_and(|b| b.origin == "__local__");
                if !has_local {
                    // Coleta origins para mensagem de erro.
                    let origins: Vec<String> = env
                        .local_bindings_full()
                        .filter(|(n, _)| *n == name)
                        .map(|(_, b)| b.origin.clone())
                        .collect();
                    eprintln!(
                        "Ambiguous type '{name}' — imported from: {}. \
                         Qualify with module.{name}.",
                        origins.join(", ")
                    );
                    // Fallback: usa o primeiro binding encontrado (comportamento
                    // anterior) para não quebrar compilação em testes existentes.
                    // O erro acima é informativo; a desambiguação real
                    // exigirá `module.Type` quando o parser suportar.
                }
            }
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
                            // UPPER_CASE sem :: é type param (ex: T, E, A).
                            Ty::Var(name.clone())
                        } else {
                            Ty::Struct(name.clone()) // fallback: tipo declarado pelo usuário
                        }
                    }
                }
            }
        }
        TypeExpr::Qualified { module, name } => {
            // `module.Type` — procura binding onde name == name && origin == module.
            if let Some(binding) = env.lookup_binding(name)
                && binding.origin == *module
            {
                return binding.ty.clone();
            }
            // Tenta no escopo local do módulo — pode ter sido copiado
            // com nome qualificado `module.Type` no merge_imports.
            let qual_name = format!("{module}.{name}");
            if let Some(ty) = env.lookup(&qual_name) {
                return ty.clone();
            }
            // Fallback: se é um tipo primitivo conhecido (ex: core.Int),
            // resolve pelo nome sem qualificar.
            match name.as_str() {
                "Int" => Ty::Prim(PrimTy::Int),
                "Float" => Ty::Prim(PrimTy::Float),
                "Text" => Ty::Prim(PrimTy::Text),
                "Rational" => Ty::Prim(PrimTy::Rational),
                "Boolean" => Ty::Sum("Boolean".into()),
                "Unit" => Ty::Unit,
                _ => {
                    // Tenta interface registry
                    if iface_reg.get_interface(name).is_some() {
                        Ty::Interface(name.clone())
                    } else {
                        Ty::Struct(name.clone())
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
        TypeExpr::ActionType { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env, iface_reg))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env, iface_reg);
            Ty::Action(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, params } => {
            // Result::(Int, Text) → resolve params → Ty::Generic("Result", [Int, Text]).
            // Se o enum é genérico no EnumRegistry, produz Ty::Generic.
            // Se não é genérico (fallback), produz Ty::Sum como antes.
            let resolved_params: Vec<Ty> = params
                .iter()
                .map(|p| resolve_type_expr(&p.node, env, iface_reg))
                .collect();
            // Tipos intrínsecos de coleção — List::(A), Array::(A), Range::(A).
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
                "Dict" => {
                    let mut params = resolved_params.into_iter();
                    let key = params
                        .next()
                        .expect("Dict::(K, V) exige exatamente 2 params");
                    let val = params
                        .next()
                        .expect("Dict::(K, V) exige exatamente 2 params");
                    Ty::Dict(Box::new(key), Box::new(val))
                }
                "Set" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("Set::(A) exige exatamente 1 param");
                    Ty::Set(Box::new(elem))
                }
                // Tuple::(Int, Text) → Ty::Tuple([Int, Text]).
                // Permite anotar tuplas em posições de tipo (ex: Sender::Tuple::(Int, Int)).
                "Tuple" => Ty::Tuple(resolved_params),
                // Tipos intrínsecos de canal.
                "Sender" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("Sender::(A) exige exatamente 1 param");
                    Ty::Sender(Box::new(elem))
                }
                "Receiver" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("Receiver::(A) exige exatamente 1 param");
                    Ty::Receiver(Box::new(elem))
                }
                "ReceiverFactory" => {
                    let elem = resolved_params
                        .into_iter()
                        .next()
                        .expect("ReceiverFactory::(A) exige exatamente 1 param");
                    Ty::ReceiverFactory(Box::new(elem))
                }
                _ => {
                    // Tenta resolver como Ty::Var se o param é um nome que não está no TypeEnv
                    // (ex: "T" em Result::(T, E) dentro de uma declaração de função genérica).
                    Ty::Generic(name.clone(), resolved_params)
                }
            }
        }
        // Self é resolvido na resolution de implements.
        // Por ora, mapeia para Ty::Var("Self") como placeholder.
        TypeExpr::SelfRef => Ty::Var("Self".into()),

        // `T?` — açúcar sintático para `Result::(T, Err)`.
        // Err é Text (mensagens de erro), consistente com o construtor
        // falível de constructors_refined.rs (D13 do PRD-refines).
        TypeExpr::Question(inner) => {
            let inner_ty = resolve_type_expr(&inner.node, env, iface_reg);
            Ty::Generic("Result".into(), vec![inner_ty, Ty::text()])
        }
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

/// Infere o tipo do payload a partir do literal do valor fixo.
/// `200` → Int, `3.14` → Float, `"hello"` → Text.
pub(crate) fn infer_payload_ty_from_literal(expr: &Expr) -> Option<Ty> {
    match expr {
        Expr::IntLit { .. } => Some(Ty::Prim(PrimTy::Int)),
        Expr::FloatLit { .. } => Some(Ty::Prim(PrimTy::Float)),
        Expr::TextLit { .. } => Some(Ty::Prim(PrimTy::Text)),
        _ => None,
    }
}

/// Verifica se um nome é um type param.
///
/// Convenção: UPPER_CASE (todas as letras maiúsculas, pelo menos 1 char).
/// `T`, `E`, `A` → true. `Int`, `Complex`, `NUM` → false (tem minúsculas).
/// `Self` → false (não é type param genérico, é placeholder de interface).
pub(crate) fn is_type_param_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_uppercase()) && !name.is_empty() && name != "Self"
}

/// Coleta type params de uma assinatura resolvida.
///
/// Percorre param_types e return_type recursivamente buscando `Ty::Var(name)`
/// onde `name` é UPPER_CASE, e `Ty::Interface(name)` (interfaces usadas como
/// params de função/action — habilita monomorfização por interface).
/// Recursa em `Ty::Generic` args. Remove duplicatas, preservando ordem de
/// primeira ocorrência.
pub fn collect_type_params(param_types: &[Ty], return_type: &Ty) -> Vec<String> {
    fn collect_into(ty: &Ty, result: &mut Vec<String>) {
        match ty {
            Ty::Var(name) if is_type_param_name(name) && !result.contains(name) => {
                result.push(name.clone());
            }
            // Interface como param de função/action — coleta o nome para
            // monomorfização por interface (ex: `echo :: SHOW => Unit`).
            Ty::Interface(name) if !result.contains(name) => {
                result.push(name.clone());
            }
            Ty::Generic(_, args) => {
                for arg in args {
                    collect_into(arg, result);
                }
            }
            // Coleções intrínsecas: recursar no tipo do elemento.
            Ty::List(inner) | Ty::Array(inner) | Ty::Range(inner) | Ty::Set(inner) => {
                collect_into(inner, result);
            }
            Ty::Dict(key, val) => {
                collect_into(key, result);
                collect_into(val, result);
            }
            // Canais: recursar no tipo do canal.
            Ty::Sender(inner) | Ty::Receiver(inner) | Ty::ReceiverFactory(inner) => {
                collect_into(inner, result);
            }
            Ty::Tuple(elements) => {
                for elem in elements {
                    collect_into(elem, result);
                }
            }
            Ty::Function(params, ret) => {
                for p in params {
                    collect_into(p, result);
                }
                collect_into(ret, result);
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
