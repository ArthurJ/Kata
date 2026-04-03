use crate::parser::ast::{Module, TopLevel, Spanned, TypeRef, Pattern, Stmt};
use crate::type_checker::environment::TypeEnv;
use crate::type_checker::tast::{TExpr, TStmt};
use crate::type_checker::arity_resolver::ArityResolver;
use crate::type_checker::directives::{KataDirective, validate_and_parse_directives};

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
    Data(String, crate::parser::ast::DataDef, Vec<Spanned<KataDirective>>),
    Enum(String, Vec<crate::parser::ast::Variant>, Vec<Spanned<KataDirective>>),
    Signature(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>, Vec<Spanned<KataDirective>>),
    LambdaDef(Vec<Spanned<Pattern>>, Spanned<TExpr>, Vec<Spanned<TExpr>>, Vec<Spanned<KataDirective>>),
    ActionDef(String, Vec<(String, Spanned<TypeRef>)>, Spanned<TypeRef>, Vec<Spanned<TStmt>>, Vec<Spanned<KataDirective>>),
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
        let mut all_prelude_tast = Vec::new();
        
        for (name, module) in prelude_modules {
            let mut temp_checker = Checker::new();
            temp_checker.compiled_modules = self.compiled_modules.clone();
            
            // We must call check_module to resolve everything and generate TAST
            temp_checker.check_module(module);
            
            all_prelude_tast.extend(temp_checker.tast);
            self.errors.extend(temp_checker.errors);

            let imports = temp_checker.env.imports.clone();
            for (path, specific) in imports {
                if let Some(target_module_name) = path.split('.').next() {
                    if let Some(target_env) = self.compiled_modules.get(target_module_name) {
                        temp_checker.env.import_from(target_env, target_module_name, &specific);
                    }
                }
            }

            temp_checker.env.expand_exports(); // Expand again to catch methods imported from interfaces/types

            self.compiled_modules.insert(name.to_string(), temp_checker.env);
        }
        
        if let Some(prelude_env) = self.compiled_modules.get("prelude") {
            let env_clone = prelude_env.clone();
            let all_exports: Vec<(String, Option<String>)> = env_clone.exports.iter().map(|e| (e.clone(), None)).collect();

            self.env.import_from(&env_clone, "prelude", &all_exports);
        }
        
        self.tast.extend(all_prelude_tast);
        self.local_types.clear();
        self.local_interfaces.clear();
    }

    fn process_lambda_group(&self, group: &mut Vec<&Vec<Spanned<Pattern>>>, sig: &Option<(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>)>, errs: &mut Vec<(String, crate::parser::ast::Span)>) {
        if let Some((name, params, _)) = sig {
            if !group.is_empty() && !params.is_empty() {
                // If any lambda in the group is a catch-all "otherwise" (params.len() == 1 and it's "otherwise"), 
                // then it covers everything.
                let has_otherwise = group.iter().any(|p| p.len() == 1 && matches!(&p[0].0, Pattern::Ident(n) if n == "otherwise"));
                if !has_otherwise {
                    for i in 0..params.len() {
                        let param_type = &params[i].0;
                        let patterns: Vec<&Pattern> = group.iter().filter_map(|p| p.get(i).map(|pat| &pat.0)).collect();
                        self.check_exhaustiveness(param_type, &patterns, errs, &format!("Pattern Matching (arg {}) em `{}`", i, name), &(0..0));
                    }
                }
            }
        }
        group.clear();
    }

    fn replace_hole(expr: &mut crate::parser::ast::Expr, replacement: &crate::parser::ast::Expr) {
        match expr {
            crate::parser::ast::Expr::Hole => {
                *expr = replacement.clone();
            }
            crate::parser::ast::Expr::ActionCall(_, args) => {
                for arg in args {
                    Self::replace_hole(&mut arg.0, replacement);
                }
            }
            crate::parser::ast::Expr::Try(inner) | crate::parser::ast::Expr::ExplicitApp(inner) => {
                Self::replace_hole(&mut inner.0, replacement);
            }
            crate::parser::ast::Expr::Pipe(l, r) => {
                Self::replace_hole(&mut l.0, replacement);
                Self::replace_hole(&mut r.0, replacement);
            }
            crate::parser::ast::Expr::Tuple(es) | crate::parser::ast::Expr::List(es) | crate::parser::ast::Expr::Sequence(es) => {
                for e in es {
                    Self::replace_hole(&mut e.0, replacement);
                }
            }
            crate::parser::ast::Expr::Array(rows) => {
                for row in rows {
                    for e in row {
                        Self::replace_hole(&mut e.0, replacement);
                    }
                }
            }
            crate::parser::ast::Expr::Guard(branches, otherwise) => {
                for (_, body) in branches {
                    Self::replace_hole(&mut body.0, replacement);
                }
                Self::replace_hole(&mut otherwise.0, replacement);
            }
            crate::parser::ast::Expr::Lambda(_, body, with) => {
                Self::replace_hole(&mut body.0, replacement);
                for w in with {
                    Self::replace_hole(&mut w.0, replacement);
                }
            }
            crate::parser::ast::Expr::WithDecl(_, e) => {
                Self::replace_hole(&mut e.0, replacement);
            }
            _ => {}
        }
    }

    fn flatten_declarations(decls: &[Spanned<TopLevel>]) -> Vec<Spanned<TopLevel>> {
        let mut flat = Vec::new();
        for (decl, span) in decls {
            flat.push((decl.clone(), span.clone()));
            match decl {
                TopLevel::Interface(_, _, methods, _) | TopLevel::Implements(_, _, methods) => {
                    flat.extend(Self::flatten_declarations(methods));
                }
                _ => {}
            }
        }
        flat
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

        let flat_decls = Self::flatten_declarations(&module.declarations);

        for (decl, span) in &flat_decls {
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

            let t_decls = self.resolve_top_level(decl, span, &resolver, &mut local_errors, &last_signature);
            for t_decl in t_decls {
                self.tast.push((t_decl, span.clone()));
            }
        }
        self.process_lambda_group(&mut lambda_group, &last_signature, &mut local_errors);

        self.errors.extend(local_errors);
        self.errors.extend(resolver.errors.into_inner());
    }

    fn collect_top_level(&mut self, decl: &TopLevel, span: &crate::parser::ast::Span) {
        let mut local_errs: Vec<(String, crate::parser::ast::Span)> = Vec::new();


        let _extract_test_desc = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> Option<String> {
            for (dir, _dir_span) in dirs {
                if dir.name == "test" {
                    if let Some((crate::parser::ast::Expr::String(desc), _)) = match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.first(), _ => None } {
                        return Some(desc.clone());
                    }
                    return Some("Sem descricao".to_string());
                }
            }
            None
        };



        match decl {
            TopLevel::Data(name, def, dirs) => {
                let (_parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
                self.local_types.insert(name.clone());
                match def {
                    crate::parser::ast::DataDef::Struct(fields) => {
                        let mut params = Vec::new();
                        for _ in 0..fields.len() {
                            params.push((TypeRef::Simple("Unknown".to_string()), 0..0));
                        }
                        self.env.define(name.clone(), fields.len(), TypeRef::Function(params, Box::new((TypeRef::Simple(name.clone()), 0..0))), false, false, None);
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
                            false,
                            None
                        );
                    }
                }
            }
            TopLevel::Enum(name, variants, dirs) => {
                let (_parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
                self.local_types.insert(name.clone());
                let mut has_smart_constructor = false;
                let mut has_predicate = false;
                let mut var_names = Vec::new();

                for variant in variants {
                    var_names.push(variant.name.clone());
                    let (arity, type_info) = match &variant.data {
                        crate::parser::ast::VariantData::Unit => (0, TypeRef::Simple(name.clone())),
                        crate::parser::ast::VariantData::Type(t) => (1, TypeRef::Function(vec![t.clone()], Box::new((TypeRef::Simple(name.clone()), 0..0)))),
                        crate::parser::ast::VariantData::FixedValue(_) => { has_smart_constructor = true; (0, TypeRef::Simple(name.clone())) },
                        crate::parser::ast::VariantData::Predicate(_) => { has_smart_constructor = true; has_predicate = true; (1, TypeRef::Function(vec![(TypeRef::TypeVar("A".to_string()), 0..0)], Box::new((TypeRef::Simple(name.clone()), 0..0)))) },
                    };
                    self.env.define(variant.name.clone(), arity, type_info, false, false, None);
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
                            vec![(TypeRef::TypeVar("A".to_string()), 0..0)],
                            Box::new((TypeRef::Simple(name.clone()), 0..0))
                        ),
                        false,
                        false,
                        None
                    );
                }
            }
            TopLevel::Interface(name, super_traits, methods, dirs) => {
                let (_parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
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
                let (parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
                if let Some(desc) = parsed_dirs.iter().find_map(|(d, _)| if let KataDirective::Test(s) = d { Some(s.clone()) } else { None }) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: false });
                }
                let ffi_name = parsed_dirs.iter().find_map(|(d, _)| if let KataDirective::Ffi(name) = d { Some(name.clone()) } else { None });
                self.env.define(name.clone(), params.len(), TypeRef::Function(params.clone(), Box::new(ret.clone())), false, parsed_dirs.iter().any(|(d, _)| matches!(d, KataDirective::Commutative)), ffi_name);
            }
            TopLevel::ActionDef(name, params, ret, _, dirs) => {
                let (parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
                if let Some(desc) = parsed_dirs.iter().find_map(|(d, _)| if let KataDirective::Test(s) = d { Some(s.clone()) } else { None }) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: true });
                }
                let ffi_name = parsed_dirs.iter().find_map(|(d, _)| if let KataDirective::Ffi(name) = d { Some(name.clone()) } else { None });
                self.env.define(
                    name.clone(), 
                    params.len(), 
                    TypeRef::Function(params.iter().map(|(_, t)| t.clone()).collect(), Box::new(ret.clone())), 
                    true,
                    parsed_dirs.iter().any(|(d, _)| matches!(d, KataDirective::Commutative)),
                    ffi_name
                );
            }
            TopLevel::Alias(name, target, dirs) => {
                let (_parsed_dirs, errs) = validate_and_parse_directives(dirs);
                local_errs.extend(errs);
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

    fn resolve_top_level(&self, decl: &TopLevel, _span: &crate::parser::ast::Span, resolver: &ArityResolver, local_errors: &mut Vec<(String, crate::parser::ast::Span)>, last_signature: &Option<(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>)>) -> Vec<TTopLevel> {
        match decl {
            TopLevel::Data(name, def, dirs) => vec![TTopLevel::Data(name.clone(), def.clone(), validate_and_parse_directives(dirs).0)],
            TopLevel::Enum(name, variants, dirs) => {
                let mut has_predicate = false;
                for variant in variants {
                    if matches!(variant.data, crate::parser::ast::VariantData::Predicate(_)) {
                        has_predicate = true; break;
                    }
                }

                let mut decls = Vec::new();

                if has_predicate {
                    let mut branches = Vec::new();
                    let mut otherwise_branch = None;

                    for variant in variants {
                        if let crate::parser::ast::VariantData::Predicate(expr) = &variant.data {
                            // Construir a condição: aplicar o predicado ao parâmetro recebido `val`
                            let (op, rhs) = if let crate::parser::ast::Expr::Sequence(seq) = expr {
                                if seq.len() == 3 && matches!(seq[1].0, crate::parser::ast::Expr::Hole) {
                                    (seq[0].clone(), seq[2].clone())
                                } else { panic!("Formato invalido de predicado") }
                            } else { panic!("Formato invalido de predicado") };

                            let val_ident = (TExpr::Ident("__val".to_string(), TypeRef::Simple("A".to_string())), _span.clone());
                            
                            // Resolver o operador para o TAST
                            let mut op_expr = resolver.resolve_expr(&op);
                            if let TExpr::Ident(ref op_name, _) = op_expr.0 {
                                op_expr = (TExpr::Ident(op_name.clone(), TypeRef::Function(vec![(TypeRef::Simple("A".to_string()), 0..0), (TypeRef::Simple("A".to_string()), 0..0)], Box::new((TypeRef::Simple("Bool".to_string()), 0..0)))), _span.clone());
                            }
                            let rhs_expr = resolver.resolve_expr(&rhs);
                            
                            let cond_call = (TExpr::Call(Box::new(op_expr), vec![val_ident.clone(), rhs_expr], TypeRef::Simple("Bool".to_string())), _span.clone());

                            let variant_callee = Box::new((TExpr::Ident(variant.name.clone(), TypeRef::Simple(name.clone())), _span.clone()));
                            let body_call = (TExpr::Call(variant_callee, vec![val_ident], TypeRef::Simple(name.clone())), _span.clone());

                            branches.push((cond_call, body_call));
                        } else {
                            let val_ident = (TExpr::Ident("__val".to_string(), TypeRef::Simple("A".to_string())), _span.clone());
                            let variant_callee = Box::new((TExpr::Ident(variant.name.clone(), TypeRef::Simple(name.clone())), _span.clone()));
                            let body_call = (TExpr::Call(variant_callee, vec![val_ident], TypeRef::Simple(name.clone())), _span.clone());
                            otherwise_branch = Some(body_call);
                        }
                    }

                    if let Some(other) = otherwise_branch {
                        let guard_expr = (TExpr::Guard(branches, Box::new(other), TypeRef::Simple(name.clone())), _span.clone());
                        let lambda_def = TTopLevel::LambdaDef(
                            vec![(Pattern::Ident("__val".to_string()), _span.clone())],
                            guard_expr,
                            Vec::new(),
                            Vec::new()
                        );
                        
                        let sig_def = TTopLevel::Signature(
                            name.clone(),
                            vec![(TypeRef::Simple("A".to_string()), _span.clone())],
                            (TypeRef::Simple(name.clone()), _span.clone()),
                            Vec::new()
                        );

                        decls.push(sig_def);
                        decls.push(lambda_def);
                    }
                }

                decls.push(TTopLevel::Enum(name.clone(), variants.clone(), validate_and_parse_directives(dirs).0));
                decls
            }
            TopLevel::Signature(name, params, ret, dirs) => vec![TTopLevel::Signature(name.clone(), params.clone(), ret.clone(), validate_and_parse_directives(dirs).0)],
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
                    vec![TTopLevel::LambdaDef(params.clone(), t_body, t_with, validate_and_parse_directives(dirs).0)]
                } else {
                    let t_body = resolver.resolve_expr(body);
                    let mut t_with = Vec::new();
                    for w in with {
                        t_with.push(resolver.resolve_expr(w));
                    }
                    vec![TTopLevel::LambdaDef(params.clone(), t_body, t_with, validate_and_parse_directives(dirs).0)]
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

                let parsed_dirs = validate_and_parse_directives(dirs).0;
                let is_ffi = parsed_dirs.iter().any(|(d, _)| matches!(d, KataDirective::Ffi(_)));

                if !is_ffi && !resolver.types_compatible(&last_stmt_ty, &ret.0) {
                    local_errors.push((format!("Type Mismatch: Expected return type `{:?}`, Found `{:?}` in Action `{}`", ret.0, last_stmt_ty, name), ret.1.clone()));
                }

                vec![TTopLevel::ActionDef(name.clone(), params.clone(), ret.clone(), t_body, parsed_dirs)]
            }
            TopLevel::Execution(expr) => {
                resolver.pure_context.set(false); // Top-level execution allows actions
                *resolver.current_action.borrow_mut() = None;
                
                resolver.local_vars.borrow_mut().clear();
                resolver.constraints.borrow_mut().clear();
                
                let t_expr = resolver.resolve_expr(expr);
                vec![TTopLevel::Execution(t_expr)]
            }
            _ => Vec::new(),
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
                        TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty, _) | TExpr::Sequence(_, ty) => ty.clone(),
                        _ => TypeRef::Simple("Unknown".to_string()),
                    };
                    resolver.declare_local(name.clone(), ty);
                }
                (TStmt::Let(p.clone(), t_e), span.clone())
            }
            Stmt::Var(n, e) => {
                let t_e = resolver.resolve_expr(e);
                let ty = match &t_e.0 {
                    TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty, _) | TExpr::Sequence(_, ty) => ty.clone(),
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

                let target_type = match &t_target.0 {
                    TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Sequence(_, ty) => ty.clone(),
                    _ => TypeRef::Simple("Unknown".to_string()),
                };

                for arm in arms {
                    let crate::parser::ast::MatchArm::Pattern((pat, pat_span), stmts) = arm;
                    
                    // Salvar o estado anterior do escopo local para não vazar variáveis do arm atual
                    let old_locals = resolver.local_vars.borrow().clone();

                    // Inferência de Tipo do Payload do Enum
                    if let Pattern::Sequence(atoms) = pat {
                        if atoms.len() == 2 {
                            if let (Pattern::Ident(variant_name), _) = &atoms[0] {
                                if let (Pattern::Ident(var_name), _) = &atoms[1] {
                                    let mut payload_ty = TypeRef::Simple("Unknown".to_string());
                                    
                                    if let TypeRef::Generic(enum_name, args) = &target_type {
                                        if enum_name == "Result" && args.len() == 2 {
                                            payload_ty = if variant_name == "Ok" { args[0].0.clone() } else { args[1].0.clone() };
                                        } else if enum_name == "Optional" && args.len() == 1 {
                                            payload_ty = args[0].0.clone();
                                        }
                                    } else if let TypeRef::Simple(enum_name) = &target_type {
                                        if let Some(info) = self.env.lookup_first(variant_name) {
                                            if let TypeRef::Function(params, _) = &info.type_info {
                                                if !params.is_empty() {
                                                    payload_ty = params[0].0.clone();
                                                }
                                            }
                                        }
                                    }
                                    
                                    resolver.declare_local(var_name.clone(), payload_ty);
                                }
                            }
                        }
                    }

                    let mut t_stmts = Vec::new();
                    for s in stmts {
                        t_stmts.push(self.resolve_stmt(s, resolver, local_errors));
                    }

                    // Restaurar o escopo
                    *resolver.local_vars.borrow_mut() = old_locals;

                    t_arms.push(crate::type_checker::tast::TMatchArm {
                        pattern: (pat.clone(), pat_span.clone()),
                        body: t_stmts,
                    });
                }

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
                    if (name == "List" || name == "Array" || name == "Range" || name == "ITERABLE") && !args.is_empty() {
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
            Stmt::Select(arms, timeout) => {
                let mut t_arms = Vec::new();
                for arm in arms {
                    let t_op = resolver.resolve_expr(&arm.operation);
                    
                    if let Some(ref b) = arm.binding {
                        if let Pattern::Ident(name) = &b.0 {
                            let ty = match &t_op.0 {
                                TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Sequence(_, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty, _) => ty.clone(),
                                _ => TypeRef::Simple("Unknown".to_string()),
                            };
                            resolver.declare_local(name.clone(), ty);
                        }
                    }

                    let mut t_body = Vec::new();
                    for s in &arm.body {
                        t_body.push(self.resolve_stmt(s, resolver, local_errors));
                    }
                    
                    t_arms.push(crate::type_checker::tast::TSelectArm {
                        operation: t_op,
                        binding: arm.binding.clone(),
                        body: t_body,
                    });
                }
                
                let t_timeout = if let Some((t_expr, t_stmts)) = timeout {
                    let tt_expr = resolver.resolve_expr(t_expr);
                    let mut tt_body = Vec::new();
                    for s in t_stmts {
                        tt_body.push(self.resolve_stmt(s, resolver, local_errors));
                    }
                    Some((tt_expr, tt_body))
                } else {
                    None
                };
                
                (TStmt::Select(t_arms, t_timeout), span.clone())
            }
            Stmt::ActionAssign(target, val) => {
                let t_target = resolver.resolve_expr(target);
                let t_val = resolver.resolve_expr(val);
                (TStmt::ActionAssign(t_target, t_val), span.clone())
            }
            Stmt::Break => (TStmt::Break, span.clone()),
            Stmt::Continue => (TStmt::Continue, span.clone()),
        }
    }
}
