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
use kata_diagnostics::MiddleError;

/// Resultado de unificação — mapa de type param → tipo concreto.
pub type Substitutions = HashMap<String, Ty>;

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
) -> Result<(), MiddleError> {
    for (param, arg) in params.iter().zip(args) {
        unify_one(param, arg, type_params, subs)?;
    }
    Ok(())
}

/// Unifica um único par (param, arg).
fn unify_one(
    param: &Ty,
    arg: &Ty,
    type_params: &[String],
    subs: &mut Substitutions,
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
        (Ty::Interface(name), _) if type_params.contains(name) => {
            if let Some(existing) = subs.get(name) {
                if existing != arg {
                    return Err(MiddleError::TypeMismatch {
                        expected: format!("{}", existing),
                        found: format!("{}", arg),
                        span: kata_ast::Span::synthetic().into(),
                    });
                }
            } else {
                subs.insert(name.clone(), arg.clone());
            }
            Ok(())
        }

        // Generic: unifica recursivamente os argumentos de tipo
        (Ty::Generic(n1, ps), Ty::Generic(n2, as_)) if n1 == n2 && ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                unify_one(p, a, type_params, subs)?;
            }
            Ok(())
        }

        // Ty::Var que não é type param (ex: "Self") — aceita qualquer arg
        // (mesma semântica de fits_return)
        (Ty::Var(_), _) => Ok(()),

        // List/Array/Range — unifica recursivamente o elem_ty.
        (Ty::List(p), Ty::List(a)) => unify_one(p, a, type_params, subs),
        (Ty::Array(p), Ty::Array(a)) => unify_one(p, a, type_params, subs),
        (Ty::Range(p), Ty::Range(a)) => unify_one(p, a, type_params, subs),
        // Dict — unifica recursivamente K e V.
        (Ty::Dict(pk, pv), Ty::Dict(ak, av)) => {
            unify_one(pk, ak, type_params, subs)?;
            unify_one(pv, av, type_params, subs)
        }
        // Set — unifica recursivamente o elem_ty.
        (Ty::Set(p), Ty::Set(a)) => unify_one(p, a, type_params, subs),
        // Sender/Receiver/ReceiverFactory — unifica o tipo do canal.
        (Ty::Sender(p), Ty::Sender(a)) => unify_one(p, a, type_params, subs),
        (Ty::Receiver(p), Ty::Receiver(a)) => unify_one(p, a, type_params, subs),
        (Ty::ReceiverFactory(p), Ty::ReceiverFactory(a)) => unify_one(p, a, type_params, subs),

        // Generic("Dict", [K, V]) unifica com Ty::Dict(ak, av):
        // O prelude usa `Dict::(K, V)` que vira Generic("Dict", [Var("K"), Var("V")]).
        // O typeck produz Ty::Dict(Text, Int). Precisamos casar structuralmente.
        (Ty::Generic(n, ps), Ty::Dict(ak, av)) if n == "Dict" && ps.len() == 2 => {
            unify_one(&ps[0], ak, type_params, subs)?;
            unify_one(&ps[1], av, type_params, subs)
        }
        // Generic("Set", [T]) unifica com Ty::Set(a).
        (Ty::Generic(n, ps), Ty::Set(a)) if n == "Set" && ps.len() == 1 => {
            unify_one(&ps[0], a, type_params, subs)
        }
        // Ty::Dict unifica com Generic("Dict", ...) — caminho reverso.
        (Ty::Dict(pk, pv), Ty::Generic(n, as_)) if n == "Dict" && as_.len() == 2 => {
            unify_one(pk, &as_[0], type_params, subs)?;
            unify_one(pv, &as_[1], type_params, subs)
        }
        // Ty::Set unifica com Generic("Set", ...) — caminho reverso.
        (Ty::Set(p), Ty::Generic(n, as_)) if n == "Set" && as_.len() == 1 => {
            unify_one(p, &as_[0], type_params, subs)
        }

        // Tuple — unifica recursivamente cada elemento.
        (Ty::Tuple(ps), Ty::Tuple(as_)) if ps.len() == as_.len() => {
            for (p, a) in ps.iter().zip(as_) {
                unify_one(p, a, type_params, subs)?;
            }
            Ok(())
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
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|e| apply_subs(e, subs)).collect()),
        // List/Array/Range — substitui no elem_ty.
        Ty::List(elem) => Ty::List(Box::new(apply_subs(elem, subs))),
        Ty::Array(elem) => Ty::Array(Box::new(apply_subs(elem, subs))),
        Ty::Range(elem) => Ty::Range(Box::new(apply_subs(elem, subs))),
        // Dict — substitui em K e V.
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(apply_subs(k, subs)),
            Box::new(apply_subs(v, subs)),
        ),
        // Set — substitui no elem_ty.
        Ty::Set(elem) => Ty::Set(Box::new(apply_subs(elem, subs))),
        // Sender/Receiver/ReceiverFactory — substitui no tipo do canal.
        Ty::Sender(elem) => Ty::Sender(Box::new(apply_subs(elem, subs))),
        Ty::Receiver(elem) => Ty::Receiver(Box::new(apply_subs(elem, subs))),
        Ty::ReceiverFactory(elem) => Ty::ReceiverFactory(Box::new(apply_subs(elem, subs))),
        _ => ty.clone(),
    }
}
