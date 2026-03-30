use crate::parser::ast::{Module, TopLevel, Spanned, TypeRef, Pattern, Stmt};
use crate::type_checker::environment::TypeEnv;
use crate::type_checker::tast::{TExpr, TStmt};
use crate::type_checker::arity_resolver::ArityResolver;

#[derive(Debug, Clone, PartialEq)]
pub struct TestInfo {
    pub name: String,
    pub description: String,
    pub is_action: bool,
}

pub struct Checker {
    pub env: TypeEnv,
    pub tast: Vec<Spanned<TTopLevel>>,
    pub errors: Vec<(String, crate::parser::ast::Span)>,
    pub tests: Vec<TestInfo>,
    pub local_types: std::collections::HashSet<String>,
    pub local_interfaces: std::collections::HashSet<String>,
    pub compiled_modules: std::collections::HashMap<String, TypeEnv>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TTopLevel {
    Data(String, crate::parser::ast::DataDef, Vec<Spanned<crate::parser::ast::Directive>>),
    Enum(String, Vec<crate::parser::ast::Variant>, Vec<Spanned<crate::parser::ast::Directive>>),
    Signature(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>, Vec<Spanned<crate::parser::ast::Directive>>),
    LambdaDef(Vec<Spanned<Pattern>>, Spanned<TExpr>, Vec<Spanned<TExpr>>, Vec<Spanned<crate::parser::ast::Directive>>),
    ActionDef(String, Vec<(String, Spanned<TypeRef>)>, Spanned<TypeRef>, Vec<Spanned<TStmt>>, Vec<Spanned<crate::parser::ast::Directive>>),
    Execution(Spanned<TExpr>),
}

impl Checker {
    pub fn new() -> Self {
        Self {
            env: TypeEnv::new(),
            tast: Vec::new(),
            errors: Vec::new(),
            tests: Vec::new(),
            local_types: std::collections::HashSet::new(),
            local_interfaces: std::collections::HashSet::new(),
            compiled_modules: std::collections::HashMap::new(),
        }
    }

    pub fn load_prelude(&mut self, prelude_modules: &[(&str, Module)]) {
        for (name, module) in prelude_modules {
            let mut temp_checker = Checker::new();
            temp_checker.compiled_modules = self.compiled_modules.clone();
            
            for decl in &module.declarations {
                temp_checker.collect_top_level(&decl.0, &decl.1);
            }
            
            let imports = temp_checker.env.imports.clone();
            for (path, specific) in imports {
                if let Some(target_module_name) = path.split('.').next() {
                    if let Some(target_env) = self.compiled_modules.get(target_module_name) {
                        temp_checker.env.import_from(target_env, target_module_name, &specific);
                    }
                }
            }

            // Mark all items as exported if this is the core prelude? No, the .kata files have `export` statements!
            // Wait, we just keep whatever they `export`.
            self.compiled_modules.insert(name.to_string(), temp_checker.env);
        }
        
        // After all prelude modules are compiled, the actual "prelude.kata" module is the one we want to inject globally.
        if let Some(prelude_env) = self.compiled_modules.get("prelude") {
            let env_clone = prelude_env.clone();
            let all_exports: Vec<(String, Option<String>)> = env_clone.exports.iter().map(|e| (e.clone(), None)).collect();
            // Injeção global silenciosa
            self.env.import_from(&env_clone, "prelude", &all_exports);
        }
        
        self.local_types.clear();
        self.local_interfaces.clear();
    }

    fn process_lambda_group(&self, group: &mut Vec<&Vec<Spanned<Pattern>>>, sig: &Option<(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>)>, errs: &mut Vec<(String, crate::parser::ast::Span)>) {
        if let Some((name, params, _)) = sig {
            if !group.is_empty() && !params.is_empty() {
                for i in 0..params.len() {
                    let param_type = &params[i].0;
                    let patterns: Vec<&Pattern> = group.iter().filter_map(|p| p.get(i).map(|pat| &pat.0)).collect();
                    self.check_exhaustiveness(param_type, &patterns, errs, &format!("Pattern Matching (arg {}) em `{}`", i, name), &(0..0));
                }
            }
        }
        group.clear();
    }

    pub fn check_module(&mut self, module: &Module) {
        let mut local_errors: Vec<(String, crate::parser::ast::Span)> = Vec::new();
        for (decl, span) in &module.declarations {
            self.collect_top_level(decl, span);
        }
        
        self.env.expand_exports();

        let resolver = ArityResolver::new(&self.env);
        let mut last_signature: Option<(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>)> = None;
        let mut lambda_group: Vec<&Vec<Spanned<Pattern>>> = Vec::new();

        for (decl, span) in &module.declarations {
            match decl {
                TopLevel::Signature(name, params, ret, _) => {
                    self.process_lambda_group(&mut lambda_group, &last_signature, &mut local_errors);
                    last_signature = Some((name.clone(), params.clone(), ret.clone()));
                }
                TopLevel::LambdaDef(params, _, _, _) => {
                    lambda_group.push(params);
                }
                _ => {
                    self.process_lambda_group(&mut lambda_group, &last_signature, &mut local_errors);
                    last_signature = None;
                }
            }

            if let Some(t_decl) = self.resolve_top_level(decl, span, &resolver, &mut local_errors, &last_signature) {
                self.tast.push((t_decl, span.clone()));
            }
        }
        self.process_lambda_group(&mut lambda_group, &last_signature, &mut local_errors);

        self.errors.extend(local_errors);
        self.errors.extend(resolver.errors.into_inner());
    }

    fn collect_top_level(&mut self, decl: &TopLevel, span: &crate::parser::ast::Span) {
        let mut local_errs: Vec<(String, crate::parser::ast::Span)> = Vec::new();

        let validate_directives = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> Vec<(String, crate::parser::ast::Span)> {
            let mut errs: Vec<(String, crate::parser::ast::Span)> = Vec::new();
            for (dir, dir_span) in dirs {
                if dir.name == "log" {
                    if let Some((arg, _)) = match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.first(), _ => None } {
                        if let crate::parser::ast::Expr::Ident(level) = arg {
                            let valid_variants = ["Error", "Warn", "Info", "Debug", "Trace"];
                            if !valid_variants.contains(&level.as_str()) {
                                errs.push((format!("Diretiva @log invalida: Variante de LogLevel '{}' desconhecida. Use uma de {:?}", level, valid_variants), dir_span.clone()));
                            }
                        } else {
                            errs.push(("Diretiva @log invalida: O primeiro argumento deve ser uma variante de LogLevel (ex: Info).".to_string(), dir_span.clone()));
                        }
                    }
                }
            }
            errs
        };

        let extract_test_desc = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> Option<String> {
            for (dir, dir_span) in dirs {
                if dir.name == "test" {
                    if let Some((crate::parser::ast::Expr::String(desc), _)) = match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.first(), _ => None } {
                        return Some(desc.clone());
                    }
                    return Some("Sem descricao".to_string());
                }
            }
            None
        };

        let is_commutative = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> bool {
            dirs.iter().any(|(d, _)| d.name == "commutative")
        };

        match decl {
            TopLevel::Data(name, def, dirs) => {
                local_errs.extend(validate_directives(dirs));
                self.local_types.insert(name.clone());
                match def {
                    crate::parser::ast::DataDef::Struct(fields) => {
                        self.env.define(name.clone(), fields.len(), TypeRef::Simple(name.clone()), false, false);
                    }
                    crate::parser::ast::DataDef::Refined((base, _), predicates) => {
                        self.env.define_refined(name.clone(), base.clone(), predicates.iter().map(|(e, _)| e.clone()).collect());
                        // Smart constructor para tipos refinados: (Base) -> Result::(Refined, Text)
                        self.env.define(
                            name.clone(),
                            1,
                            TypeRef::Function(
                                vec![(base.clone(), 0..0)],
                                Box::new((
                                    TypeRef::Generic(
                                        "Result".to_string(),
                                        vec![
                                            (TypeRef::Simple(name.clone()), 0..0),
                                            (TypeRef::Simple("Text".to_string()), 0..0)
                                        ]
                                    ),
                                    0..0
                                ))
                            ),
                            false,
                            false
                        );
                    }
                }
            }
            TopLevel::Enum(name, variants, dirs) => {
                local_errs.extend(validate_directives(dirs));
                self.local_types.insert(name.clone());
                let mut has_smart_constructor = false;
                let mut has_predicate = false;
                let mut var_names = Vec::new();

                for variant in variants {
                    var_names.push(variant.name.clone());
                    let arity = match &variant.data {
                        crate::parser::ast::VariantData::Unit => 0,
                        crate::parser::ast::VariantData::Type(_) => 1,
                        crate::parser::ast::VariantData::FixedValue(_) => { has_smart_constructor = true; 0 },
                        crate::parser::ast::VariantData::Predicate(_) => { has_smart_constructor = true; has_predicate = true; 1 },
                    };
                    self.env.define(variant.name.clone(), arity, TypeRef::Simple(name.clone()), false, false);
                }

                if has_predicate {
                    let len = variants.len();
                    for (i, variant) in variants.iter().enumerate() {
                        let is_pred = matches!(variant.data, crate::parser::ast::VariantData::Predicate(_));
                        if i < len - 1 && !is_pred {
                            local_errs.push((format!("Enum Predicativo Invalido ({}): Variante '{}' no meio da declaracao nao possui predicado.", name, variant.name), span.clone()));
                        } else if i == len - 1 && is_pred {
                            local_errs.push((format!("Enum Predicativo Invalido ({}): A ultima variante ('{}') deve ser um catch-all sem predicado.", name, variant.name), span.clone()));
                        }
                    }
                }

                self.env.define_enum(name.clone(), var_names);

                if has_smart_constructor {
                    self.env.define(
                        name.clone(),
                        1,
                        TypeRef::Function(
                            vec![(TypeRef::Simple("A".to_string()), 0..0)],
                            Box::new((TypeRef::Simple(name.clone()), 0..0))
                        ),
                        false,
                        false
                    );
                }
            }
            TopLevel::Interface(name, super_traits, methods, dirs) => {
                local_errs.extend(validate_directives(dirs));
                self.local_interfaces.insert(name.clone());
                self.env.define_interface(name.clone(), super_traits.clone());
                let mut method_names = Vec::new();
                for (m, span) in methods {
                    if let TopLevel::Signature(m_name, _, _, _) = m {
                        method_names.push(m_name.clone());
                    }
                    self.collect_top_level(m, span);
                }
                self.env.interface_methods.insert(name.clone(), method_names);
            }
            TopLevel::Implements(type_name, interface_name, methods) => {
                if !self.local_types.contains(type_name) && !self.local_interfaces.contains(interface_name) {
                    local_errs.push((format!(
                        "Erro de Coerencia (Orphan Rule): Nao eh permitido implementar a Interface `{}` para o Tipo `{}` pois ambos sao externos ao modulo atual.",
                        interface_name, type_name
                    ), span.clone()));
                }
                self.env.define_implementation(type_name.clone(), interface_name.clone());
                let mut method_names = Vec::new();
                for (m, span) in methods {
                    if let TopLevel::Signature(m_name, _, _, _) = m {
                        method_names.push(m_name.clone());
                    }
                    self.collect_top_level(m, span);
                }
                self.env.type_methods.entry(type_name.clone()).or_default().extend(method_names);
            }
            TopLevel::Signature(name, params, ret, dirs) => {
                local_errs.extend(validate_directives(dirs));
                if let Some(desc) = extract_test_desc(dirs) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: false });
                }
                self.env.define(name.clone(), params.len(), TypeRef::Function(params.clone(), Box::new(ret.clone())), false, is_commutative(dirs));
            }
            TopLevel::ActionDef(name, params, ret, _, dirs) => {
                local_errs.extend(validate_directives(dirs));
                if let Some(desc) = extract_test_desc(dirs) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: true });
                }
                self.env.define(
                    name.clone(), 
                    params.len(), 
                    TypeRef::Function(params.iter().map(|(_, t)| t.clone()).collect(), Box::new(ret.clone())), 
                    true,
                    is_commutative(dirs)
                );
            }
            TopLevel::Alias(name, target, dirs) => {
                local_errs.extend(validate_directives(dirs));
                self.local_types.insert(name.clone());
                self.env.define_alias(name.clone(), target.clone());
            }
            TopLevel::Export(names) => {
                for name in names {
                    self.env.exports.insert(name.clone());
                }
            }
            TopLevel::Import(path, specific) => {
                self.env.imports.push((path.clone(), specific.clone()));
            }
            _ => {}
        }

        self.errors.extend(local_errs);
    }

    fn resolve_top_level(&self, decl: &TopLevel, span: &crate::parser::ast::Span, resolver: &ArityResolver, local_errors: &mut Vec<(String, crate::parser::ast::Span)>, last_signature: &Option<(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>)>) -> Option<TTopLevel> {
        match decl {
            TopLevel::Data(name, def, dirs) => Some(TTopLevel::Data(name.clone(), def.clone(), dirs.clone())),
            TopLevel::Enum(name, variants, dirs) => Some(TTopLevel::Enum(name.clone(), variants.clone(), dirs.clone())),
            TopLevel::Signature(name, params, ret, dirs) => Some(TTopLevel::Signature(name.clone(), params.clone(), ret.clone(), dirs.clone())),
            TopLevel::LambdaDef(params, body, with, dirs) => {
                resolver.pure_context.set(true);
                *resolver.current_action.borrow_mut() = None;

                resolver.local_vars.borrow_mut().clear();
                resolver.constraints.borrow_mut().clear();

                for (w, _) in with {
                    if let crate::parser::ast::Expr::WithDecl(name, expr) = w {
                        if let crate::parser::ast::Expr::Ident(interface_name) = &expr.0 {
                            resolver.constraints.borrow_mut().insert(name.clone(), interface_name.clone());
                        }
                    }
                }

                if let Some((_, sig_params, expected_ret)) = last_signature {
                    for (i, (pat, _)) in params.iter().enumerate() {
                        if let Pattern::Ident(name) = pat {
                            let ty = if i < sig_params.len() {
                                sig_params[i].0.clone()
                            } else {
                                TypeRef::Simple("Unknown".to_string())
                            };
                            resolver.declare_local(name.clone(), ty);
                        }
                    }

                    // Checagem de retorno do lambda
                    let t_body = resolver.resolve_expr(body);
                    let body_ty = ArityResolver::get_expr_type(&t_body.0);
                    if !resolver.types_compatible(&body_ty, &expected_ret.0) {
                        local_errors.push((format!("Type Mismatch: Expected return type `{:?}`, Found `{:?}` in Lambda", expected_ret.0, body_ty), body.1.clone()));
                    }

                    let mut t_with = Vec::new();
                    for w in with {
                        t_with.push(resolver.resolve_expr(w));
                    }
                    Some(TTopLevel::LambdaDef(params.clone(), t_body, t_with, dirs.clone()))
                } else {
                    let t_body = resolver.resolve_expr(body);
                    let mut t_with = Vec::new();
                    for w in with {
                        t_with.push(resolver.resolve_expr(w));
                    }
                    Some(TTopLevel::LambdaDef(params.clone(), t_body, t_with, dirs.clone()))
                }
            }
            TopLevel::ActionDef(name, params, ret, body, dirs) => {
                resolver.pure_context.set(false); 
                *resolver.current_action.borrow_mut() = Some(name.clone()); 

                resolver.local_vars.borrow_mut().clear();
                resolver.constraints.borrow_mut().clear();

                for (p_name, p_ty) in params {
                    resolver.declare_local(p_name.clone(), p_ty.0.clone());
                }

                let mut t_body = Vec::new();
                let mut last_stmt_ty = TypeRef::Simple("()".into());
                for stmt in body {
                    let res_stmt = self.resolve_stmt(stmt, resolver, local_errors);
                    if let TStmt::Expr(ref e) = res_stmt.0 {
                        last_stmt_ty = ArityResolver::get_expr_type(&e.0);
                    }
                    t_body.push(res_stmt);
                }

                if !resolver.types_compatible(&last_stmt_ty, &ret.0) {
                    local_errors.push((format!("Type Mismatch: Expected return type `{:?}`, Found `{:?}` in Action `{}`", ret.0, last_stmt_ty, name), ret.1.clone()));
                }

                Some(TTopLevel::ActionDef(name.clone(), params.clone(), ret.clone(), t_body, dirs.clone()))
            }
            TopLevel::Execution(expr) => {
                resolver.pure_context.set(false); // Top-level execution allows actions
                *resolver.current_action.borrow_mut() = None;
                
                resolver.local_vars.borrow_mut().clear();
                resolver.constraints.borrow_mut().clear();
                
                let t_expr = resolver.resolve_expr(expr);
                Some(TTopLevel::Execution(t_expr))
            }
            _ => None,
        }
    }
    fn check_exhaustiveness(&self, target_type: &TypeRef, patterns: &[&Pattern], local_errors: &mut Vec<(String, crate::parser::ast::Span)>, context: &str, span: &crate::parser::ast::Span) {
        let enum_name = match target_type {
            TypeRef::Simple(n) => n.clone(),
            TypeRef::Generic(n, _) => n.clone(),
            _ => return,
        };

        if let Some(expected_variants) = self.env.enums.get(&enum_name) {
                let mut covered_variants = std::collections::HashSet::new();
                let mut has_catch_all = false;

                for pat in patterns {
                    match pat {
                        Pattern::Ident(name) => {
                            if name == "otherwise" { has_catch_all = true; } 
                            else { covered_variants.insert(name.clone()); }
                        }
                        Pattern::Sequence(atoms) => {
                            if let Some((Pattern::Ident(name), _)) = atoms.first() {
                                covered_variants.insert(name.clone());
                            }
                        }
                        Pattern::Hole => { has_catch_all = true; }
                        _ => {}
                    }
                }

                if !has_catch_all {
                    let mut missing = Vec::new();
                    for var in expected_variants {
                        if !covered_variants.contains(var) { missing.push(var.clone()); }
                    }
                    if !missing.is_empty() {
                        local_errors.push((format!("Erro Semantico: {} em `{}` nao eh exaustivo. Faltam: {:?}", context, enum_name, missing), span.clone()));
                    }
                }
            }
    }

    fn resolve_stmt(&self, stmt: &Spanned<Stmt>, resolver: &ArityResolver, local_errors: &mut Vec<(String, crate::parser::ast::Span)>) -> Spanned<TStmt> {
        let (s, span) = stmt;
        match s {
            Stmt::Let(p, e) => {
                let t_e = resolver.resolve_expr(e);
                if let Pattern::Ident(name) = &p.0 {
                    let ty = match &t_e.0 {
                        TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty) | TExpr::Sequence(_, ty) => ty.clone(),
                        _ => TypeRef::Simple("Unknown".to_string()),
                    };
                    resolver.declare_local(name.clone(), ty);
                }
                (TStmt::Let(p.clone(), t_e), span.clone())
            }
            Stmt::Var(n, e) => {
                let t_e = resolver.resolve_expr(e);
                let ty = match &t_e.0 {
                    TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty) | TExpr::Sequence(_, ty) => ty.clone(),
                    _ => TypeRef::Simple("Unknown".to_string()),
                };
                resolver.declare_local(n.clone(), ty);
                (TStmt::Var(n.clone(), t_e), span.clone())
            }
            Stmt::Expr(e) => (TStmt::Expr(resolver.resolve_expr(e)), span.clone()),
            Stmt::Match(target, arms) => {
                let t_target = resolver.resolve_expr(target);
                let mut t_arms = Vec::new();
                
                let patterns: Vec<&Pattern> = arms.iter().map(|arm| {
                    let crate::parser::ast::MatchArm::Pattern((pat, _), _) = arm;
                    pat
                }).collect();

                for arm in arms {
                    let crate::parser::ast::MatchArm::Pattern((pat, pat_span), stmts) = arm;
                    let mut t_stmts = Vec::new();
                    for s in stmts {
                        t_stmts.push(self.resolve_stmt(s, resolver, local_errors));
                    }
                    t_arms.push(crate::type_checker::tast::TMatchArm {
                        pattern: (pat.clone(), pat_span.clone()),
                        body: t_stmts,
                    });
                }

                let target_type = match &t_target.0 {
                    TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Sequence(_, ty) => ty.clone(),
                    _ => TypeRef::Simple("Unknown".to_string()),
                };

                self.check_exhaustiveness(&target_type, &patterns, local_errors, "Match", span);

                (TStmt::Match(t_target, t_arms), span.clone())
            }
            Stmt::Loop(body) => {
                let mut t_body = Vec::new();
                for s in body {
                    t_body.push(self.resolve_stmt(s, resolver, local_errors));
                }
                (TStmt::Loop(t_body), span.clone())
            }
            Stmt::For(ident, target, body) => {
                let t_target = resolver.resolve_expr(target);
                let target_ty = match &t_target.0 {
                    TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Sequence(_, ty) | TExpr::List(_, ty, _) | TExpr::Tuple(_, ty, _) => ty.clone(),
                    _ => TypeRef::Simple("Unknown".to_string()),
                };
                
                let elem_ty = if let TypeRef::Generic(name, args) = &target_ty {
                    if (name == "List" || name == "Array" || name == "Range") && !args.is_empty() {
                        args[0].0.clone()
                    } else {
                        TypeRef::Simple("Unknown".to_string())
                    }
                } else {
                    TypeRef::Simple("Unknown".to_string())
                };

                resolver.declare_local(ident.clone(), elem_ty);
                
                let mut t_body = Vec::new();
                for s in body {
                    t_body.push(self.resolve_stmt(s, resolver, local_errors));
                }
                (TStmt::For(ident.clone(), t_target, t_body), span.clone())
            }
            _ => (TStmt::Break, span.clone()),
        }
    }
}
