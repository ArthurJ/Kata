//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

pub(crate) mod module_loader;
mod pass0;
mod prelude_sigs;
mod type_resolve;
mod types;

pub use types::*;

use kata_ast::{Item, Module};
use kata_core::{Ty, TypeEnv};
use type_resolve::{collect_type_params, resolve_type_expr};

/// Resolve um módulo: Pass 0 + Pass 1.
pub fn resolve(module: &Module) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut type_env = TypeEnv::new();
    // Unit é tipo primitivo da linguagem — sempre disponível no TypeEnv.
    type_env.define("Unit", Ty::Unit);
    let mut signatures: Vec<Signature> = Vec::new();
    let mut functions: Vec<FunctionDef> = Vec::new();
    let mut actions: Vec<ActionDef> = Vec::new();
    let mut enum_registry = kata_core::EnumRegistry::new();
    let mut struct_registry = kata_core::StructRegistry::new();
    let mut refined_decls = Vec::new();
    let mut enum_pred_decls = Vec::new();
    let mut interface_registry = kata_core::InterfaceRegistry::new();

    // Pass 0: popula TypeEnv com tipos declarados
    pass0::run_pass0(
        &module.items,
        &mut type_env,
        &mut enum_registry,
        &mut struct_registry,
        &mut refined_decls,
        &mut enum_pred_decls,
        &mut interface_registry,
        &mut signatures,
        &mut functions,
    );

    // Pass 1: coleta assinaturas de funções
    for item in &module.items {
        match &item.node {
            Item::Sig {
                name,
                params,
                ret,
                directives,
                body,
            } => {
                // Converte TypeExpr → Ty
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env, &interface_registry);

                // Extrai metadados de diretivas
                let mut ffi_symbol = None;
                let mut is_associative = false;
                let mut associative_neutral = None;
                let mut is_commutative = false;

                for d in directives {
                    match d.name.as_str() {
                        "ffi" => {
                            if let Some(kata_ast::DirectiveArg::Str(s)) = d.args.first() {
                                ffi_symbol = Some(s.clone());
                            }
                        }
                        "associative" => {
                            is_associative = true;
                            if let Some(kata_ast::DirectiveArg::Int(n)) = d.args.first() {
                                associative_neutral = Some(*n);
                            }
                        }
                        "commutative" => {
                            is_commutative = true;
                        }
                        _ => {}
                    }
                }

                // Fase 5: coleta type params (Ty::Var UPPER_CASE em params/ret).
                let type_params = collect_type_params(&param_types, &return_type);

                // Se tem corpo Kata (cláusulas lambda), preserva para o inference.
                if let Some(clauses) = body {
                    functions.push(FunctionDef {
                        name: name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        clauses: clauses.clone(),
                    });
                }

                signatures.push(Signature {
                    name: name.clone(),
                    param_types,
                    return_type,
                    ffi_symbol,
                    is_associative,
                    associative_neutral,
                    is_action: false,
                    is_commutative,
                    type_params,
                });
            }
            Item::ActionDecl {
                name,
                params,
                ret,
                directives: action_dirs,
                body,
            } => {
                // Converte TypeExpr → Ty para os parâmetros e retorno.
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env, &interface_registry);

                // Extrai ffi_symbol das diretivas da Action.
                let ffi_symbol = action_dirs.iter().find_map(|d| {
                    if d.name == "ffi"
                        && let Some(kata_ast::DirectiveArg::Str(s)) = d.args.first()
                    {
                        return Some(s.clone());
                    }
                    None
                });

                // Se tem @ffi e body vazio → Action FFI builtin.
                // Produz uma Signature com is_action = true para o DispatchTable.
                // Não produz ActionDef (sem corpo Kata para o inference processar).
                if ffi_symbol.is_some() && body.is_empty() {
                    signatures.push(Signature {
                        name: name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        ffi_symbol,
                        is_associative: false,
                        associative_neutral: None,
                        is_action: true,
                        is_commutative: false,
                        type_params: vec![],
                    });
                } else {
                    // Action com corpo Kata — produz ActionDef para o inference.
                    actions.push(ActionDef {
                        name: name.clone(),
                        param_types,
                        return_type,
                        body: body.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    let errors: Vec<ResolveError> = Vec::new();
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        functions,
        actions,
    })
}

pub use prelude_sigs::load_prelude;
