//! Naming helpers — canonical string generation for monomorphized instances.
//!
//! Extraído de `instantiate.rs` para separar a responsabilidade de naming
//! (geração de nomes canônicos de instância) da responsabilidade de
//! instantiation (substituição recursiva de Ty::Var).

use kata_core::ty::Ty;
use kata_inference::Substitutions;

/// Gera uma string canônica para um mapa de substitutions.
///
/// Ordena por nome do type param para que `T=Int, E|Text` e `E|Text, T=Int`
/// produzam a mesma chave.
pub(crate) fn canonicalize_subs(type_params: &[String], subs: &Substitutions) -> String {
    let mut parts: Vec<String> = Vec::new();
    for param in type_params {
        if let Some(ty) = subs.get(param) {
            parts.push(format!("{param}_{}", ty_to_string(ty)));
        }
    }
    parts.join("_")
}

/// Converte um `Ty` para string canônica usada no nome da instância.
fn ty_to_string(ty: &Ty) -> String {
    match ty {
        Ty::Prim(p) => format!("{p:?}"),
        Ty::Var(name) => name.clone(),
        Ty::Generic(name, args) => {
            let args_str = args.iter().map(ty_to_string).collect::<Vec<_>>().join("_");
            format!("{name}_{args_str}")
        }
        Ty::List(elem) => format!("List_{}", ty_to_string(elem)),
        Ty::Array(elem) => format!("Array_{}", ty_to_string(elem)),
        Ty::Range(elem) => format!("Range_{}", ty_to_string(elem)),
        Ty::Sum(name) => name.clone(),
        Ty::Function(params, ret) => {
            let p = params
                .iter()
                .map(ty_to_string)
                .collect::<Vec<_>>()
                .join("_");
            format!("Fn_{p}_{}", ty_to_string(ret))
        }
        Ty::Action(params, ret) => {
            let p = params
                .iter()
                .map(ty_to_string)
                .collect::<Vec<_>>()
                .join("_");
            format!("Act_{p}_{}", ty_to_string(ret))
        }
        Ty::Tuple(elems) => {
            let e = elems.iter().map(ty_to_string).collect::<Vec<_>>().join("_");
            format!("Tup_{e}")
        }
        Ty::Unit => "Unit".to_string(),
        Ty::InferVar(_) => "Inf".to_string(),
        Ty::Interface(name) => format!("Iface_{name}"),
        _ => format!("{ty:?}"),
    }
}
