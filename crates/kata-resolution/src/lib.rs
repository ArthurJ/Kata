//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

pub mod module_loader;
pub(crate) mod prelude;
mod prelude_sigs;

use kata_ast::{ActionStmt, Expr, Item, LambdaClause, Module, Spanned, TypeExpr};
use kata_core::{
    EnumRegistry, FieldInfo, ImplEntry, ImplMethodInfo, InterfaceInfo, InterfaceRegistry,
    InterfaceSignature, PrimTy, StructRegistry, Ty, TypeEnv, VariantInfo,
};

/// Resultado da resolution — TypeEnv populado + assinaturas coletadas.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub type_env: TypeEnv,
    pub signatures: Vec<Signature>,
    /// Catálogo de variantes por enum (Fio 2).
    pub enum_registry: EnumRegistry,
    /// Catálogo de structs com campos e offsets (Fio 5).
    pub struct_registry: StructRegistry,
    /// Fio 6: declarações refined pendentes para o inference sintetizar
    /// funções predicado e smart constructors falíveis.
    pub refined_decls: Vec<RefinedDeclInfo>,
    /// Fio 6: enums com variantes predicadas pendentes para o inference
    /// sintetizar o construtor despachador.
    pub enum_pred_decls: Vec<EnumPredDeclInfo>,
    /// Fio 7: catálogo de interfaces e implementações.
    pub interface_registry: InterfaceRegistry,
    /// Funções nomeadas com corpo Kata (Fio 2 Fase 10).
    /// Cada entrada preserva as cláusulas lambda para o inference processar.
    pub functions: Vec<FunctionDef>,
    /// Actions definidas no módulo (Fio 3).
    pub actions: Vec<ActionDef>,
}

/// Assinatura de função coletada no Pass 1.
#[derive(Debug, Clone)]
pub struct Signature {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub ffi_symbol: Option<String>,
    pub is_associative: bool,
    pub associative_neutral: Option<i64>,
    /// Se `true`, esta assinatura é uma Action (Fio 3).
    /// Actions são chamadas com `!` e têm `is_action = true` no DispatchTable.
    pub is_action: bool,
    /// Fase 5: type params da assinatura genérica (ex: `["T"]` para `id :: T => T`).
    /// Vazio para funções não-genéricas. Coletado examinando os `Ty::Var` em
    /// param_types e return_type cujo nome é UPPER_CASE e não está no TypeEnv.
    pub type_params: Vec<String>,
}

/// Definição de função nomeada com corpo Kata (não-FFI).
///
/// Produzida no resolution quando `Item::Sig` tem `body = Some(clauses)`.
/// O inference consome as cláusulas e produz `TypedExprKind::Lambda` com
/// `func_name = Some(name)` e os tipos da assinatura.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub clauses: Vec<Spanned<LambdaClause>>,
}

/// Definição de Action com body Kata (Fio 3).
///
/// Produzida no resolution quando `Item::ActionDecl` é encontrado.
/// O inference consome o body e produz `TypedAction`.
#[derive(Debug, Clone)]
pub struct ActionDef {
    pub name: String,
    pub param_types: Vec<Ty>,
    pub return_type: Ty,
    pub body: Vec<ActionStmt>,
}

/// Informação de um tipo refinado declarado pelo usuário (Fio 6).
/// O inference sintetiza funções predicado e smart constructor falível a partir desta info.
#[derive(Debug, Clone)]
pub struct RefinedDeclInfo {
    /// Nome do tipo refinado (ex: "PositiveInt").
    pub name: String,
    /// Tipo base (ex: Ty::Prim(PrimTy::Int)).
    pub base_ty: Ty,
    /// Predicados como `Spanned<Expr>` (com Hole como placeholder).
    pub predicates: Vec<Spanned<Expr>>,
}

/// Informação de um enum com variantes predicadas (Fio 6).
/// O inference sintetiza o construtor que despacha para a variante correta.
#[derive(Debug, Clone)]
pub struct EnumPredDeclInfo {
    /// Nome do enum (ex: "IMC").
    pub name: String,
    /// Tipo do payload comum a todas as variantes (ex: Ty::Prim(PrimTy::Float)).
    pub payload_ty: Ty,
    /// Variantes predicadas: (nome, predicado, tag).
    /// A última variante (sem predicado) é o fallback/default.
    pub variants: Vec<EnumPredVariant>,
}

/// Variante de um enum predicado.
#[derive(Debug, Clone)]
pub struct EnumPredVariant {
    /// Nome da variante (ex: "Magreza").
    pub name: String,
    /// Predicado como `Spanned<Expr>` (com Hole como placeholder).
    /// None = variante default/fallback.
    pub predicate: Option<Spanned<Expr>>,
    /// Tag da variante no enum (índice na declaração).
    pub tag: usize,
}

/// Erro de resolution (wrapped FrontendError/MiddleError).
#[derive(Debug, Clone)]
pub enum ResolveError {
    UnknownType { name: String },
    UnknownFfi { name: String },
    DuplicateSignature { name: String },
}

/// Resolve um módulo: Pass 0 + Pass 1.
pub fn resolve(module: &Module) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut type_env = TypeEnv::new();
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

                // data Int () com @ffi("i64") → Ty::Prim(PrimTy::Int)
                // Por enquanto, registra como Struct. O FfiSymbol será resolvido
                // na inferência quando cruzar com a diretiva @ffi.
                type_env.define(name, Ty::Struct(name.clone()));

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
                enum_registry.register(name, variant_infos);

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
                        let ffi_symbol = m.directives.iter().find_map(|d| {
                            if d.name == "ffi"
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
                    type_params,
                });
            }
            Item::ActionDecl {
                name,
                params,
                ret,
                body,
                ..
            } => {
                // Converte TypeExpr → Ty para os parâmetros e retorno.
                let param_types: Vec<Ty> = params
                    .iter()
                    .map(|t| resolve_type_expr(&t.node, &type_env, &interface_registry))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env, &interface_registry);

                actions.push(ActionDef {
                    name: name.clone(),
                    param_types,
                    return_type,
                    body: body.clone(),
                });
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

/// Converte TypeExpr → Ty usando TypeEnv para resolver nomes.
///
/// Se `name` é uma interface registrada no `InterfaceRegistry`, produz
/// `Ty::Interface(name)` em vez de `Ty::Struct(name)`.
fn resolve_type_expr(expr: &TypeExpr, env: &TypeEnv, iface_reg: &InterfaceRegistry) -> Ty {
    match expr {
        TypeExpr::Named(name) => {
            // Tenta resolver no TypeEnv
            if let Some(ty) = env.lookup(name) {
                ty.clone()
            } else {
                // Tipos conhecidos do prelude
                match name.as_str() {
                    "Int" => Ty::Prim(PrimTy::Int),
                    "Float" => Ty::Prim(PrimTy::Float),
                    "Text" => Ty::Prim(PrimTy::Text),
                    "Rational" => Ty::Prim(PrimTy::Rational),
                    "Boolean" => Ty::Sum("Boolean".into()),
                    "Unit" => Ty::Unit,
                    _ => {
                        // Se é uma interface registrada, produz Ty::Interface.
                        if iface_reg.get_interface(name).is_some() {
                            Ty::Interface(name.clone())
                        } else if is_type_param_name(name) {
                            // Fase 5: UPPER_CASE sem :: é type param (ex: T, E, A).
                            Ty::Var(name.clone())
                        } else {
                            Ty::Struct(name.clone()) // fallback: tipo declarado pelo usuário
                        }
                    }
                }
            }
        }
        TypeExpr::Unit => Ty::Unit,
        TypeExpr::Grouping(inner) => resolve_type_expr(&inner.node, env, iface_reg),
        TypeExpr::Tuple(elements) => {
            let tys: Vec<Ty> = elements
                .iter()
                .map(|t| resolve_type_expr(&t.node, env, iface_reg))
                .collect();
            Ty::Tuple(tys)
        }
        TypeExpr::Func { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env, iface_reg))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env, iface_reg);
            Ty::Function(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, params } => {
            // Fase 6: Result::(Int, Text) → resolve params → Ty::Generic("Result", [Int, Text]).
            // Se o enum é genérico no EnumRegistry, produz Ty::Generic.
            // Se não é genérico (fallback), produz Ty::Sum como antes.
            let resolved_params: Vec<Ty> = params
                .iter()
                .map(|p| resolve_type_expr(&p.node, env, iface_reg))
                .collect();
            // Tenta resolver como Ty::Var se o param é um nome que não está no TypeEnv
            // (ex: "T" em Result::(T, E) dentro de uma declaração de função genérica).
            Ty::Generic(name.clone(), resolved_params)
        }
        // Fio 7: Self é resolvido na Fase 2 (resolution de implements).
        // Por ora, mapeia para Ty::Var("Self") como placeholder.
        TypeExpr::SelfRef => Ty::Var("Self".into()),
    }
}

pub use prelude_sigs::load_prelude;

/// Infere o tipo do payload a partir do predicado da variante.
///
/// `Magreza(< _ 18.5)` → predicado `Apply { Ident("<"), [Hole, FloatLit("18.5")] }`
/// → o tipo do payload é o tipo do literal (`Float`).
///
/// Suporta predicados no formato `op _ literal` (Apply com callee Ident e args [Hole, literal]).
fn infer_payload_ty_from_pred(expr: &Expr) -> Option<Ty> {
    if let Expr::Apply { callee, args } = expr {
        // callee deve ser Ident (operador)
        if matches!(callee.node, Expr::Ident { .. }) {
            // args[0] deve ser Hole, args[1] deve ser literal
            if args.len() == 2 && matches!(args[0].node, Expr::Hole) {
                return match &args[1].node {
                    Expr::IntLit { .. } => Some(Ty::Prim(PrimTy::Int)),
                    Expr::FloatLit { .. } => Some(Ty::Prim(PrimTy::Float)),
                    Expr::TextLit { .. } => Some(Ty::Prim(PrimTy::Text)),
                    _ => None,
                };
            }
        }
    }
    None
}

/// Fase 5: verifica se um nome é um type param.
///
/// Convenção: UPPER_CASE (todas as letras maiúsculas, pelo menos 1 char).
/// `T`, `E`, `A` → true. `Int`, `Complex`, `NUM` → false (tem minúsculas).
/// `Self` → false (não é type param genérico, é placeholder de interface).
fn is_type_param_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_uppercase()) && !name.is_empty() && name != "Self"
}

/// Fase 5: coleta type params de uma assinatura resolvida.
///
/// Percorre param_types e return_type recursivamente buscando `Ty::Var(name)`
/// onde `name` é UPPER_CASE. Recursa em `Ty::Generic` args. Remove duplicatas,
/// preservando ordem de primeira ocorrência.
fn collect_type_params(param_types: &[Ty], return_type: &Ty) -> Vec<String> {
    fn collect_into(ty: &Ty, result: &mut Vec<String>) {
        match ty {
            Ty::Var(name) if is_type_param_name(name) && !result.contains(name) => {
                result.push(name.clone());
            }
            Ty::Generic(_, args) => {
                for arg in args {
                    collect_into(arg, result);
                }
            }
            _ => {}
        }
    }
    let mut result: Vec<String> = Vec::new();
    for ty in param_types {
        collect_into(ty, &mut result);
    }
    collect_into(return_type, &mut result);
    result
}
