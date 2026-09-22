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
    TypeEnv, TypeParamDecl,
};

use crate::type_resolve::{
    collect_type_params, infer_payload_ty_from_literal, infer_payload_ty_from_pred,
    is_type_param_name, resolve_type_expr,
};
use crate::types::{
    EnumPredDeclInfo, EnumPredVariant, FunctionDef, RefinedDeclInfo, ResolveError, Signature,
};

/// Extrai o nome do type parameter livre de uma coleção parametrizada.
///
/// `Ty::List(Ty::Var("A"))` → `Some("A")` — família polimórfica lazy.
/// `Ty::List(Ty::Prim(PrimTy::Int))` → `None` — refined concreto.
/// `Ty::Array(Ty::Var("A"))` → `Some("A")`.
/// `Ty::Set(Ty::Var("A"))` → `Some("A")`.
/// Outros tipos → `None`.
fn extract_lazy_type_param(base_ty: &Ty) -> Option<String> {
    match base_ty {
        Ty::List(inner) | Ty::Array(inner) | Ty::Set(inner) | Ty::Range(inner) => {
            if let Ty::Var(name) = inner.as_ref() {
                Some(name.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Substitui `Struct(Plain(name))` por `Struct(Generic(name, type_args))`
/// quando `name` é um struct paramétrico no StructRegistry.
///
/// Em assinaturas de métodos de `implements` para tipos genéricos, o tipo
/// aparece como `Pair` (Plain) porque `resolve_type_expr` não sabe que é
/// genérico. Esta substituição injeta os type params como `Ty::Var`, fazendo
/// `collect_type_params` detectá-los e marcar o método como genérico para
/// monomorfização.
///
/// Recursiva em tipos compostos (Function, Tuple, List, etc.).
pub(crate) fn instantiate_generic_struct_refs(ty: &Ty, struct_reg: &StructRegistry) -> Ty {
    match ty {
        Ty::Struct(StructKey::Plain(name)) => {
            if let Some(info) = struct_reg.get(name) {
                if let Some(type_params) = &info.type_params {
                    // Gerar type args a partir das **ocorrências** de type params
                    // nos fields (não por variável distinta). Ex: Complex com
                    // re::T im::T produz [Var("T"), Var("T")].
                    let type_args: Vec<Ty> =
                        kata_core::struct_registry::type_param_occurrences_in_fields(
                            &info.fields,
                            type_params,
                        )
                        .into_iter()
                        .map(|name| Ty::Var(name))
                        .collect();
                    return Ty::Struct(StructKey::Generic(name.clone(), type_args));
                }
            }
            ty.clone()
        }
        Ty::Struct(StructKey::Generic(name, args)) => Ty::Struct(StructKey::Generic(
            name.clone(),
            args.iter()
                .map(|a| instantiate_generic_struct_refs(a, struct_reg))
                .collect(),
        )),
        Ty::Function(params, ret) => Ty::Function(
            params
                .iter()
                .map(|p| instantiate_generic_struct_refs(p, struct_reg))
                .collect(),
            Box::new(instantiate_generic_struct_refs(ret, struct_reg)),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params
                .iter()
                .map(|p| instantiate_generic_struct_refs(p, struct_reg))
                .collect(),
            Box::new(instantiate_generic_struct_refs(ret, struct_reg)),
        ),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| instantiate_generic_struct_refs(e, struct_reg))
                .collect(),
        ),
        Ty::List(inner) => Ty::List(Box::new(instantiate_generic_struct_refs(inner, struct_reg))),
        Ty::Array(inner) => Ty::Array(Box::new(instantiate_generic_struct_refs(inner, struct_reg))),
        Ty::Set(inner) => Ty::Set(Box::new(instantiate_generic_struct_refs(inner, struct_reg))),
        Ty::Range(inner) => Ty::Range(Box::new(instantiate_generic_struct_refs(inner, struct_reg))),
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(instantiate_generic_struct_refs(k, struct_reg)),
            Box::new(instantiate_generic_struct_refs(v, struct_reg)),
        ),
        Ty::Generic(name, args) => {
            // Se o nome é um struct paramétrico no struct_registry,
            // converter Ty::Generic → Ty::Struct(StructKey::Generic).
            // Isto corrige tipos produzidos por resolve_type_expr antes
            // do struct_registry do módulo importado estar disponível
            // (ex: math.kata referencia Complex::(Float) antes do
            // struct_registry de complex.kata ser merged).
            if let Some(info) = struct_reg.get(name) {
                if info.type_params.is_some() {
                    let converted_args: Vec<Ty> = args
                        .iter()
                        .map(|a| instantiate_generic_struct_refs(a, struct_reg))
                        .collect();
                    return Ty::Struct(StructKey::Generic(name.clone(), converted_args));
                }
            }
            Ty::Generic(
                name.clone(),
                args.iter()
                    .map(|a| instantiate_generic_struct_refs(a, struct_reg))
                    .collect(),
            )
        }
        _ => ty.clone(),
    }
}

/// Substitui `Family(name)` por `Instance(name, concrete_type)` em um `Ty`,
/// recursivamente, quando a instância existe no StructRegistry.
///
/// No contexto de `Int implements NUM`, os params do impl referenciam `NonZero`
/// (família polimórfica). `resolve_type_expr` produz `Family("NonZero")`.
/// Mas o implements é específico de Int — `NonZero` aqui significa
/// `Instance("NonZero", "Int")`, não a família abstrata. Sem esta substituição,
/// `expand_family_signatures` expande cegamente para TODAS as instâncias
/// (Int, Float, Rational), criando overloads espúrias que causam
/// `AmbiguousDispatch` no call-site.
pub(crate) fn instantiate_family_for_concrete(
    ty: &Ty,
    concrete_type: &str,
    struct_reg: &StructRegistry,
) -> Ty {
    match ty {
        Ty::Struct(StructKey::Family(name)) => {
            if struct_reg.get_instance(name, concrete_type).is_some() {
                Ty::Struct(StructKey::Instance(name.clone(), concrete_type.to_string()))
            } else {
                ty.clone()
            }
        }
        // Plain pode ser uma família polimórfica que ainda não foi
        // registrada quando a interface foi parseada (ex: NonZero é
        // declarado depois de interface NUM). Se o nome é família
        // registrada agora, instanciar para o tipo concreto.
        Ty::Struct(StructKey::Plain(name)) => {
            if struct_reg.is_family(name) && struct_reg.get_instance(name, concrete_type).is_some()
            {
                Ty::Struct(StructKey::Instance(name.clone(), concrete_type.to_string()))
            } else {
                ty.clone()
            }
        }
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter()
                .map(|a| instantiate_family_for_concrete(a, concrete_type, struct_reg))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params
                .iter()
                .map(|p| instantiate_family_for_concrete(p, concrete_type, struct_reg))
                .collect(),
            Box::new(instantiate_family_for_concrete(
                ret,
                concrete_type,
                struct_reg,
            )),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params
                .iter()
                .map(|p| instantiate_family_for_concrete(p, concrete_type, struct_reg))
                .collect(),
            Box::new(instantiate_family_for_concrete(
                ret,
                concrete_type,
                struct_reg,
            )),
        ),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| instantiate_family_for_concrete(e, concrete_type, struct_reg))
                .collect(),
        ),
        Ty::List(elem) => Ty::List(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::Array(elem) => Ty::Array(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::Range(elem) => Ty::Range(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(instantiate_family_for_concrete(
                k,
                concrete_type,
                struct_reg,
            )),
            Box::new(instantiate_family_for_concrete(
                v,
                concrete_type,
                struct_reg,
            )),
        ),
        Ty::Set(elem) => Ty::Set(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::Sender(elem) => Ty::Sender(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::Receiver(elem) => Ty::Receiver(Box::new(instantiate_family_for_concrete(
            elem,
            concrete_type,
            struct_reg,
        ))),
        Ty::ReceiverFactory(elem) => Ty::ReceiverFactory(Box::new(
            instantiate_family_for_concrete(elem, concrete_type, struct_reg),
        )),
        _ => ty.clone(),
    }
}

/// Pass 0: popula TypeEnv + registries com tipos declarados no módulo.
///
/// Recebe mut refs para os acumuladores que `resolve()` criou e preenche
/// com base nos items do módulo.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_pass0(
    items: &[kata_ast::ModuleEntry],
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
    // Two-pass: coleta InterfaceDecl e ImplementsDecl para defer,
    // processa o resto (data, enum, alias, refines) inline.
    // Pass 0a: registra interfaces/impls com signatures/methods vazios +
    //   processa data/enum/alias inline (popula type_env, struct_registry).
    // Pass 0b: resolve assinaturas de interfaces e impls (agora todos os
    //   tipos estão disponíveis no type_env e struct_registry).

    /// InterfaceDecl deferido — coletado no passo 0a, resolvido no 0b.
    struct DeferredInterface {
        name: String,
        signatures: Vec<kata_ast::InterfaceSig>,
    }

    /// ImplementsDecl deferido — coletado no passo 0a, resolvido no 0b.
    struct DeferredImpl {
        type_name: String,
        interface_name: String,
        methods: Vec<kata_ast::ImplMethod>,
        /// True se o impl tem `#!allow type.incomplete_interface` anexado.
        /// Quando true, métodos default da interface não são instanciados
        /// para evitar corpos que referenciam métodos não definidos.
        allows_incomplete: bool,
    }

    let mut deferred_interfaces: Vec<DeferredInterface> = Vec::new();
    let mut deferred_impls: Vec<DeferredImpl> = Vec::new();

    // ── Pass 0a: registrar declarações ──────────────────────────
    for item in items {
        match &item.item.node {
            Item::InterfaceDecl {
                name,
                supertraits,
                type_params,
                signatures: iface_sigs,
            } => {
                // Registrar interface com signatures vazias — serão
                // preenchidas no passo 0b, quando todos os tipos (incluindo
                // famílias polimórficas como NonZero) estiverem no type_env.
                let info = InterfaceInfo {
                    name: name.clone(),
                    supertraits: supertraits.clone(),
                    type_params: type_params.clone(),
                    signatures: Vec::new(),
                };
                if let Err(e) = interface_registry.register_interface(origin, info) {
                    eprintln!("[resolution] warning: {e}");
                }
                deferred_interfaces.push(DeferredInterface {
                    name: name.clone(),
                    signatures: iface_sigs.clone(),
                });
            }
            Item::ImplementsDecl {
                type_name,
                type_params,
                interface_name,
                iface_params,
                methods,
            } => {
                // Verificar se o impl tem #!allow ou #!warn
                // type.incomplete_interface. Ambos permitem que a impl
                // incompleta proceeda — a diferença (warn emite diagnóstico,
                // allow silencia) é tratada pelo filter_errors no pipeline.
                let allows_incomplete = item.pragmas.iter().any(|p| {
                    matches!(p, kata_ast::Pragma::DiagnosticControl(dc)
                        if matches!(dc.level,
                            kata_ast::DiagnosticLevel::Allow
                            | kata_ast::DiagnosticLevel::Warn)
                        && dc.code == "type.incomplete_interface")
                });

                // Extrair type_bounds do StructInfo do tipo, se for genérico.
                // `data Complex (...) where T implements SCALAR` registra
                // `type_params: Some([TypeParamDecl { name: "T", bound: Some("SCALAR") }])`
                // no StructRegistry. O impl herda esses bounds.
                let type_bounds = struct_registry
                    .get(type_name)
                    .and_then(|info| info.type_params.as_ref())
                    .map(|params| {
                        params
                            .iter()
                            .filter_map(|p| p.bound.as_ref().map(|b| (p.name.clone(), b.clone())))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                // Registrar impl com methods vazios — serão preenchidos no 0b.
                let entry = ImplEntry {
                    origin: origin.to_string(),
                    type_name: type_name.clone(),
                    type_params: type_params.clone(),
                    interface_name: interface_name.clone(),
                    iface_params: iface_params.clone(),
                    methods: Vec::new(),
                    span: item.item.span,
                    allows_incomplete,
                    type_bounds,
                };
                if let Err(e) = interface_registry.register_impl(entry) {
                    eprintln!("[resolution] warning: {e}");
                }
                deferred_impls.push(DeferredImpl {
                    type_name: type_name.clone(),
                    interface_name: interface_name.clone(),
                    methods: methods.clone(),
                    allows_incomplete,
                });
            }
            // Processar data/enum/alias/refines inline no passo 0a.
            Item::DataDecl {
                name,
                fields,
                directives: data_dirs,
                refined,
                where_bounds,
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
                    let base_ty = resolve_type_expr(
                        &refined_decl.base_ty.node,
                        type_env,
                        interface_registry,
                        &*struct_registry,
                        None,
                    );

                    match &base_ty {
                        Ty::Interface(iface_name) => {
                            // Refined polimórfico: expandir em instâncias por tipo concreto.
                            // Registrar o mapeamento family→iface para permitir
                            // extensão quando novos implementors aparecerem.
                            struct_registry.register_family_iface(name, iface_name);
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
                                    lazy_type_param: None,
                                    extension_impl: None,
                                    allows_incomplete: false,
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
                                        lazy_type_param: None,
                                        extension_impl: None,
                                        allows_incomplete: false,
                                    });
                                }
                                // Registrar o nome público no type_env como Family
                                // — NonZero é família, não struct concreto.
                                type_env.define(
                                    name,
                                    Ty::Struct(StructKey::Family(name.clone())),
                                    origin,
                                );
                            }
                        }
                        _ => {
                            // Verificar se é família polimórfica lazy:
                            // base é coleção parametrizada com type var livre.
                            // Ex: `data (List::A, >= (len _) 1) as NonEmpty`
                            // → base_ty = Ty::List(Ty::Var("A"))
                            let lazy_param = extract_lazy_type_param(&base_ty);
                            if lazy_param.is_some() {
                                // Família polimórfica lazy: registrar como Family,
                                // NÃO instanciar (instâncias criadas on-demand no call-site).
                                let pred_names: Vec<String> = (0..refined_decl.predicates.len())
                                    .map(|i| format!("__pred_{name}_{i}"))
                                    .collect();
                                struct_registry.register_refined(origin, name, "List", pred_names);
                                type_env.define(
                                    name,
                                    Ty::Struct(StructKey::Family(name.clone())),
                                    origin,
                                );
                                refined_decls.push(RefinedDeclInfo {
                                    name: name.clone(),
                                    base_ty,
                                    predicates: refined_decl.predicates.clone(),
                                    lazy_type_param: lazy_param,
                                    extension_impl: None,
                                    allows_incomplete: false,
                                });
                            } else {
                                // Refined concreto: `data (Int, > _ 0) as PositiveInt`
                                let base_ty_name = match &refined_decl.base_ty.node {
                                    TypeExpr::Named(n) => n.clone(),
                                    // `data ([Int], ...) as NonEmpty` →
                                    // TypeExpr::ParamApp { name: "List", ... }
                                    // Extrair o nome do tipo base.
                                    TypeExpr::ParamApp { name, .. } => name.clone(),
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
                                    lazy_type_param: None,
                                    extension_impl: None,
                                    allows_incomplete: false,
                                });
                            }
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

                // data sem campos e sem @ffi reconhecido é inválido:
                // sem construtor, sem show, sem layout — não tem significado.
                if fields.is_empty() && ffi_symbol.is_none() {
                    errors.push(ResolveError::EmptyDataNoFfi { name: name.clone() });
                }

                // Se o DataDecl tem campos não-vazios, registra no StructRegistry.
                // Offset de cada campo = field_index * 8 (todos os campos são words de 8 bytes).
                if !fields.is_empty() {
                    let mut field_infos: Vec<FieldInfo> = fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| FieldInfo {
                            name: f.name.clone(),
                            ty: resolve_type_expr(
                                &f.ty.node,
                                type_env,
                                interface_registry,
                                &*struct_registry,
                                None,
                            ),
                            offset: (i as u32) * 8,
                        })
                        .collect();

                    // Desugar: interfaces em posição de field viram vars
                    // anônimas frescas com bound = a interface. Cada ocorrência
                    // é uma var distinta (PRD §"Forma independente"):
                    // `data Pair (fst::SCALAR scd::SCALAR)` desugar para
                    // `Pair (fst::_SCALAR_0 scd::_SCALAR_1)` com bounds
                    // `_SCALAR_0 implements SCALAR, _SCALAR_1 implements SCALAR`.
                    // Isso permite tipos independentes em cada field.
                    let mut anon_bounds: Vec<(String, String)> = Vec::new();
                    let mut anon_counter = 0usize;
                    for fi in &mut field_infos {
                        desugar_interface_to_var(
                            &mut fi.ty,
                            interface_registry,
                            &mut anon_bounds,
                            &mut anon_counter,
                        );
                    }

                    // Detectar type params: coletar Ty::Var(names) nos fields
                    // onde name é PascalCase (is_type_param_name).
                    // resolve_type_expr já produz Ty::Var("T") para PascalCase
                    // que não é interface nem struct registrado.
                    // Vars anônimas do desugar (_SCALAR_0 etc.) também são
                    // coletadas — is_type_param_name retorna true para elas
                    // (todas maiúsculas + underscores).
                    let mut type_param_names: Vec<String> = Vec::new();
                    for fi in &field_infos {
                        collect_type_param_names(&fi.ty, &mut type_param_names);
                    }

                    if !type_param_names.is_empty() {
                        // Struct paramétrico: construir TypeParamDecls dos where_bounds
                        // + bounds anônimos do desugar de interface.
                        let type_params: Vec<TypeParamDecl> = type_param_names
                            .iter()
                            .map(|pn| {
                                // where_bound explícito tem prioridade.
                                if let Some(bound) = where_bounds
                                    .iter()
                                    .find(|(bn, _)| bn == pn)
                                    .map(|(_, iface)| iface.clone())
                                {
                                    return TypeParamDecl {
                                        name: pn.clone(),
                                        bound: Some(bound),
                                    };
                                }
                                // Bound anônimo do desugar (ex: _SCALAR_0 → SCALAR).
                                if let Some(bound) = anon_bounds
                                    .iter()
                                    .find(|(bn, _)| bn == pn)
                                    .map(|(_, iface)| iface.clone())
                                {
                                    return TypeParamDecl {
                                        name: pn.clone(),
                                        bound: Some(bound),
                                    };
                                }
                                // Sem bound.
                                TypeParamDecl {
                                    name: pn.clone(),
                                    bound: None,
                                }
                            })
                            .collect();
                        struct_registry.register_generic(origin, name, field_infos, type_params);
                    } else {
                        struct_registry.register(origin, name, field_infos);
                    }
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
                            lazy_type_param: rd.lazy_type_param.clone(),
                            extension_impl: None,
                            allows_incomplete: false,
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
                                .map(|p| {
                                    resolve_type_expr(
                                        &p.node,
                                        type_env,
                                        interface_registry,
                                        &*struct_registry,
                                        None,
                                    )
                                })
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
                                .map(|p| {
                                    resolve_type_expr(
                                        &p.node,
                                        type_env,
                                        interface_registry,
                                        &*struct_registry,
                                        None,
                                    )
                                })
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
                        let payload_ty = resolve_type_expr(
                            &payload.node,
                            type_env,
                            interface_registry,
                            &*struct_registry,
                            None,
                        );
                        if let Ty::Var(n) = &payload_ty
                            && is_type_param_name(n)
                        {
                            // type param name ainda não registrado
                            if !type_params.contains(n) {
                                type_params.push(n.clone());
                                // Se o variant tem default, resolve e registra.
                                let default_ty = v.default.as_ref().map(|d| {
                                    resolve_type_expr(
                                        &d.node,
                                        type_env,
                                        interface_registry,
                                        &*struct_registry,
                                        None,
                                    )
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
                            v.payload.as_ref().map(|p| {
                                resolve_type_expr(
                                    &p.node,
                                    type_env,
                                    interface_registry,
                                    &*struct_registry,
                                    None,
                                )
                            })
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

                // O tipo base implementa a interface? A validação final
                // acontece em infer_module, depois do merge com o prelude.
                // (eprintln de warning removido — a validação post-merge em
                // infer/mod.rs cobre ambos os casos: interface inexistente e
                // base não implementa, com mensagens claras.)

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
                        .map(|t| {
                            resolve_type_expr(
                                &t.node,
                                type_env,
                                interface_registry,
                                &*struct_registry,
                                None,
                            )
                        })
                        .collect();
                    let return_type = resolve_type_expr(
                        &m.ret.node,
                        type_env,
                        interface_registry,
                        &*struct_registry,
                        None,
                    );
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
                        param_names: vec![],
                        param_defaults: vec![],
                    });

                    // Método com corpo Kata (lambda) precisa de FunctionDef.
                    if let Some(clauses) = &m.body {
                        functions.push(FunctionDef {
                            name: m.name.clone(),
                            param_types,
                            return_type,
                            clauses: clauses.clone(),
                            cache_strategy: None,
                            cache_capacity: None,
                            timer: None,
                            custom_directives: Vec::new(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // ── Pass 0b: resolver assinaturas de interfaces e impls ──────
    // Agora que todas as declarações (data, enum, interface, implements,
    // refines, refined polimórfico) foram processadas no passo 0a,
    // todos os tipos estão disponíveis no type_env e struct_registry.
    // Isto permite que assinaturas de interface referenciem famílias
    // polimórficas (ex: NonZero) mesmo se declaradas depois da interface.

    // 0b.1: Resolver assinaturas das interfaces.
    for deferred in &deferred_interfaces {
        let iface_sigs: Vec<InterfaceSignature> = deferred
            .signatures
            .iter()
            .map(|s| InterfaceSignature {
                name: s.name.clone(),
                params: s
                    .params
                    .iter()
                    .map(|t| {
                        resolve_type_expr(
                            &t.node,
                            type_env,
                            interface_registry,
                            &*struct_registry,
                            None,
                        )
                    })
                    .collect(),
                ret: resolve_type_expr(
                    &s.ret.node,
                    type_env,
                    interface_registry,
                    &*struct_registry,
                    None,
                ),
                default_body: s.default_body.clone(),
            })
            .collect();
        if let Err(e) =
            interface_registry.update_interface_signatures(origin, &deferred.name, iface_sigs)
        {
            eprintln!("[resolution] warning: {e}");
        }
    }

    // 0b.2: Resolver métodos dos impls + gerar Signatures/FunctionDefs +
    //       processar default methods.
    for deferred in &deferred_impls {
        // Valida diretivas de cada método.
        for m in &deferred.methods {
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

        // Resolver métodos para InterfaceRegistry (ImplMethodInfo).
        let impl_methods: Vec<ImplMethodInfo> = deferred
            .methods
            .iter()
            .map(|m| {
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
                        .map(|t| {
                            let ty = resolve_type_expr(
                                &t.node,
                                type_env,
                                interface_registry,
                                &*struct_registry,
                                None,
                            );
                            let ty = instantiate_family_for_concrete(
                                &ty,
                                &deferred.type_name,
                                struct_registry,
                            );
                            instantiate_generic_struct_refs(&ty, struct_registry)
                        })
                        .collect(),
                    ret: {
                        let ty = resolve_type_expr(
                            &m.ret.node,
                            type_env,
                            interface_registry,
                            &*struct_registry,
                            None,
                        );
                        let ty = instantiate_family_for_concrete(
                            &ty,
                            &deferred.type_name,
                            struct_registry,
                        );
                        instantiate_generic_struct_refs(&ty, struct_registry)
                    },
                    ffi_symbol,
                }
            })
            .collect();
        if let Err(e) = interface_registry.update_impl_methods(
            origin,
            &deferred.type_name,
            &deferred.interface_name,
            impl_methods,
        ) {
            eprintln!("[resolution] warning: {e}");
        }

        // Gerar Signature + FunctionDef para cada método do impl.
        for m in &deferred.methods {
            let param_types: Vec<Ty> = m
                .params
                .iter()
                .map(|t| {
                    let ty = resolve_type_expr(
                        &t.node,
                        type_env,
                        interface_registry,
                        &*struct_registry,
                        None,
                    );
                    let ty =
                        instantiate_family_for_concrete(&ty, &deferred.type_name, struct_registry);
                    instantiate_generic_struct_refs(&ty, struct_registry)
                })
                .collect();
            let return_type = {
                let ty = resolve_type_expr(
                    &m.ret.node,
                    type_env,
                    interface_registry,
                    &*struct_registry,
                    None,
                );
                let ty = instantiate_family_for_concrete(&ty, &deferred.type_name, struct_registry);
                instantiate_generic_struct_refs(&ty, struct_registry)
            };
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
                param_names: vec![],
                param_defaults: vec![],
            });

            // Método com corpo Kata (lambda) precisa de FunctionDef.
            if let Some(clauses) = &m.body {
                functions.push(FunctionDef {
                    name: m.name.clone(),
                    param_types,
                    return_type,
                    clauses: clauses.clone(),
                    cache_strategy: None,
                    cache_capacity: None,
                    timer: None,
                    custom_directives: Vec::new(),
                });
            }
        }

        // ── Default methods: métodos da interface com default_body
        // que não foram definidos no impl. Gera Signature +
        // FunctionDef sintetizada usando o default_body da interface.
        // Self na assinatura é substituído pelo tipo concreto.
        // Percorre supertraits (ex: NUM herda mod// de FIELD).
        let iface_sigs = interface_registry.all_signatures(&deferred.interface_name);
        // Coletar assinaturas definidas no impl (nome + param_types
        // após instantiate_family_for_concrete) para decidir se o
        // default method deve ser pulado. Só pula se o impl define
        // o mesmo método com os mesmos param_types (override real).
        // Cross-type overloads (param_types diferentes) NÃO pulam
        // o default method — elas cobrem combinações de tipos diferentes.
        let defined_sigs: Vec<(String, Vec<Ty>)> = deferred
            .methods
            .iter()
            .map(|m| {
                let pts: Vec<Ty> = m
                    .params
                    .iter()
                    .map(|t| {
                        let ty = resolve_type_expr(
                            &t.node,
                            type_env,
                            interface_registry,
                            &*struct_registry,
                            None,
                        );
                        instantiate_family_for_concrete(&ty, &deferred.type_name, struct_registry)
                    })
                    .collect();
                (m.name.clone(), pts)
            })
            .collect();
        for sig in &iface_sigs {
            if let Some(default_clauses) = &sig.default_body {
                let concrete_ty = resolve_type_expr(
                    &kata_ast::TypeExpr::Named(deferred.type_name.clone()),
                    type_env,
                    interface_registry,
                    &*struct_registry,
                    None,
                );
                let param_types: Vec<Ty> = sig
                    .params
                    .iter()
                    .map(|t| {
                        let ty = t.substitute_self(&concrete_ty);
                        let ty = instantiate_family_for_concrete(&ty, &deferred.type_name, struct_registry);
                        instantiate_generic_struct_refs(&ty, struct_registry)
                    })
                    .collect();
                let return_type = {
                    let ty = sig.ret.substitute_self(&concrete_ty);
                    let ty = instantiate_family_for_concrete(&ty, &deferred.type_name, struct_registry);
                    instantiate_generic_struct_refs(&ty, struct_registry)
                };

                // Pular se o impl já define este método com os mesmos
                // param_types (override real). Cross-type overloads
                // (param_types diferentes) não pular o default method.
                // Também verifica signatures já registradas por OUTROS
                // blocos `implements` do mesmo tipo — sem isso, default
                // methods de supertraits são regenerados duplicadamente
                // quando um novo impl (ex: SCALAR extends NUM) é
                // processado, causando erros de dispatch no corpo.
                let is_overridden = defined_sigs
                    .iter()
                    .any(|(name, pts)| name == &sig.name && pts == &param_types)
                    || signatures
                        .iter()
                        .any(|s| s.name == sig.name && s.param_types == param_types);
                if is_overridden {
                    continue;
                }

                // Se o impl tem #!allow type.incomplete_interface, não
                // instanciar métodos default da interface. O corpo do
                // default pode referenciar métodos que o tipo não define
                // (ex: `mod` chama `/`), causando erros de dispatch
                // confusos. O usuário explicitou que sabe que não
                // implementou tudo — não gerar defaults que podem falhar.
                if deferred.allows_incomplete {
                    continue;
                }

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
                    param_names: vec![],
                    param_defaults: vec![],
                });

                functions.push(FunctionDef {
                    name: sig.name.clone(),
                    param_types,
                    return_type,
                    clauses: default_clauses.clone(),
                    cache_strategy: None,
                    cache_capacity: None,
                    timer: None,
                    custom_directives: Vec::new(),
                });
            }
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

/// Coleta nomes de type params (Ty::Var com nome PascalCase) de um tipo,
/// recursivamente. Remove duplicatas preservando ordem de primeira ocorrência.
/// Usado no pass0 para detectar se um `data` é paramétrico.
fn collect_type_param_names(ty: &Ty, result: &mut Vec<String>) {
    match ty {
        Ty::Var(name) if is_type_param_name(name) && !result.contains(name) => {
            result.push(name.clone());
        }
        Ty::Generic(_, args) => {
            for arg in args {
                collect_type_param_names(arg, result);
            }
        }
        Ty::Struct(StructKey::Generic(_, args)) => {
            for arg in args {
                collect_type_param_names(arg, result);
            }
        }
        Ty::List(inner)
        | Ty::Array(inner)
        | Ty::Range(inner)
        | Ty::Set(inner)
        | Ty::Tensor(inner) => collect_type_param_names(inner, result),
        Ty::Dict(k, v) => {
            collect_type_param_names(k, result);
            collect_type_param_names(v, result);
        }
        Ty::Tuple(elems) => {
            for e in elems {
                collect_type_param_names(e, result);
            }
        }
        Ty::Function(params, ret) | Ty::Action(params, ret) => {
            for p in params {
                collect_type_param_names(p, result);
            }
            collect_type_param_names(ret, result);
        }
        _ => {}
    }
}

/// Desugar: substitui cada `Ty::Interface(name)` por `Ty::Var(fresh_name)`
/// onde `fresh_name` é único por ocorrência. Registra o bound
/// `(fresh_name, name)` em `anon_bounds`.
///
/// Isso implementa a "forma independente" do PRD: `data Pair (fst::SCALAR scd::SCALAR)`
/// desugar para `Pair (fst::_SCALAR_0 scd::_SCALAR_1)` com bounds independentes.
///
/// Recursiva em tipos compostos (Function, Tuple, List, etc.).
fn desugar_interface_to_var(
    ty: &mut Ty,
    iface_reg: &kata_core::InterfaceRegistry,
    anon_bounds: &mut Vec<(String, String)>,
    counter: &mut usize,
) {
    match ty {
        Ty::Interface(name) => {
            // Só desugar se é uma interface registrada. Se não está registrada,
            // é um nome não-resolvido — deixar como está (erro em outro lugar).
            if iface_reg.get_interface(name).is_some() {
                let fresh = format!("_{name}_{counter}");
                *counter += 1;
                anon_bounds.push((fresh.clone(), name.clone()));
                *ty = Ty::Var(fresh);
            }
        }
        Ty::Generic(_, args) => {
            for arg in args.iter_mut() {
                desugar_interface_to_var(arg, iface_reg, anon_bounds, counter);
            }
        }
        Ty::Struct(StructKey::Generic(_, args)) => {
            for arg in args.iter_mut() {
                desugar_interface_to_var(arg, iface_reg, anon_bounds, counter);
            }
        }
        Ty::List(inner) | Ty::Array(inner) | Ty::Range(inner) | Ty::Set(inner)
        | Ty::Tensor(inner) => {
            desugar_interface_to_var(inner, iface_reg, anon_bounds, counter);
        }
        Ty::Dict(k, v) => {
            desugar_interface_to_var(k, iface_reg, anon_bounds, counter);
            desugar_interface_to_var(v, iface_reg, anon_bounds, counter);
        }
        Ty::Tuple(elems) => {
            for e in elems.iter_mut() {
                desugar_interface_to_var(e, iface_reg, anon_bounds, counter);
            }
        }
        Ty::Function(params, ret) | Ty::Action(params, ret) => {
            for p in params.iter_mut() {
                desugar_interface_to_var(p, iface_reg, anon_bounds, counter);
            }
            desugar_interface_to_var(ret, iface_reg, anon_bounds, counter);
        }
        _ => {}
    }
}
