//! Conversão `Ty` → `kata_rt::TypeShape` e registro da type table no runtime.
//!
//! O runtime (`kata-rt::marshal`) mantém uma type table TLS indexada por
//! `type_id` (usize). `to_bytes`/`from_bytes` consultam essa tabela para
//! saber como caminhar a estrutura de um valor em runtime.
//!
//! Este módulo faz a ponte: coleta todos os `Ty` que aparecem no módulo,
//! converte cada um para `TypeShape` (marshal), e registra a tabela antes
//! do JIT. É `pub` para que testes E2E em `kata-codegen/tests/` possam
//! importá-lo — `kata-driver` é crate binária e não pode ser importada.

use kata_core::enum_registry::EnumRegistry;
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::{PrimTy, Ty};
use kata_monomorph::MonoModule;
use kata_rt::TypeShape;
use std::collections::HashMap;

/// Converte `Ty` → `kata_rt::TypeShape` (formato de marshalling).
///
/// Usa `StructRegistry` e `EnumRegistry` para resolver campos de structs
/// e variants de enums. Tipos que não têm representação em marshal
/// (InferVar, Var, Interface) mapeiam para `Unit` graceful.
pub(crate) fn ty_to_marshal_shape(
    ty: &Ty,
    structs: &StructRegistry,
    enums: &EnumRegistry,
) -> TypeShape {
    match ty {
        // Primitivos inline: Int, Float, Byte — 8 bytes.
        Ty::Prim(PrimTy::Int) | Ty::Prim(PrimTy::Float) | Ty::Byte => TypeShape::Prim,
        // Text — C string na arena (ponteiro).
        Ty::Prim(PrimTy::Text) => TypeShape::Text,
        // Rational — ponteiro para kata_rt_rat na arena. Marshal como Prim
        // (copia os 8 bytes do ponteiro; o receptor faz deref via COW ou
        // to_bytes walk do conteúdo apontado).
        // FIXME: Rational deveria ter seu próprio variant ou serializar o
        // conteúdo. Por ora, trata como Prim (copia ponteiro).
        Ty::Prim(PrimTy::Rational) => TypeShape::Prim,
        // Unit — zero bytes.
        Ty::Unit => TypeShape::Unit,
        // Bytes — blob contíguo (ponteiro na arena).
        Ty::Bytes => TypeShape::Bytes,
        // Tupla — elementos heterogêneos, cada um 8 bytes.
        Ty::Tuple(elements) => TypeShape::Tuple(
            elements
                .iter()
                .map(|t| ty_to_marshal_shape(t, structs, enums))
                .collect(),
        ),
        // Struct — campos em ordem de declaração, cada um 8 bytes.
        Ty::Struct(name) => {
            let info = structs.get(name);
            match info {
                Some(info) => {
                    if let Some(alias_of) = &info.alias_of {
                        // Alias de outro tipo — resolve o target.
                        let target = Ty::Struct(alias_of.clone());
                        ty_to_marshal_shape(&target, structs, enums)
                    } else {
                        TypeShape::Struct(
                            info.fields
                                .iter()
                                .map(|f| ty_to_marshal_shape(&f.ty, structs, enums))
                                .collect(),
                        )
                    }
                }
                None => TypeShape::Struct(Vec::new()),
            }
        }
        // Sum (enum sem type params) — variants com payload opcional.
        Ty::Sum(name) => {
            let variants = enums.all_variants(name);
            match variants {
                Some(vs) => TypeShape::Sum(
                    vs.iter()
                        .map(|v| {
                            v.payload_ty
                                .as_ref()
                                .map(|ty| Box::new(ty_to_marshal_shape(ty, structs, enums)))
                        })
                        .collect(),
                ),
                None => TypeShape::Sum(Vec::new()),
            }
        }
        // Generic instanciado (ex: Result<Int, Text>) — trata como Sum,
        // substituindo type params pelos args concretos.
        Ty::Generic(name, args) => {
            let variants = enums.all_variants(name);
            let type_params = enums.type_params_of(name);
            match (variants, type_params) {
                (Some(vs), Some(params)) => {
                    let subst = build_subst(params, args);
                    TypeShape::Sum(
                        vs.iter()
                            .map(|v| {
                                v.payload_ty.as_ref().map(|ty| {
                                    let resolved = apply_subst(ty, &subst);
                                    Box::new(ty_to_marshal_shape(&resolved, structs, enums))
                                })
                            })
                            .collect(),
                    )
                }
                _ => TypeShape::Sum(Vec::new()),
            }
        }
        // List — Cons cells (head: 8 bytes, tail: ptr|0).
        Ty::List(elem) => TypeShape::List(Box::new(ty_to_marshal_shape(elem, structs, enums))),
        // Array — contíguo (len: i64, elements: i64 cada).
        Ty::Array(elem) => TypeShape::Array(Box::new(ty_to_marshal_shape(elem, structs, enums))),
        // Dict — HAMT de pares chave-valor. Marshal como Struct com 2 fields
        // (key_type, value_type) — estrutura opaca para o walk.
        Ty::Dict(k, v) => TypeShape::Struct(vec![
            ty_to_marshal_shape(k, structs, enums),
            ty_to_marshal_shape(v, structs, enums),
        ]),
        // Set — HAMT de chaves. Marshal como Struct com 1 field (elem_type).
        Ty::Set(elem) => TypeShape::Struct(vec![ty_to_marshal_shape(elem, structs, enums)]),
        // Range — start, step, end. Marshal como Struct com 3 fields.
        Ty::Range(elem) => TypeShape::Struct(vec![
            ty_to_marshal_shape(elem, structs, enums),
            ty_to_marshal_shape(elem, structs, enums),
            ty_to_marshal_shape(elem, structs, enums),
        ]),
        // Function/Action — fn ptr (8 bytes). Marshal como Prim.
        Ty::Function(_, _) | Ty::Action(_, _) => TypeShape::Prim,
        // Sender/Receiver/ReceiverFactory — handles de canal (8 bytes).
        Ty::Sender(_) | Ty::Receiver(_) | Ty::ReceiverFactory(_) => TypeShape::Prim,
        // File — handle opaco (8 bytes). Marshal como Prim.
        Ty::File => TypeShape::Prim,
        // Socket — handle opaco (8 bytes). Marshal como Prim.
        Ty::Socket => TypeShape::Prim,
        // InferVar/Var/Interface — não deveriam aparecer em runtime.
        // Mapeia para Unit graceful (não deve ser marshal'd em produção).
        Ty::InferVar(_) | Ty::Var(_) | Ty::Interface(_) => TypeShape::Unit,
        // OverloadSet: tipo interno do compilador, não tem representação runtime.
        Ty::OverloadSet { .. } => TypeShape::Unit,
    }
}

/// Constrói mapa de substituição type_param → arg concreto.
fn build_subst(params: &[String], args: &[Ty]) -> HashMap<String, Ty> {
    params
        .iter()
        .zip(args.iter())
        .map(|(p, a)| (p.clone(), a.clone()))
        .collect()
}

/// Substitui `Ty::Var(name)` pelo arg concreto do mapa.
fn apply_subst(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Var(name) => subst.get(name).cloned().unwrap_or(ty.clone()),
        Ty::List(elem) => Ty::List(Box::new(apply_subst(elem, subst))),
        Ty::Array(elem) => Ty::Array(Box::new(apply_subst(elem, subst))),
        Ty::Range(elem) => Ty::Range(Box::new(apply_subst(elem, subst))),
        Ty::Set(elem) => Ty::Set(Box::new(apply_subst(elem, subst))),
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(apply_subst(k, subst)),
            Box::new(apply_subst(v, subst)),
        ),
        Ty::Tuple(elements) => Ty::Tuple(elements.iter().map(|t| apply_subst(t, subst)).collect()),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(|t| apply_subst(t, subst)).collect(),
            Box::new(apply_subst(ret, subst)),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params.iter().map(|t| apply_subst(t, subst)).collect(),
            Box::new(apply_subst(ret, subst)),
        ),
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter().map(|t| apply_subst(t, subst)).collect(),
        ),
        Ty::Sender(elem) => Ty::Sender(Box::new(apply_subst(elem, subst))),
        Ty::Receiver(elem) => Ty::Receiver(Box::new(apply_subst(elem, subst))),
        Ty::ReceiverFactory(elem) => Ty::ReceiverFactory(Box::new(apply_subst(elem, subst))),
        // Tipos sem Var — clone direto.
        _ => ty.clone(),
    }
}

/// Coleta todos os `Ty` únicos que aparecem nos params e retornos das
/// functions e actions do módulo, incluindo sub-tipos de canais
/// (Sender/Receiver elem types) e tipos compostos (Tuple, List, etc).
/// Retorna vec na ordem de descoberta (que vira a ordem dos type_ids).
pub(crate) fn collect_module_types(mono: &MonoModule) -> Vec<Ty> {
    let mut seen: Vec<Ty> = Vec::new();

    fn insert_recursive(seen: &mut Vec<Ty>, ty: &Ty) {
        if !seen.contains(ty) {
            seen.push(ty.clone());
        }
        // Recursivamente coleta sub-tipos para que o type_id do
        // elemento do canal (ex: Tuple([Int, Int])) esteja disponível
        // no type_id_map, mesmo que só apareça dentro de Sender/Receiver.
        match ty {
            Ty::Sender(inner) | Ty::Receiver(inner) | Ty::ReceiverFactory(inner) => {
                insert_recursive(seen, inner);
            }
            Ty::List(elem) | Ty::Array(elem) | Ty::Range(elem) | Ty::Set(elem) => {
                insert_recursive(seen, elem);
            }
            Ty::Tuple(elems) => {
                for e in elems {
                    insert_recursive(seen, e);
                }
            }
            Ty::Dict(k, v) => {
                insert_recursive(seen, k);
                insert_recursive(seen, v);
            }
            Ty::Generic(_, args) => {
                for a in args {
                    insert_recursive(seen, a);
                }
            }
            Ty::Function(params, ret) | Ty::Action(params, ret) => {
                for p in params {
                    insert_recursive(seen, p);
                }
                insert_recursive(seen, ret);
            }
            _ => {}
        }
    }

    // Params e retornos de functions.
    for f in &mono.functions {
        for p in &f.param_types {
            insert_recursive(&mut seen, p);
        }
        insert_recursive(&mut seen, &f.ret_ty);
    }

    // Params e retornos de actions.
    for a in &mono.actions {
        for p in &a.param_types {
            insert_recursive(&mut seen, p);
        }
        insert_recursive(&mut seen, &a.ret_ty);
    }

    seen
}

/// Constrói e registra a type table no runtime.
///
/// Converte cada `Ty` coletado do módulo para `kata_rt::TypeShape` e
/// chama `register_type_table`. O `type_id` de cada tipo é seu índice
/// no Vec retornado — o codegen usa esse índice ao emitir chamadas
/// `to_bytes`.
///
/// Retorna o mapa `Ty` → `type_id` para o codegen.
pub fn build_and_register_type_table(
    mono: &MonoModule,
    structs: &StructRegistry,
    enums: &EnumRegistry,
) -> HashMap<Ty, i64> {
    let types = collect_module_types(mono);
    let shapes: Vec<TypeShape> = types
        .iter()
        .map(|ty| ty_to_marshal_shape(ty, structs, enums))
        .collect();
    let type_id_map: HashMap<Ty, i64> = types
        .iter()
        .enumerate()
        .map(|(i, ty)| (ty.clone(), i as i64))
        .collect();

    kata_rt::register_type_table(shapes);
    type_id_map
}
