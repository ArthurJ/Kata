//! Pass 0 + Pass 1: resolution.
//!
//! - Pass 0: popula TypeEnv com tipos declarados (`data` → Struct, `enum` → Sum)
//! - Pass 1: coleta assinaturas de funções `@ffi` e registra no DispatchTable
//!
//! Produz o `ResolvedModule` (imutável).

pub mod prelude;

use kata_ast::{Expr, Item, LambdaClause, Module, Spanned, TypeExpr};
use kata_core::{EnumRegistry, PrimTy, Ty, TypeEnv};

/// Resultado da resolution — TypeEnv populado + assinaturas coletadas.
#[derive(Debug, Clone)]
pub struct ResolvedModule {
    pub type_env: TypeEnv,
    pub signatures: Vec<Signature>,
    /// Catálogo de variantes por enum (Fio 2).
    pub enum_registry: EnumRegistry,
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
    pub body: Vec<Spanned<Expr>>,
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
    let errors: Vec<ResolveError> = Vec::new();

    // Pass 0: popula TypeEnv com tipos declarados
    for item in &module.items {
        match &item.node {
            Item::DataDecl { name, .. } => {
                // data Int () com @ffi("i64") → Ty::Prim(PrimTy::Int)
                // Por enquanto, registra como Struct. O FfiSymbol será resolvido
                // na inferência quando cruzar com a diretiva @ffi.
                type_env.define(name, Ty::Struct(name.clone()));
            }
            Item::EnumDecl { name, variants, .. } => {
                type_env.define(name, Ty::Sum(name.clone()));
                // Fio 2: cataloga variantes no EnumRegistry
                enum_registry.register(name, variants.iter().map(|v| v.name.clone()).collect());
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
                    .map(|t| resolve_type_expr(&t.node, &type_env))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env);

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
                    .map(|t| resolve_type_expr(&t.node, &type_env))
                    .collect();
                let return_type = resolve_type_expr(&ret.node, &type_env);

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
        functions,
        actions,
    })
}

/// Converte TypeExpr → Ty usando TypeEnv para resolver nomes.
fn resolve_type_expr(expr: &TypeExpr, env: &TypeEnv) -> Ty {
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
                    _ => Ty::Struct(name.clone()), // fallback: tipo declarado pelo usuário
                }
            }
        }
        TypeExpr::Unit => Ty::Unit,
        TypeExpr::Grouping(inner) => resolve_type_expr(&inner.node, env),
        TypeExpr::Func { params, ret } => {
            let param_types: Vec<Ty> = params
                .iter()
                .map(|t| resolve_type_expr(&t.node, env))
                .collect();
            let return_type = resolve_type_expr(&ret.node, env);
            Ty::Function(param_types, Box::new(return_type))
        }
        TypeExpr::ParamApp { name, params: _ } => {
            // Fio 4: Result::(T, E) — por enquanto resolve como Sum
            Ty::Sum(name.clone())
        }
    }
}

/// Carrega o prelude hardcoded e retorna o TypeEnv + signatures populados.
/// O driver chama isto antes de resolver o módulo do usuário.
pub fn load_prelude() -> Result<ResolvedModule, Vec<ResolveError>> {
    // O prelude é parseado como um módulo Kata normal
    // Por enquanto, construímos o TypeEnv manualmente — Fio 10 fará parse real
    let mut type_env = TypeEnv::new();

    // Tipos do prelude
    type_env.define("Int", Ty::Prim(PrimTy::Int));
    type_env.define("Float", Ty::Prim(PrimTy::Float));
    type_env.define("Text", Ty::Prim(PrimTy::Text));
    type_env.define("Rational", Ty::Prim(PrimTy::Rational));
    type_env.define("Boolean", Ty::Sum("Boolean".into()));
    type_env.define("Unit", Ty::Unit);

    // Variantes de Boolean
    // (serão registradas como construtores no DispatchTable na inferência)
    let mut enum_registry = EnumRegistry::new();
    enum_registry.register("Boolean", vec!["True".into(), "False".into()]);

    // Assinaturas do prelude
    let signatures = vec![
        // Int aritmética
        sig(
            "+",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_add",
            true,
            Some(0),
        ),
        sig(
            "-",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_sub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_mul",
            true,
            Some(1),
        ),
        sig(
            "/",
            vec![Ty::int(), Ty::int()],
            Ty::int(),
            "kata_rt_bi_div",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::int(), Ty::int()],
            Ty::boolean(),
            "kata_rt_bi_gt",
            false,
            None,
        ),
        // Float aritmética
        sig(
            "+",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fadd",
            false,
            None,
        ),
        sig(
            "-",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fsub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fmul",
            false,
            None,
        ),
        sig(
            "/",
            vec![Ty::float(), Ty::float()],
            Ty::float(),
            "kata_rt_fdiv",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::float(), Ty::float()],
            Ty::boolean(),
            "kata_rt_fcmp_gt",
            false,
            None,
        ),
        // Rational aritmética
        sig(
            "+",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_add",
            true,
            Some(0),
        ),
        sig(
            "-",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_sub",
            false,
            None,
        ),
        sig(
            "*",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_mul",
            true,
            Some(1),
        ),
        sig(
            "/",
            vec![Ty::rational(), Ty::rational()],
            Ty::rational(),
            "kata_rt_rat_div",
            false,
            None,
        ),
        sig(
            "=",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_eq",
            false,
            None,
        ),
        sig(
            "<",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_lt",
            false,
            None,
        ),
        sig(
            ">",
            vec![Ty::rational(), Ty::rational()],
            Ty::boolean(),
            "kata_rt_rat_gt",
            false,
            None,
        ),
        // Conversões
        sig(
            "to_float",
            vec![Ty::rational()],
            Ty::float(),
            "kata_rt_rat_to_float",
            false,
            None,
        ),
        sig(
            "from_float",
            vec![Ty::float()],
            Ty::rational(),
            "kata_rt_rat_from_float",
            false,
            None,
        ),
        sig(
            "from_int",
            vec![Ty::int()],
            Ty::rational(),
            "kata_rt_int_to_rational",
            false,
            None,
        ),
        // I/O — echo é uma Action builtin (Fio 3)
        sig_action(
            "echo",
            vec![Ty::text()],
            Ty::Unit,
            "kata_rt_print",
        ),
        // Show
        sig(
            "show",
            vec![Ty::int()],
            Ty::text(),
            "kata_rt_bi_show",
            false,
            None,
        ),
        sig(
            "show",
            vec![Ty::rational()],
            Ty::text(),
            "kata_rt_rat_show",
            false,
            None,
        ),
    ];

    Ok(ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        functions: Vec::new(),
        actions: Vec::new(),
    })
}

/// Helper para construir Signature
fn sig(
    name: &str,
    params: Vec<Ty>,
    ret: Ty,
    ffi: &str,
    assoc: bool,
    neutral: Option<i64>,
) -> Signature {
    Signature {
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        ffi_symbol: Some(ffi.to_string()),
        is_associative: assoc,
        associative_neutral: neutral,
        is_action: false,
    }
}

/// Helper para construir Signature de Action builtin (Fio 3).
fn sig_action(name: &str, params: Vec<Ty>, ret: Ty, ffi: &str) -> Signature {
    Signature {
        name: name.to_string(),
        param_types: params,
        return_type: ret,
        ffi_symbol: Some(ffi.to_string()),
        is_associative: false,
        associative_neutral: None,
        is_action: true,
    }
}
