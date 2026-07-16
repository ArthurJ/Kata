//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

pub(crate) mod module_loader;
mod prelude_sigs;
mod type_resolve;
mod types;

pub use types::*;

use kata_ast::{Item, Module, TypeExpr};
use kata_core::{
    EnumRegistry, FieldInfo, ImplEntry, ImplMethodInfo, InterfaceInfo, InterfaceRegistry,
    InterfaceSignature, PrimTy, StructRegistry, Ty, TypeEnv, VariantInfo,
};
use type_resolve::{
    collect_type_params, infer_payload_ty_from_pred, is_type_param_name, resolve_type_expr,
};

/// Resolve um módulo: Pass 0 + Pass 1.
pub fn resolve(module: &Module) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut type_env = TypeEnv::new();
    // Unit é tipo primitivo da linguagem — sempre disponível no TypeEnv.
    type_env.define("Unit", Ty::Unit);
    let mut signatures: Vec<Signature> = Vec::new();
    let mut functions: Vec<FunctionDef> = Vec::new();
    let mut actions: Vec<ActionDef> = Vec::new();
    let mut enum_registry = EnumRegistry::new();
    let mut struct_registry = StructRegistry::new();
    let mut refined_decls: Vec<RefinedDeclInfo> = Vec::new();
    let mut enum_pred_decls: Vec<EnumPredDeclInfo> = Vec::new();
    let mut interface_registry = InterfaceRegistry::new();
    let errors: Vec<ResolveError> = Vec::new();

    // Pass 0: popula TypeEnv com tipos declarados
    for item in &module.items {
        match &item.node {
            Item::DataDecl {
                name,
                fields,
                directives: data_dirs,
                refined,
                ..
            } => {
                // Fio 6: refined declaration?
                if let Some(refined_decl) = refined {
                    // `data (Int, > _ 0) as PositiveInt`
                    // Registra no StructRegistry como refined:
                    //   alias_of = base_ty_name, predicates = [nomes das funções]
                    // As funções predicado são sintetizadas no inference.
                    let base_ty_name = match &refined_decl.base_ty.node {
                        TypeExpr::Named(n) => n.clone(),
                        _ => {
                            // TODO: base non-named (Tuple, etc.) — fora do escopo
                            String::new()
                        }
                    };
                    // Gera nomes das funções predicado: __pred_<TypeName>_<idx>
                    let pred_names: Vec<String> = (0..refined_decl.predicates.len())
                        .map(|i| format!("__pred_{name}_{i}"))
                        .collect();
                    struct_registry.register_refined(name, &base_ty_name, pred_names);
                    type_env.define(name, Ty::Struct(name.clone()));

                    // Guarda para o inference sintetizar as funções predicado.
                    let base_ty = resolve_type_expr(
                        &refined_decl.base_ty.node,
                        &type_env,
                        &interface_registry,
                    );
                    refined_decls.push(RefinedDeclInfo {
                        name: name.clone(),
                        base_ty,
                        predicates: refined_decl.predicates.clone(),
                    });
                    continue;
                }

                // Fase 8: data Int () @ffi("i64") → Ty::Prim(PrimTy::Int)
                // Mapeia FFI symbols conhecidos para PrimTy. Se não tem @ffi
                // ou o símbolo não é reconhecido, registra como Ty::Struct.
                let ffi_symbol = data_dirs.iter().find_map(|d| {
                    if d.name == "ffi"
                        && let Some(kata_ast::DirectiveArg::Str(s)) = d.args.first()
                    {
                        return Some(s.clone());
                    }
                    None
                });
                let ty = match ffi_symbol.as_deref() {
                    Some("i64") => Ty::Prim(PrimTy::Int),
                    Some("f64") => Ty::Prim(PrimTy::Float),
                    Some("kata_rt_string") => Ty::Prim(PrimTy::Text),
                    Some("kata_rt_rat") => Ty::Prim(PrimTy::Rational),
                    _ => Ty::Struct(name.clone()),
                };
                type_env.define(name, ty);

                // Fio 5: se o DataDecl tem campos não-vazios, registra no StructRegistry.
                // Offset de cada campo = field_index * 8 (todos os campos são words de 8 bytes).
                if !fields.is_empty() {
                    let field_infos: Vec<FieldInfo> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| FieldInfo {
                            name: f.name.clone(),
                            ty: resolve_type_expr(&f.ty.node, &type_env, &interface_registry),
                            offset: (i as u32) * 8,
                        })
                        .collect();
                    struct_registry.register(name, field_infos);
                }
            }
            Item::AliasDecl { target, new_name } => {
                // alias Target as NewName — cria tipo nominal distinto.
                // O alias é Ty::Struct(new_name) independentemente do target.
                type_env.define(new_name, Ty::Struct(new_name.clone()));

                // Registra no StructRegistry com alias_of = Some(target).
                // Se o target é um struct com campos, herda os campos (para field access).
                // Se o target é primitivo/opaco, campos = vazio.
                let fields = if let Some(target_info) = struct_registry.get(target) {
                    target_info.fields.clone()
                } else {
                    Vec::new()
                };
                struct_registry.register_with_alias(new_name, fields, Some(target.clone()));
            }
            Item::EnumDecl { name, variants, .. } => {
                type_env.define(name, Ty::Sum(name.clone()));
                // Fio 2: cataloga variantes no EnumRegistry.
                // Fase 5: resolve payload types das variantes.
                // Fio 6: processa predicados das variantes.
                let has_predicates = variants.iter().any(|v| v.predicate.is_some());

                let variant_infos: Vec<VariantInfo> = {
                    // Se tem predicados, infere payload_ty base a partir das
                    // variantes predicadas. A variante default herda esse tipo.
                    let base_payload_ty = if has_predicates {
                        variants.iter().find_map(|v| {
                            v.payload
                                .as_ref()
                                .map(|p| resolve_type_expr(&p.node, &type_env, &interface_registry))
                                .or_else(|| {
                                    v.predicate
                                        .as_ref()
                                        .and_then(|pred| infer_payload_ty_from_pred(&pred.node))
                                })
                        })
                    } else {
                        None
                    };

                    variants
                        .iter()
                        .map(|v| {
                            let payload_ty = v
                                .payload
                                .as_ref()
                                .map(|p| resolve_type_expr(&p.node, &type_env, &interface_registry))
                                .or_else(|| {
                                    v.predicate
                                        .as_ref()
                                        .and_then(|pred| infer_payload_ty_from_pred(&pred.node))
                                })
                                // Variante default herda payload_ty das variantes predicadas
                                // (apenas quando o enum tem predicados).
                                .or_else(|| base_payload_ty.clone());
                            let predicate = v
                                .predicate
                                .as_ref()
                                .map(|_| format!("__pred_enum_{name}_{}", v.name));
                            VariantInfo {
                                name: v.name.clone(),
                                payload_ty,
                                predicate,
                            }
                        })
                        .collect()
                };
                enum_registry.register(name, variant_infos.clone());

                // Fase 8: se variantes têm payloads Ty::Var (type params),
                // registrar como enum genérico. Coleta type params dos payloads.
                let type_params: Vec<String> = variant_infos
                    .iter()
                    .filter_map(|v| {
                        if let Some(Ty::Var(n)) = &v.payload_ty {
                            if is_type_param_name(n) {
                                Some(n.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                if !type_params.is_empty() {
                    enum_registry.register_generic(name, type_params, variant_infos);
                }

                // Se tem variantes predicadas, guarda para o inference sintetizar
                // o construtor despachador.
                if has_predicates {
                    // O tipo do payload é inferido a partir do predicado:
                    // `Magreza(< _ 18.5)` → literal 18.5 é Float → payload é Float.
                    // Se uma variante tem payload explícito, usa esse.
                    // Senão, infere do literal no predicado.
                    let payload_ty = variants
                        .iter()
                        .find_map(|v| {
                            v.payload
                                .as_ref()
                                .map(|p| resolve_type_expr(&p.node, &type_env, &interface_registry))
                        })
                        .unwrap_or_else(|| {
                            // Infere do predicado: aplicação `op _ literal` → tipo do literal.
                            variants
                                .iter()
                                .find_map(|v| {
                                    v.predicate
                                        .as_ref()
                                        .and_then(|pred| infer_payload_ty_from_pred(&pred.node))
                                })
                                .unwrap_or(Ty::Unit)
                        });

                    let enum_variants: Vec<EnumPredVariant> = variants
                        .iter()
                        .enumerate()
                        .map(|(i, v)| EnumPredVariant {
                            name: v.name.clone(),
                            predicate: v.predicate.clone(),
                            tag: i,
                        })
                        .collect();
                    enum_pred_decls.push(EnumPredDeclInfo {
                        name: name.clone(),
                        payload_ty,
                        variants: enum_variants,
                    });
                }
            }
            // Fio 7: InterfaceDecl — registra no InterfaceRegistry.
            Item::InterfaceDecl {
                name,
                supertraits,
                type_params,
                signatures,
            } => {
                let iface_sigs: Vec<InterfaceSignature> = signatures
                    .iter()
                    .map(|s| InterfaceSignature {
                        name: s.name.clone(),
                        params: s
                            .params
                            .iter()
                            .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                            .collect(),
                        ret: resolve_type_expr(&s.ret.node, &type_env, &interface_registry),
                    })
                    .collect();
                let info = InterfaceInfo {
                    name: name.clone(),
                    supertraits: supertraits.clone(),
                    type_params: type_params.clone(),
                    signatures: iface_sigs,
                };
                if let Err(e) = interface_registry.register_interface(info) {
                    eprintln!("[resolution] warning: {e}");
                }
            }
            // Fio 7: ImplementsDecl — registra no InterfaceRegistry.
            // O registro no DispatchTable será feito quando o prelude migrar
            // para Kata (Fase 8). Por ora, o InterfaceRegistry cataloga a impl.
            Item::ImplementsDecl {
                type_name,
                type_params,
                interface_name,
                iface_params,
                methods,
            } => {
                let impl_methods: Vec<ImplMethodInfo> = methods
                    .iter()
                    .map(|m| {
                        // Fio 8: extrai @ffi OU @builtin como símbolo.
                        // @ffi("kata_rt_array_next") → Some("kata_rt_array_next")
                        // @builtin("range_next") → Some("range_next")
                        let ffi_symbol = m.directives.iter().find_map(|d| {
                            if (d.name == "ffi" || d.name == "builtin")
                                && let Some(kata_ast::DirectiveArg::Str(s)) = d.args.first()
                            {
                                return Some(s.clone());
                            }
                            None
                        });
                        ImplMethodInfo {
                            name: m.name.clone(),
                            params: m
                                .params
                                .iter()
                                .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                                .collect(),
                            ret: resolve_type_expr(&m.ret.node, &type_env, &interface_registry),
                            ffi_symbol,
                        }
                    })
                    .collect();
                let entry = ImplEntry {
                    type_name: type_name.clone(),
                    type_params: type_params.clone(),
                    interface_name: interface_name.clone(),
                    iface_params: iface_params.clone(),
                    methods: impl_methods,
                };
                if let Err(e) = interface_registry.register_impl(entry) {
                    eprintln!("[resolution] warning: {e}");
                }

                // Fase 8: cada método de implements vira uma Signature flat
                // (como se fosse uma Sig standalone). O dispatch usa estas
                // signatures para popular o DispatchTable.
                for m in methods {
                    let param_types: Vec<Ty> = m
                        .params
                        .iter()
                        .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                        .collect();
                    let return_type =
                        resolve_type_expr(&m.ret.node, &type_env, &interface_registry);
                    let ffi_symbol = m.directives.iter().find_map(|d| {
                        if (d.name == "ffi" || d.name == "builtin")
                            && let Some(kata_ast::DirectiveArg::Str(s)) = d.args.first()
                        {
                            return Some(s.clone());
                        }
                        None
                    });
                    let is_commutative = m.directives.iter().any(|d| d.name == "commutative");
                    let is_associative = m.directives.iter().any(|d| d.name == "associative");
                    let associative_neutral = m.directives.iter().find_map(|d| {
                        if d.name == "associative"
                            && let Some(kata_ast::DirectiveArg::Int(n)) = d.args.first()
                        {
                            return Some(*n);
                        }
                        None
                    });
                    let type_params = collect_type_params(&param_types, &return_type);

                    signatures.push(Signature {
                        name: m.name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        ffi_symbol: ffi_symbol.clone(),
                        is_associative,
                        associative_neutral,
                        is_action: false,
                        is_commutative,
                        type_params,
                    });

                    // Fase 9: método com corpo Kata (lambda) precisa de
                    // FunctionDef para o inference produzir TypedFunction.
                    // Sem isso, o corpo é invisível para o inference/codegen.
                    if let Some(clauses) = &m.body {
                        functions.push(FunctionDef {
                            name: m.name.clone(),
                            param_types,
                            return_type,
                            clauses: clauses.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

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
