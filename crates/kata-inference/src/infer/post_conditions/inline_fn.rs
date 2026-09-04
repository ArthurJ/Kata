// ── Inlining de funções puras no Z3 translator (§9.10) ──────────────

use std::collections::HashMap;

use kata_core::ty::{Ty, TypeEnv};
use kata_resolution::FunctionDef;

use crate::typed::TypedExpr;

use super::super::expr::{InferCtx, infer_expr_hinted};
use super::super::helpers::{check_patterns, process_with_bindings};
use super::extract_param_names;

/// Corpo tipado de uma função pura, para inlining no Z3 translator.
///
/// Quando o translator encontra `Closure { Ident(f), args }` e `f` está
/// nesta tabela, ele substitui os params pelos args (`substitute_params`)
/// e traduz o resultado. Isso torna funções puras com corpo Kata
/// transparentes para o Z3 — sem codificar semântica de cada função
/// individualmente no translator.
#[derive(Debug, Clone)]
pub(crate) struct InlineFnBody {
    /// Nomes dos parâmetros (da primeira cláusula).
    pub param_names: Vec<String>,
    /// Tipos dos parâmetros (para desambiguar overloads).
    pub param_types: Vec<Ty>,
    /// Corpo tipado da cláusula (quando há uma única cláusula sem guards).
    /// `None` se a função tem múltiplas cláusulas ou guards — não inlinable.
    pub body: Option<TypedExpr>,
}

/// Tabela de funções puras inlinable, por nome.
#[derive(Debug, Clone, Default)]
pub(crate) struct InlineFnTable {
    map: HashMap<String, Vec<InlineFnBody>>,
}

impl InlineFnTable {
    /// Consulta o corpo tipado de uma função pelo nome, escolhendo o
    /// overload cujos param types são compatíveis com os args da chamada.
    pub(crate) fn get(&self, name: &str, arg_types: &[Ty]) -> Option<&InlineFnBody> {
        let overloads = self.map.get(name)?;
        // Match por arity + compatibilidade de tipos.
        overloads.iter().find(|body| {
            body.param_types.len() == arg_types.len()
                && body
                    .param_types
                    .iter()
                    .zip(arg_types.iter())
                    .all(|(param, arg)| param == arg)
        })
    }

    /// True se a tabela está vazia.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Extrai corpos tipados de funções puras simples (uma cláusula, sem guards,
/// sem `@ffi`) para inlining no Z3 translator.
///
/// Segue o mesmo padrão de `extract_post_conditions`: usa um `InferCtx`
/// temporário para tipar o corpo. Conservador: se a tipagem falha ou a
/// função não é simples, skip.
pub(crate) fn extract_inline_bodies(
    functions: &[FunctionDef],
    ctx: &InferCtx,
    module_type_env: &TypeEnv,
) -> InlineFnTable {
    let mut map: HashMap<String, Vec<InlineFnBody>> = HashMap::new();

    for func_def in functions {
        // Só inlina funções puras simples: uma cláusula, sem guards.
        if func_def.clauses.len() != 1 {
            continue;
        }
        let clause = &func_def.clauses[0].node;
        if !clause.guards.is_empty() {
            continue;
        }

        let param_names = extract_param_names(&clause.patterns);

        // Tipa o body usando um escopo filho com os tipos do módulo.
        let mut clause_env = TypeEnv::with_parent(module_type_env.clone());

        // Desugar o body (elimina Pipe/Hole antes do typeck).
        let desugared_body = crate::desugar::desugar(&clause.body);

        // Check patterns para definir bindings no escopo da cláusula.
        if check_patterns(
            &clause.patterns,
            &func_def.param_types,
            ctx.enum_registry,
            &mut clause_env,
            ctx.interface_registry,
            ctx.struct_registry,
            ctx.refined_decls,
        )
        .is_err()
        {
            continue;
        }

        // Process with bindings (pode afetar tipagem dos guards).
        let desugared_with_bindings: Vec<kata_ast::WithBinding> = clause
            .with_bindings
            .iter()
            .map(|w| kata_ast::WithBinding {
                name: w.name.clone(),
                value: crate::desugar::desugar(&w.value),
            })
            .collect();
        if process_with_bindings(&desugared_with_bindings, &mut clause_env, ctx).is_err() {
            continue;
        }

        // Tipa o body.
        let body_typed = infer_expr_hinted(
            &desugared_body.node,
            &desugared_body.span,
            &mut clause_env,
            ctx,
            false,
            None,
        );

        match body_typed {
            Ok(typed) => {
                map.entry(func_def.name.clone())
                    .or_default()
                    .push(InlineFnBody {
                        param_names,
                        param_types: func_def.param_types.clone(),
                        body: Some(typed),
                    });
            }
            Err(_) => continue,
        }
    }

    InlineFnTable { map }
}
