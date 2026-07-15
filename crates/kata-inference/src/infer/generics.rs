//! Fase 5: Unificação de type params genéricos.
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
pub(crate) type Substitutions = HashMap<String, Ty>;

/// Unifica os tipos dos argumentos com os tipos dos parâmetros de uma
/// assinatura genérica.
///
/// `type_params` lista os nomes que são type params (ex: `["T"]`).
/// A função preenche `subs` (mutável) e retorna `Ok(())` se todos os pares
/// casam, ou `Err(MiddleError::TypeMismatch)` se algum par é incompatível.
///
/// `subs` já pode conter bindings prévios (passados de cima); a função
/// apenas adiciona novos bindings e verifica consistência.
pub(crate) fn unify(
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
                        expected: format!("{:?}", existing),
                        found: format!("{:?}", arg),
                        span: kata_ast::Span::synthetic().into(),
                    });
                }
            } else {
                // Novo binding
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

        // Match estrutural para tipos concretos
        _ if param == arg => Ok(()),

        // Incompatível
        _ => Err(MiddleError::TypeMismatch {
            expected: format!("{:?}", param),
            found: format!("{:?}", arg),
            span: kata_ast::Span::synthetic().into(),
        }),
    }
}

/// Aplica substitutions em um tipo, substituindo `Ty::Var(name)` pelo tipo
/// concreto quando `name` está em `subs`.
///
/// Recursiva em `Generic` (substitui nos argumentos de tipo).
pub(crate) fn apply_subs(ty: &Ty, subs: &Substitutions) -> Ty {
    match ty {
        Ty::Var(name) => {
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
        _ => ty.clone(),
    }
}
