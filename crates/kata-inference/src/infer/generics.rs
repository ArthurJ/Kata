//! Unificação de type params genéricos.
//!
//! `unify` casa os tipos dos argumentos com os tipos dos parâmetros de uma
//! assinatura genérica, produzindo um mapa de substitutions que mapeia
//! type params (ex: `T`) para tipos concretos (ex: `Int`).
//!
//! Não é union-find — é casamento posicional top-down. Para cada par
//! `(param, arg)`:
//! - Se `param` é `Ty::Var(name)` e `name` está em `type_params`:
//!   - Se já tem substitution para `name`, verifica `arg == existing`
//!   - Se não, insere `name → arg`
//! - Se `param` é `Ty::Generic(n, ps)` e `arg` é `Ty::Generic(n', as)` com
//!   mesmo nome e mesma aridade: unifica cada sub-par recursivamente
//! - Caso contrário: verifica `param == arg` (match estrutural)
//!
//! Se qualquer par falha, retorna `Err` com o tipo esperado e o encontrado.

use std::collections::HashMap;

use kata_core::ty::Ty;
use kata_core::{InterfaceRegistry, PrimTy, RefinesRegistry, StructKey};
use kata_diagnostics::MiddleError;

/// Resultado de unificação — mapa de type param → tipo concreto.
pub type Substitutions = HashMap<String, Ty>;

/// Normaliza um arg refined para o tipo base quando o refined delega
/// a interface `interface_name` via `RefinesRegistry`.
///
/// Consulta delegações diretas apenas. Casos transitivos (supertraits)
/// caem no `try_refines_fallback` existente.
///
/// Retorna o tipo base se o arg é refined que delega a interface,
/// ou o arg inalterado caso contrário.
fn normalize_refined(arg: &Ty, interface_name: &str, refines_registry: &RefinesRegistry) -> Ty {
    // Extrai o nome do tipo do arg se é Ty::Struct.
    let type_name = match arg {
        Ty::Struct(key) => key.name(),
        _ => return arg.clone(),
    };
    // Consulta delegações diretas no RefinesRegistry.
    let entries = refines_registry.get(type_name);
    for entry in entries {
        if entry.interface_name == interface_name {
            return entry.base_ty.clone();
        }
    }
    arg.clone()
}

/// Extrai o nome de um tipo de `Ty` para consulta ao `InterfaceRegistry`.
/// Necessário porque o registry indexa por nome (`"Int"`, `"List"`, etc.),
/// não por `Ty` diretamente.
///
/// - `Ty::Prim(Int)` → `"Int"`, etc.
/// - `Ty::Struct(key)` → `key.name()` (Plain, Family, Instance).
/// - `Ty::Generic(name, _)` → `name` (família lazy como NonEmpty).
/// - `Ty::List(_)` → `"List"`, etc.
/// - Demais (`Ty::Var`, `Ty::Interface`, `Ty::Tuple`, ...) → `None`.
pub(crate) fn ty_name_for_iface_check(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Prim(PrimTy::Int) => Some("Int".into()),
        Ty::Prim(PrimTy::Float) => Some("Float".into()),
        Ty::Prim(PrimTy::Rational) => Some("Rational".into()),
        Ty::Prim(PrimTy::Text) => Some("Text".into()),
        Ty::Struct(key) => Some(key.name().to_string()),
        Ty::Generic(name, _) => Some(name.clone()),
        Ty::List(_) => Some("List".into()),
        Ty::Array(_) => Some("Array".into()),
        Ty::Range(_) => Some("Range".into()),
        Ty::Dict(_, _) => Some("Dict".into()),
        Ty::Set(_) => Some("Set".into()),
        Ty::Tensor(_) => Some("Tensor".into()),
        _ => None,
    }
}

/// Unifica os tipos dos argumentos com os tipos dos parâmetros de uma
/// assinatura genérica.
///
/// `type_params` lista os nomes que são type params (ex: `["T"]`).
/// A função preenche `subs` (mutável) e retorna `Ok(())` se todos os pares
/// casam, ou `Err(MiddleError::TypeMismatch)` se algum par é incompatível.
///
/// `subs` já pode conter bindings prévios (passados de cima); a função
/// apenas adiciona novos bindings e verifica consistência.
pub fn unify(
    params: &[Ty],
    args: &[Ty],
    type_params: &[String],
    subs: &mut Substitutions,
    refines_registry: &RefinesRegistry,
    iface_registry: &InterfaceRegistry,
) -> Result<(), MiddleError> {
    for (param, arg) in params.iter().zip(args) {
        unify_one(
            param,
            arg,
            type_params,
            subs,
            refines_registry,
            iface_registry,
        )?;
    }
    Ok(())
}

/// Unifica um único par (param, arg).
#[allow(clippy::only_used_in_recursion)]
fn unify_one(
    param: &Ty,
    arg: &Ty,
    type_params: &[String],
    subs: &mut Substitutions,
    refines_registry: &RefinesRegistry,
    iface_registry: &InterfaceRegistry,
) -> Result<(), MiddleError> {
    match (param, arg) {
        // Type param: Ty::Var("T") onde "T" está em type_params
        (Ty::Var(name), _) if type_params.contains(name) => {
            if let Some(existing) = subs.get(name) {
                // Já tem binding — verifica consistência
                if existing != arg {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{}", existing),
                        found: format!("{}", arg),
                        span: kata_ast::Span::synthetic().into(),
                    });
                }
            } else {
                // Novo binding
                subs.insert(name.clone(), arg.clone());
            }
            Ok(())
        }

        // Iface param: Ty::Interface("SHOW") onde "SHOW" está em type_params.
        // Mesma semântica de Ty::Var — insere nome_da_interface → tipo_concreto.
        // Habilita monomorfização de Actions/funções polimórficas por interface
        // (ex: `echo :: SHOW => Unit` instanciado para cada tipo concreto que
        // implementa SHOW).
        //
        // Antes de bindar, normaliza args refined: se o arg é um tipo refined
        // que delega a interface via `refines`, binda com o tipo base (ex:
        // PositiveInt refines NUM → binda NUM com Int, não PositiveInt).
        // Se o arg não implementa a interface (direta nem via refines),
        // retorna erro cedo ("Text não implementa NUM").
        (Ty::Interface(name), _) if type_params.contains(name) => {
            let normalized = normalize_refined(arg, name, refines_registry);
            let bind_ty = if normalized != *arg {
                // Arg é refined que delega — bindar com tipo base.
                normalized
            } else {
                // Arg não é refined que delega. A assinatura declara que o
                // parâmetro é uma interface — o arg deve implementá-la.
                // Se não implementa, rejeitar cedo com mensagem clara
                // ("Text não implementa NUM") em vez de bindar cegamente.
                if let Some(type_name) = ty_name_for_iface_check(arg)
                    && !iface_registry.type_implements(&type_name, name)
                {
                    return Err(MiddleError::TypeMismatch {
                        expected: name.to_string(),
                        found: format!("{type_name} — {type_name} não implementa {name}"),
                        span: kata_ast::Span::synthetic().into(),
                    });
                }
                // Tipo implementa a interface — bindar.
                arg.clone()
            };
            if let Some(existing) = subs.get(name) {
                if existing != &bind_ty {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{}", existing),
                        found: format!("{}", bind_ty),
                        span: kata_ast::Span::synthetic().into(),
                    });
                }
            } else {
                subs.insert(name.clone(), bind_ty);
            }
            Ok(())
        }

        // Generic: unifica recursivamente os argumentos de tipo
        (Ty::Generic(n1, ps), Ty::Generic(n2, as_)) if n1 == n2 && ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                unify_one(p, a, type_params, subs, refines_registry, iface_registry)?;
            }
            Ok(())
        }

        // Ty::Var que não é type param (ex: "Self") — aceita qualquer arg
        // (mesma semântica de fits_return)
        (Ty::Var(_), _) => Ok(()),

        // List/Array/Range — unifica recursivamente o elem_ty.
        (Ty::List(p), Ty::List(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        (Ty::Array(p), Ty::Array(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        (Ty::Range(p), Ty::Range(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        // Dict — unifica recursivamente K e V.
        (Ty::Dict(pk, pv), Ty::Dict(ak, av)) => {
            unify_one(pk, ak, type_params, subs, refines_registry, iface_registry)?;
            unify_one(pv, av, type_params, subs, refines_registry, iface_registry)
        }
        // Set — unifica recursivamente o elem_ty.
        (Ty::Set(p), Ty::Set(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        // Tensor — unifica recursivamente o elem_ty.
        (Ty::Tensor(p), Ty::Tensor(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        // Sender/Receiver/ReceiverFactory — unifica o tipo do canal.
        (Ty::Sender(p), Ty::Sender(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        (Ty::Receiver(p), Ty::Receiver(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }
        (Ty::ReceiverFactory(p), Ty::ReceiverFactory(a)) => {
            unify_one(p, a, type_params, subs, refines_registry, iface_registry)
        }

        // Generic("Dict", [K, V]) unifica com Ty::Dict(ak, av):
        // O prelude usa `Dict::(K, V)` que vira Generic("Dict", [Var("K"), Var("V")]).
        // O typeck produz Ty::Dict(Text, Int). Precisamos casar structuralmente.
        (Ty::Generic(n, ps), Ty::Dict(ak, av)) if n == "Dict" && ps.len() == 2 => {
            unify_one(
                &ps[0],
                ak,
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )?;
            unify_one(
                &ps[1],
                av,
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )
        }
        // Generic("Set", [T]) unifica com Ty::Set(a).
        (Ty::Generic(n, ps), Ty::Set(a)) if n == "Set" && ps.len() == 1 => unify_one(
            &ps[0],
            a,
            type_params,
            subs,
            refines_registry,
            iface_registry,
        ),
        // Ty::Dict unifica com Generic("Dict", ...) — caminho reverso.
        (Ty::Dict(pk, pv), Ty::Generic(n, as_)) if n == "Dict" && as_.len() == 2 => {
            unify_one(
                pk,
                &as_[0],
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )?;
            unify_one(
                pv,
                &as_[1],
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )
        }
        // Ty::Set unifica com Generic("Set", ...) — caminho reverso.
        (Ty::Set(p), Ty::Generic(n, as_)) if n == "Set" && as_.len() == 1 => unify_one(
            p,
            &as_[0],
            type_params,
            subs,
            refines_registry,
            iface_registry,
        ),

        // Tuple — unifica recursivamente cada elemento.
        (Ty::Tuple(ps), Ty::Tuple(as_)) if ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                unify_one(p, a, type_params, subs, refines_registry, iface_registry)?;
            }
            Ok(())
        }

        // Generic(family, [Var(param)]) vs Instance(family, concrete):
        // A assinatura `head :: NonEmpty::A => A` resolve para
        // Generic("NonEmpty", [Var("A")]) no DispatchTable. O argumento
        // `[1 2 3]::NonEmpty` produz Struct(Instance("NonEmpty", "Int")).
        // Este caso casa a família por nome e unifica Var("A") com o tipo
        // concreto extraído do Instance.
        (Ty::Generic(fam_p, ps), Ty::Struct(StructKey::Instance(fam_a, concrete_a)))
            if fam_p == fam_a && ps.len() == 1 =>
        {
            let arg_inner = match concrete_a.as_str() {
                "Int" => Ty::Prim(PrimTy::Int),
                "Float" => Ty::Prim(PrimTy::Float),
                "Rational" => Ty::Prim(PrimTy::Rational),
                "Text" => Ty::Prim(PrimTy::Text),
                other => Ty::Struct(StructKey::Plain(other.to_string())),
            };
            unify_one(
                &ps[0],
                &arg_inner,
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )
        }

        // Instance de família polimórfica com type var no concrete:
        // Instance("NonEmpty", "A") (param) vs Instance("NonEmpty", "Int") (arg).
        // Unifica o type var A → Int recursivamente.
        (
            Ty::Struct(StructKey::Instance(fam_p, concrete_p)),
            Ty::Struct(StructKey::Instance(fam_a, concrete_a)),
        ) if fam_p == fam_a => {
            // concrete_p é o nome do type param (ex: "A").
            // concrete_a é o nome do tipo concreto (ex: "Int").
            // Constrói o Ty do tipo concreto e unifica com A.
            let arg_inner = match concrete_a.as_str() {
                "Int" => Ty::Prim(PrimTy::Int),
                "Float" => Ty::Prim(PrimTy::Float),
                "Rational" => Ty::Prim(PrimTy::Rational),
                "Text" => Ty::Prim(PrimTy::Text),
                other => Ty::Struct(StructKey::Plain(other.to_string())),
            };
            let param_inner = Ty::Var(concrete_p.clone());
            unify_one(
                &param_inner,
                &arg_inner,
                type_params,
                subs,
                refines_registry,
                iface_registry,
            )
        }

        // Match estrutural para tipos concretos
        _ if param == arg => Ok(()),

        // Incompatível
        _ => Err(MiddleError::TypeMismatch {
            expected: format!("{}", param),
            found: format!("{}", arg),
            span: kata_ast::Span::synthetic().into(),
        }),
    }
}

/// Aplica substitutions em um tipo, substituindo `Ty::Var(name)` pelo tipo
/// concreto quando `name` está em `subs`.
///
/// Recursiva em `Generic` (substitui nos argumentos de tipo).
pub fn apply_subs(ty: &Ty, subs: &Substitutions) -> Ty {
    match ty {
        Ty::Var(name) => {
            if let Some(concrete) = subs.get(name) {
                concrete.clone()
            } else {
                ty.clone()
            }
        }
        // Iface param: substitui quando o nome da interface está no mapa.
        // Análogo a Ty::Var — habilita monomorfização de interfaces.
        Ty::Interface(name) => {
            if let Some(concrete) = subs.get(name) {
                concrete.clone()
            } else {
                ty.clone()
            }
        }
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter().map(|a| apply_subs(a, subs)).collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(|p| apply_subs(p, subs)).collect(),
            Box::new(apply_subs(ret, subs)),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params.iter().map(|p| apply_subs(p, subs)).collect(),
            Box::new(apply_subs(ret, subs)),
        ),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|e| apply_subs(e, subs)).collect()),
        // List/Array/Range — substitui no elem_ty.
        Ty::List(elem) => Ty::List(Box::new(apply_subs(elem, subs))),
        Ty::Array(elem) => Ty::Array(Box::new(apply_subs(elem, subs))),
        Ty::Range(elem) => Ty::Range(Box::new(apply_subs(elem, subs))),
        // Dict — substitui em K e V.
        Ty::Dict(k, v) => Ty::Dict(Box::new(apply_subs(k, subs)), Box::new(apply_subs(v, subs))),
        // Set — substitui no elem_ty.
        Ty::Set(elem) => Ty::Set(Box::new(apply_subs(elem, subs))),
        Ty::Tensor(elem) => Ty::Tensor(Box::new(apply_subs(elem, subs))),
        // Sender/Receiver/ReceiverFactory — substitui no tipo do canal.
        Ty::Sender(elem) => Ty::Sender(Box::new(apply_subs(elem, subs))),
        Ty::Receiver(elem) => Ty::Receiver(Box::new(apply_subs(elem, subs))),
        Ty::ReceiverFactory(elem) => Ty::ReceiverFactory(Box::new(apply_subs(elem, subs))),
        // Struct paramétrico: substitui nos type args.
        Ty::Struct(StructKey::Generic(name, args)) => {
            Ty::Struct(StructKey::Generic(
                name.clone(),
                args.iter().map(|a| apply_subs(a, subs)).collect(),
            ))
        }
        _ => ty.clone(),
    }
}
