//! Pass 0 — popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum).
//!
//! Percorre os items do módulo e registra:
//! - `data` → Ty::Struct ou Ty::Prim (via @ffi), structs com campos no StructRegistry
//! - `alias` → Ty::Struct (newtype), registrado com alias_of
//! - `enum` → Ty::Sum, variantes no EnumRegistry
//! - `interface` → InterfaceRegistry
//! - `implements` → InterfaceRegistry + signatures flat para o DispatchTable

use kata_ast::{Item, TypeExpr};
use kata_core::{
    EnumRegistry, FieldInfo, ImplEntry, ImplMethodInfo, InterfaceInfo, InterfaceRegistry,
    InterfaceSignature, PrimTy, RefinesEntry, RefinesRegistry, StructKey, StructRegistry, Ty,
    TypeEnv,
};

use crate::type_resolve::{
    collect_type_params, infer_payload_ty_from_literal, infer_payload_ty_from_pred,
    is_type_param_name, resolve_type_expr,
};
use crate::types::{
    EnumPredDeclInfo, EnumPredVariant, FunctionDef, RefinedDeclInfo, ResolveError, Signature,
};

/// Pass 0: popula TypeEnv + registries com tipos declarados no módulo.
///
/// Recebe mut refs para os acumuladores que `resolve()` criou e preenche
/// com base nos items do módulo.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_pass0(
    items: &[kata_ast::Spanned<Item>],
    type_env: &mut TypeEnv,
    enum_registry: &mut EnumRegistry,
    struct_registry: &mut StructRegistry,
    refined_decls: &mut Vec<RefinedDeclInfo>,
    enum_pred_decls: &mut Vec<EnumPredDeclInfo>,
    interface_registry: &mut InterfaceRegistry,
    refines_registry: &mut RefinesRegistry,
    signatures: &mut Vec<Signature>,
    functions: &mut Vec<FunctionDef>,
    errors: &mut Vec<ResolveError>,
    origin: &str,
) {
    for item in items {
        match &item.node {
            Item::DataDecl {
                name,
                fields,
                directives: data_dirs,
                refined,
                ..
            } => {
                // Valida diretivas: só @ffi é válida em data. Outras → erro.
                for d in data_dirs {
                    match d.name.as_str() {
                        "ffi" => {}
                        other => {
                            errors.push(ResolveError::UnknownDirective {
                                name: other.to_string(),
                                context: "data",
                                item_name: name.clone(),
                            });
                        }
                    }
                }
                // Refined declaration?
                if let Some(refined_decl) = refined {
                    // `data (Int, > _ 0) as PositiveInt` — refined concreto
                    // `data (NUM, != _ (zero _)) as NonZero` — refined polimórfico
                    //
                    // Registra no StructRegistry e guarda para o inference
                    // sintetizar as funções predicado.
                    let base_ty =
                        resolve_type_expr(&refined_decl.base_ty.node, type_env, interface_registry);

                    match &base_ty {
                        Ty::Interface(iface_name) => {
                            // Refined polimórfico: expandir em instâncias por tipo concreto.
                            let implementors = interface_registry.implementors_of(iface_name);
                            if implementors.is_empty() {
                                // Ninguém implementa a interface — registrar como
                                // refined concreto com alias_of = interface (fallback).
                                let pred_names: Vec<String> = (0..refined_decl.predicates.len())
                                    .map(|i| format!("__pred_{name}_{i}"))
                                    .collect();
                                struct_registry
                                    .register_refined(origin, name, iface_name, pred_names);
                                type_env.define(
                                    name,
                                    Ty::Struct(StructKey::Plain(name.clone())),
                                    origin,
                                );
                                refined_decls.push(RefinedDeclInfo {
                                    name: name.clone(),
                                    base_ty,
                                    predicates: refined_decl.predicates.clone(),
                                });
                            } else {
                                // Registrar uma instância por tipo concreto.
                                for concrete in &implementors {
                                    let pred_names: Vec<String> =
                                        (0..refined_decl.predicates.len())
                                            .map(|i| format!("__pred_{name}_{concrete}_{i}"))
                                            .collect();
                                    struct_registry.register_refined_instance(
                                        origin, name, concrete, pred_names,
                                    );
                                    // RefinedDeclInfo por instância para o inference
                                    // sintetizar o construtor.
                                    let instance_base = match concrete.as_str() {
                                        "Int" => Ty::Prim(PrimTy::Int),
                                        "Float" => Ty::Prim(PrimTy::Float),
                                        "Rational" => Ty::Prim(PrimTy::Rational),
                                        "Text" => Ty::Prim(PrimTy::Text),
                                        _ => Ty::Struct(StructKey::Plain(concrete.clone())),
                                    };
                                    refined_decls.push(RefinedDeclInfo {
                                        name: name.clone(),
                                        base_ty: instance_base,
                                        predicates: refined_decl.predicates.clone(),
                                    });
                                }
                                // Registrar o nome público no type_env.
                                type_env.define(
                                    name,
                                    Ty::Struct(StructKey::Plain(name.clone())),
                                    origin,
                                );
                            }
                        }
                        _ => {
                            // Refined concreto: `data (Int, > _ 0) as PositiveInt`
                            let base_ty_name = match &refined_decl.base_ty.node {
                                TypeExpr::Named(n) => n.clone(),
                                _ => String::new(),
                            };
                            let pred_names: Vec<String> = (0..refined_decl.predicates.len())
                                .map(|i| format!("__pred_{name}_{i}"))
                                .collect();
                            struct_registry.register_refined(
                                origin,
                                name,
                                &base_ty_name,
                                pred_names,
                            );
                            type_env.define(
                                name,
                                Ty::Struct(StructKey::Plain(name.clone())),
                                origin,
                            );
                            refined_decls.push(RefinedDeclInfo {
                                name: name.clone(),
                                base_ty,
                                predicates: refined_decl.predicates.clone(),
                            });
                        }
                    }
                    continue;
                }

                // data Int () @ffi("i64") → Ty::Prim(PrimTy::Int)
                // Mapeia FFI symbols conhecidos para PrimTy. Se não tem @ffi
                // ou o símbolo não é reconhecido, registra como Ty::Struct.
                let ffi_symbol = data_dirs.iter().find_map(|d| {
                    if d.name == "ffi"
                        && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                        && let kata_ast::Expr::TextLit { text } = &e.node
                    {
                        return Some(text.clone());
                    }
                    None
                });
                let ty = match ffi_symbol.as_deref() {
                    Some("i64") => Ty::Prim(PrimTy::Int),
                    Some("f64") => Ty::Prim(PrimTy::Float),
                    Some("kata_rt_string") => Ty::Prim(PrimTy::Text),
                    Some("kata_rt_rat") => Ty::Prim(PrimTy::Rational),
                    _ => Ty::Struct(StructKey::Plain(name.clone())),
                };
                type_env.define(name, ty, origin);

                // Se o DataDecl tem campos não-vazios, registra no StructRegistry.
                // Offset de cada campo = field_index * 8 (todos os campos são words de 8 bytes).
                if !fields.is_empty() {
                    let field_infos: Vec<FieldInfo> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| FieldInfo {
                            name: f.name.clone(),
                            ty: resolve_type_expr(&f.ty.node, type_env, interface_registry),
                            offset: (i as u32) * 8,
                        })
                        .collect();
                    struct_registry.register(origin, name, field_infos);
                }
            }
            Item::AliasDecl { target, new_name } => {
                // alias Target as NewName — cria tipo nominal distinto.
                // O alias é Ty::Struct(new_name) independentemente do target.
                type_env.define(
                    new_name,
                    Ty::Struct(StructKey::Plain(new_name.clone())),
                    origin,
                );

                // Se o target é refined (tem predicates no StructRegistry),
                // o alias herda os predicados e torna-se refined também.
                // alias_of aponta para o target imediato (não para a base),
                // para preservar a cadeia Peso → PositiveFloat → Float.
                let target_is_refined = struct_registry
                    .get(target)
                    .map(|info| info.predicates.is_some())
                    .unwrap_or(false);

                if target_is_refined {
                    let predicates = struct_registry
                        .get(target)
                        .and_then(|info| info.predicates.clone())
                        .unwrap_or_default();
                    struct_registry.register_refined(origin, new_name, target, predicates);

                    // Copia RefinedDeclInfo do target para o alias,
                    // para que o inference sintetize o construtor falível.
                    if let Some(rd) = refined_decls.iter().find(|rd| rd.name == *target) {
                        refined_decls.push(RefinedDeclInfo {
                            name: new_name.clone(),
                            base_ty: rd.base_ty.clone(),
                            predicates: rd.predicates.clone(),
                        });
                    }
                } else {
                    // Alias normal (target não-refined): herda campos se houver.
                    let fields = if let Some(target_info) = struct_registry.get(target) {
                        target_info.fields.clone()
                    } else {
                        Vec::new()
                    };
                    struct_registry.register_with_alias(
                        origin,
                        new_name,
                        fields,
                        Some(target.clone()),
                    );
                }
            }
            Item::EnumDecl { name, variants, .. } => {
                type_env.define(name, Ty::Sum(name.clone()), origin);

                // Cataloga variantes no EnumRegistry.
                // Resolve payload types das variantes.
                // Processa predicados das variantes.
                let has_predicates = variants.iter().any(|v| v.predicate.is_some());

                let variant_infos: Vec<kata_core::VariantInfo> = {
                    // Se tem predicados, infere payload_ty base a partir das
                    // variantes predicadas. A variante default herda esse tipo.
                    let base_payload_ty = if has_predicates {
                        variants.iter().find_map(|v| {
                            v.payload
                                .as_ref()
                                .map(|p| resolve_type_expr(&p.node, type_env, interface_registry))
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
                                .map(|p| resolve_type_expr(&p.node, type_env, interface_registry))
                                .or_else(|| {
                                    v.predicate
                                        .as_ref()
                                        .and_then(|pred| infer_payload_ty_from_pred(&pred.node))
                                })
                                // Variante default herda payload_ty das variantes predicadas
                                // (apenas quando o enum tem predicados).
                                .or_else(|| base_payload_ty.clone())
                                // Valor fixo: infere payload_ty do tipo do literal.
                                .or_else(|| {
                                    v.fixed_value
                                        .as_ref()
                                        .and_then(|fv| infer_payload_ty_from_literal(&fv.node))
                                });
                            let predicate = v
                                .predicate
                                .as_ref()
                                .map(|_| format!("__pred_enum_{name}_{}", v.name));
                            kata_core::VariantInfo {
                                name: v.name.clone(),
                                payload_ty,
                                predicate,
                                fixed_value: v.fixed_value.as_ref().map(|fv| match &fv.node {
                                    kata_ast::Expr::IntLit { text } => text.clone(),
                                    kata_ast::Expr::FloatLit { text } => text.clone(),
                                    kata_ast::Expr::TextLit { text } => text.clone(),
                                    _ => String::new(),
                                }),
                            }
                        })
                        .collect()
                };
                enum_registry.register(origin, name, variant_infos.clone());

                // Se variantes têm payloads Ty::Var (type params),
                // registrar como enum genérico. Coleta type params dos payloads.
                // Também coleta defaults: se um variant tem `default` (ex: `Err(E|Text)`),
                // o type param do payload tem aquele default.
                let mut type_params: Vec<String> = Vec::new();
                let mut defaults: Vec<Option<Ty>> = Vec::new();
                for v in variants.iter() {
                    if let Some(payload) = &v.payload {
                        let payload_ty =
                            resolve_type_expr(&payload.node, type_env, interface_registry);
                        if let Ty::Var(n) = &payload_ty
                            && is_type_param_name(n)
                        {
                            // type param name ainda não registrado
                            if !type_params.contains(n) {
                                type_params.push(n.clone());
                                // Se o variant tem default, resolve e registra.
                                let default_ty = v.default.as_ref().map(|d| {
                                    resolve_type_expr(&d.node, type_env, interface_registry)
                                });
                                defaults.push(default_ty);
                            }
                        }
                    }
                }
                if !type_params.is_empty() {
                    enum_registry.register_generic_with_defaults(
                        origin,
                        name,
                        type_params,
                        defaults,
                        variant_infos,
                    );
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
                                .map(|p| resolve_type_expr(&p.node, type_env, interface_registry))
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
            // InterfaceDecl — registra no InterfaceRegistry.
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
                            .map(|t| resolve_type_expr(&t.node, type_env, interface_registry))
                            .collect(),
                        ret: resolve_type_expr(&s.ret.node, type_env, interface_registry),
                        default_body: s.default_body.clone(),
                    })
                    .collect();
                let info = InterfaceInfo {
                    name: name.clone(),
                    supertraits: supertraits.clone(),
                    type_params: type_params.clone(),
                    signatures: iface_sigs,
                };
                if let Err(e) = interface_registry.register_interface(origin, info) {
                    eprintln!("[resolution] warning: {e}");
                }
            }
            // ImplementsDecl — registra no InterfaceRegistry.
            // O registro no DispatchTable será feito quando o prelude migrar
            // para Kata. Por ora, o InterfaceRegistry cataloga a impl.
            Item::ImplementsDecl {
                type_name,
                type_params,
                interface_name,
                iface_params,
                methods,
            } => {
                // Valida diretivas de cada método: só @ffi, @builtin, @commutative,
                // @associative são válidas em implements. Outras → erro.
                for m in methods {
                    for d in &m.directives {
                        match d.name.as_str() {
                            "ffi" | "builtin" | "commutative" | "associative" => {}
                            other => {
                                errors.push(ResolveError::UnknownDirective {
                                    name: other.to_string(),
                                    context: "implements method",
                                    item_name: m.name.clone(),
                                });
                            }
                        }
                    }
                }
                let impl_methods: Vec<ImplMethodInfo> = methods
                    .iter()
                    .map(|m| {
                        // Extrai @ffi OU @builtin como símbolo.
                        // @ffi("kata_rt_array_next") → Some("kata_rt_array_next")
                        // @builtin("range_next") → Some("range_next")
                        let ffi_symbol = m.directives.iter().find_map(|d| {
                            if (d.name == "ffi" || d.name == "builtin")
                                && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                                && let kata_ast::Expr::TextLit { text } = &e.node
                            {
                                return Some(text.clone());
                            }
                            None
                        });
                        ImplMethodInfo {
                            name: m.name.clone(),
                            params: m
                                .params
                                .iter()
                                .map(|t| resolve_type_expr(&t.node, type_env, interface_registry))
                                .collect(),
                            ret: resolve_type_expr(&m.ret.node, type_env, interface_registry),
                            ffi_symbol,
                        }
                    })
                    .collect();
                let entry = ImplEntry {
                    origin: origin.to_string(),
                    type_name: type_name.clone(),
                    type_params: type_params.clone(),
                    interface_name: interface_name.clone(),
                    iface_params: iface_params.clone(),
                    methods: impl_methods,
                };
                if let Err(e) = interface_registry.register_impl(entry) {
                    eprintln!("[resolution] warning: {e}");
                }

                // Cada método de implements vira uma Signature flat
                // (como se fosse uma Sig standalone). O dispatch usa estas
                // signatures para popular o DispatchTable.
                for m in methods {
                    let param_types: Vec<Ty> = m
                        .params
                        .iter()
                        .map(|t| resolve_type_expr(&t.node, type_env, interface_registry))
                        .collect();
                    let return_type = resolve_type_expr(&m.ret.node, type_env, interface_registry);
                    let ffi_symbol = m.directives.iter().find_map(|d| {
                        if (d.name == "ffi" || d.name == "builtin")
                            && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                            && let kata_ast::Expr::TextLit { text } = &e.node
                        {
                            return Some(text.clone());
                        }
                        None
                    });
                    let is_commutative = m.directives.iter().any(|d| d.name == "commutative");
                    let is_associative = m.directives.iter().any(|d| d.name == "associative");
                    let associative_neutral = m.directives.iter().find_map(|d| {
                        if d.name == "associative"
                            && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                            && let kata_ast::Expr::IntLit { text } = &e.node
                            && let Ok(n) = text.parse::<i64>()
                        {
                            return Some(n);
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

                    // Método com corpo Kata (lambda) precisa de
                    // FunctionDef para o inference produzir TypedFunction.
                    // Sem isso, o corpo é invisível para o inference/codegen.
                    if let Some(clauses) = &m.body {
                        functions.push(FunctionDef {
                            name: m.name.clone(),
                            param_types,
                            return_type,
                            clauses: clauses.clone(),
                            cache_strategy: None,
                            timer: None,
                            custom_directives: Vec::new(),
                        });
                    }
                }

                // ── Default methods: métodos da interface com default_body
                // que não foram definidos no impl. Gera Signature +
                // FunctionDef sintetizada usando o default_body da interface.
                // Self na assinatura é substituído pelo tipo concreto.
                if let Some(iface_info) = interface_registry.get_interface(interface_name) {
                    let defined_names: std::collections::HashSet<&str> =
                        methods.iter().map(|m| m.name.as_str()).collect();
                    for sig in &iface_info.signatures {
                        if defined_names.contains(sig.name.as_str()) {
                            continue; // método definido no impl — não usa default
                        }
                        if let Some(default_clauses) = &sig.default_body {
                            // Substituir Self pelo tipo concreto nos param/ret.
                            // type_name é o nome do tipo (ex: "Int", "Float").
                            // resolve_type_expr mapeia "Int" → Ty::Prim(PrimTy::Int), etc.
                            let concrete_ty = resolve_type_expr(
                                &kata_ast::TypeExpr::Named(type_name.clone()),
                                type_env,
                                interface_registry,
                            );
                            let param_types: Vec<Ty> = sig
                                .params
                                .iter()
                                .map(|t| t.substitute_self(&concrete_ty))
                                .collect();
                            let return_type = sig.ret.substitute_self(&concrete_ty);
                            let type_params = collect_type_params(&param_types, &return_type);

                            signatures.push(Signature {
                                name: sig.name.clone(),
                                param_types: param_types.clone(),
                                return_type: return_type.clone(),
                                ffi_symbol: None,
                                is_associative: false,
                                associative_neutral: None,
                                is_action: false,
                                is_commutative: false,
                                type_params,
                            });

                            // O default_body tem Self no tipo — precisa ser
                            // substituído pelo tipo concreto. O typeck vai
                            // tipar o corpo com os tipos concretos.
                            functions.push(FunctionDef {
                                name: sig.name.clone(),
                                param_types,
                                return_type,
                                clauses: default_clauses.clone(),
                                cache_strategy: None,
                                timer: None,
                                custom_directives: Vec::new(),
                            });
                        }
                    }
                }
            }
            // RefinesDecl — registra no RefinesRegistry.
            // Não registra no InterfaceRegistry nem cria overloads no DispatchTable.
            // O fallback no dispatch (apply.rs) usa este registry para substituir
            // args refined pelo tipo base e retentar.
            //
            // Validações:
            // - type_name deve ser refined (StructInfo com alias_of e predicates)
            // - base deve implementar a interface no InterfaceRegistry
            // - métodos com corpo (override) são processados como ImplementsDecl
            //   (criam overload real no DispatchTable)
            Item::RefinesDecl {
                type_name,
                interface_name,
                methods,
            } => {
                // Validar que type_name é refined.
                // Para refined polimórfico, `get` retorna None (só há
                // instâncias Instance, não Plain). Verificar também
                // se há instâncias de família com is_instance_of.
                let struct_info = struct_registry.get(type_name);
                let is_refined = struct_info
                    .map(|si| si.alias_of.is_some() && si.predicates.is_some())
                    .unwrap_or(false);
                // Para famílias polimórficas, get_instance com qualquer
                // tipo concreto conhecido deve retornar Some.
                let is_family = struct_registry
                    .get_instance(type_name, "Int")
                    .or_else(|| struct_registry.get_instance(type_name, "Float"))
                    .is_some();
                if !is_refined && !is_family {
                    errors.push(ResolveError::InvalidRefines {
                        type_name: type_name.clone(),
                        reason:
                            "refines só se aplica a tipos refined (data (Base, predicados) as Nome)"
                                .into(),
                    });
                    // Continua para processar overrides mesmo assim — o erro
                    // já foi reportado.
                }

                // Resolver tipo base via alias_of no StructRegistry.
                // Para famílias polimórficas, struct_info é None — usar
                // uma instância para obter alias_of (todas compartilham
                // o mesmo base_ty conceitual: a interface).
                let base_ty_name = if let Some(si) = struct_info {
                    si.alias_of.as_deref().unwrap_or("").to_string()
                } else if is_family {
                    // Para famílias, o base_ty é a interface (ex: "NUM").
                    // Usar alias_of da primeira instância encontrada.
                    struct_registry
                        .get_instance(type_name, "Int")
                        .or_else(|| struct_registry.get_instance(type_name, "Float"))
                        .and_then(|si| si.alias_of.as_deref().map(String::from))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let base_ty = resolve_base_ty(&base_ty_name, type_env, interface_registry);

                // Validar que o base implementa a interface.
                // Aviso apenas — a validação final acontece em infer_module,
                // depois do merge com o prelude (que contém `Int implements NUM`).
                if !base_ty_name.is_empty()
                    && !interface_registry.type_implements(&base_ty_name, interface_name)
                {
                    eprintln!(
                        "[resolution] warning: tipo base {base_ty_name} pode não implementar \
                         a interface {interface_name} (PositiveInt refines {interface_name}) — \
                         validação final em infer_module"
                    );
                }

                // Registrar delegação no RefinesRegistry.
                refines_registry.register(RefinesEntry {
                    origin: origin.to_string(),
                    type_name: type_name.clone(),
                    base_ty,
                    interface_name: interface_name.clone(),
                });

                // Métodos com corpo (override) são processados como overloads
                // reais no DispatchTable — mesmas regras de ImplementsDecl.
                for m in methods {
                    let param_types: Vec<Ty> = m
                        .params
                        .iter()
                        .map(|t| resolve_type_expr(&t.node, type_env, interface_registry))
                        .collect();
                    let return_type = resolve_type_expr(&m.ret.node, type_env, interface_registry);
                    let ffi_symbol = m.directives.iter().find_map(|d| {
                        if (d.name == "ffi" || d.name == "builtin")
                            && let Some(kata_ast::DirectiveArg::Expr(e)) = d.args.first()
                            && let kata_ast::Expr::TextLit { text } = &e.node
                        {
                            return Some(text.clone());
                        }
                        None
                    });
                    let type_params = collect_type_params(&param_types, &return_type);

                    signatures.push(Signature {
                        name: m.name.clone(),
                        param_types: param_types.clone(),
                        return_type: return_type.clone(),
                        ffi_symbol: ffi_symbol.clone(),
                        is_associative: false,
                        associative_neutral: None,
                        is_action: false,
                        is_commutative: false,
                        type_params,
                    });

                    // Método com corpo Kata (lambda) precisa de FunctionDef.
                    if let Some(clauses) = &m.body {
                        functions.push(FunctionDef {
                            name: m.name.clone(),
                            param_types,
                            return_type,
                            clauses: clauses.clone(),
                            cache_strategy: None,
                            timer: None,
                            custom_directives: Vec::new(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Resolve um nome de tipo base (ex: "Int") para `Ty`.
/// Usado pelo processamento de RefinesDecl para obter o tipo base do refined.
fn resolve_base_ty(base_name: &str, type_env: &TypeEnv, iface_reg: &InterfaceRegistry) -> Ty {
    if let Some(ty) = type_env.lookup(base_name) {
        return ty.clone();
    }
    // Fallback: nomes conhecidos do prelude.
    match base_name {
        "Int" => Ty::Prim(PrimTy::Int),
        "Float" => Ty::Prim(PrimTy::Float),
        "Text" => Ty::Prim(PrimTy::Text),
        "Rational" => Ty::Prim(PrimTy::Rational),
        "Boolean" => Ty::Sum("Boolean".into()),
        "Unit" => Ty::Unit,
        _ => {
            if iface_reg.get_interface(base_name).is_some() {
                Ty::Interface(base_name.into())
            } else {
                Ty::Struct(StructKey::Plain(base_name.into()))
            }
        }
    }
}
