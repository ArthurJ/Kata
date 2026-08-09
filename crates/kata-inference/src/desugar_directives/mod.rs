//! Desugaring de diretivas customizadas — inlining de bodies.
//!
//! Passada separada entre `resolve` e `infer_module` no driver.
//! Transforma a AST (`Module`) expandindo diretivas customizadas (`@nome`)
//! aplicadas em `Item::ActionDecl` e `Item::Sig`, inlineando o body da diretiva
//! conforme o Hook (Enter, Exit, ShortCircuit, Transform).
//!
//! O desugaring produz AST expandida que o typeck valida normalmente.
//! As variáveis de reflexão (`_name`, `_arity`, `_types`, `_return_type`,
//! `_is_action`, `_args`, `_return`) são sintetizadas como `let` bindings.

mod action_hooks;
mod lambda_hooks;
mod reflection;
mod transform;

use kata_resolution::ResolvedModule;

use self::reflection::ReflectionInfo;

/// Desugara diretivas customizadas em um `ResolvedModule`, aplicando inlining
/// nos bodies das actions e cláusulas das funções que têm diretivas customizadas.
///
/// Deve ser chamado entre `resolve` (que popula `DirectiveRegistry` e
/// `custom_directives` em ActionDef/FunctionDef) e `infer_module`.
pub fn desugar_directives(resolved: &mut ResolvedModule) {
    let registry = &resolved.directive_registry;

    // Actions: aplicar inlining nos bodies.
    for action in &mut resolved.actions {
        if action.custom_directives.is_empty() {
            continue;
        }
        let refl = ReflectionInfo::for_action(
            &action.name,
            &action.param_types,
            &action.param_names,
            &action.return_type,
        );
        action.body = action_hooks::apply_directives_to_action_body(
            std::mem::take(&mut action.body),
            &action.custom_directives,
            &refl,
            registry,
        );
    }

    // Functions: aplicar inlining nas cláusulas lambda.
    for func in &mut resolved.functions {
        if func.custom_directives.is_empty() {
            continue;
        }
        let refl = ReflectionInfo::for_function(&func.name, &func.param_types, &func.return_type);
        for clause in &mut func.clauses {
            clause.node = lambda_hooks::apply_directives_to_lambda_clause(
                clause.node.clone(),
                &func.custom_directives,
                &refl,
                registry,
            );
        }
    }
}
