use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt, TMatchArm};
use crate::parser::ast::{Spanned, TypeRef};
use crate::type_checker::environment::TypeEnv;
use crate::optimizer::error::OptimizerError;
use std::collections::HashMap;

pub struct Monomorphizer<'a> {
    pub _env: &'a TypeEnv,
    pub instantiations: HashMap<String, Vec<TTopLevel>>,
    pub templates: HashMap<String, Vec<TTopLevel>>,
}

impl<'a> Monomorphizer<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self {
            _env: env,
            instantiations: HashMap::new(),
            templates: HashMap::new(),
        }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Monomorfização (Zero-Cost Generics)...");

        let mut current_sig_name = None;
        for (decl, _) in &tast {
            match decl {
                TTopLevel::Signature(name, _, _, _) => {
                    current_sig_name = Some(name.clone());
                    self.templates.entry(name.clone()).or_default().push(decl.clone());
                }
                TTopLevel::LambdaDef(..) => {
                    if let Some(name) = &current_sig_name {
                        self.templates.entry(name.clone()).or_default().push(decl.clone());
                    }
                }
                TTopLevel::ActionDef(name, ..) => {
                    current_sig_name = None;
                    self.templates.entry(name.clone()).or_default().push(decl.clone());
                }
                TTopLevel::Execution(_) => {
                    current_sig_name = None;
                }
                _ => {
                    current_sig_name = None;
                }
            }
        }
        
        let mut folded_tast: Vec<Spanned<TTopLevel>> = tast.into_iter()
            .map(|(decl, span)| (self.fold_toplevel(decl, errors), span))
            .collect();

        // Recursively keep folding new instantiations if they also contain generic calls
        let mut new_functions_to_add = Vec::new();
        while !self.instantiations.is_empty() {
            let current_insts: Vec<(String, Vec<TTopLevel>)> = self.instantiations.drain().collect();
            for (_, inst_decls) in current_insts {
                for nova_def in inst_decls {
                    let folded_def = self.fold_toplevel(nova_def, errors);
                    new_functions_to_add.push((folded_def, 0..0));
                }
            }
        }

        folded_tast.extend(new_functions_to_add);
        folded_tast
    }

    fn fold_toplevel(&mut self, decl: TTopLevel, errors: &mut Vec<OptimizerError>) -> TTopLevel {
        match decl {
            TTopLevel::LambdaDef(params, body, with, dirs) => {
                let folded_body = self.fold_expr_spanned(body, errors);
                let folded_with = with.into_iter().map(|w| self.fold_expr_spanned(w, errors)).collect();
                TTopLevel::LambdaDef(params, folded_body, folded_with, dirs)
            }
            TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                let folded_body = body.into_iter().map(|s| self.fold_stmt_spanned(s, errors)).collect();
                TTopLevel::ActionDef(name, params, ret, folded_body, dirs)
            }
            TTopLevel::Execution(expr) => {
                TTopLevel::Execution(self.fold_expr_spanned(expr, errors))
            }
            other => other,
        }
    }

    fn fold_stmt_spanned(&mut self, stmt: Spanned<TStmt>, errors: &mut Vec<OptimizerError>) -> Spanned<TStmt> {
        let (s, span) = stmt;
        let folded = match s {
            TStmt::Let(pat, expr) => TStmt::Let(pat, self.fold_expr_spanned(expr, errors)),
            TStmt::Var(name, expr) => TStmt::Var(name, self.fold_expr_spanned(expr, errors)),
            TStmt::Loop(body) => TStmt::Loop(body.into_iter().map(|s| self.fold_stmt_spanned(s, errors)).collect()),
            TStmt::For(name, iter, body) => TStmt::For(
                name,
                self.fold_expr_spanned(iter, errors),
                body.into_iter().map(|s| self.fold_stmt_spanned(s, errors)).collect(),
            ),
            TStmt::Match(target, arms) => {
                let folded_target = self.fold_expr_spanned(target, errors);
                let folded_arms = arms
                    .into_iter()
                    .map(|arm| TMatchArm {
                        pattern: arm.pattern,
                        body: arm.body.into_iter().map(|s| self.fold_stmt_spanned(s, errors)).collect(),
                    })
                    .collect();
                TStmt::Match(folded_target, folded_arms)
            }
            TStmt::Expr(expr) => TStmt::Expr(self.fold_expr_spanned(expr, errors)),
            TStmt::Break => TStmt::Break,
            TStmt::Continue => TStmt::Continue,
        };
        (folded, span)
    }

    fn fold_expr_spanned(&mut self, expr: Spanned<TExpr>, errors: &mut Vec<OptimizerError>) -> Spanned<TExpr> {
        let (e, span) = expr;
        let folded = match e {
            TExpr::Call(callee, args, ty) => {
                let mut folded_callee = Box::new(self.fold_expr_spanned(*callee, errors));
                let folded_args: Vec<_> = args.into_iter().map(|a| self.fold_expr_spanned(a, errors)).collect();

                if let TExpr::Ident(name, ref callee_ty) = &folded_callee.0 {
                    if self.is_generic_type(callee_ty) {
                        let concrete_types = folded_args.iter().map(|a| crate::type_checker::arity_resolver::ArityResolver::get_expr_type(&a.0)).collect::<Vec<_>>();
                        let mangled_name = self.mangle_name(name, &concrete_types);

                        self.schedule_instantiation(name, &mangled_name, &concrete_types);

                        folded_callee = Box::new((TExpr::Ident(mangled_name, callee_ty.clone()), folded_callee.1.clone()));
                    }
                }

                TExpr::Call(folded_callee, folded_args, ty)
            }
            TExpr::Tuple(exprs, ty, alloc) => TExpr::Tuple(exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(), ty.clone(), alloc),
            TExpr::List(exprs, ty, alloc) => TExpr::List(exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(), ty.clone(), alloc),
            TExpr::Lambda(params, body, ty) => TExpr::Lambda(params, Box::new(self.fold_expr_spanned(*body, errors)), ty),
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(), ty),
            TExpr::Guard(branches, otherwise, ty) => {
                let mut folded_branches = Vec::new();
                for (cond, body) in branches {
                    folded_branches.push((
                        self.fold_expr_spanned(cond, errors),
                        self.fold_expr_spanned(body, errors)
                    ));
                }
                TExpr::Guard(folded_branches, Box::new(self.fold_expr_spanned(*otherwise, errors)), ty)
            }
            TExpr::Try(inner, ty) => TExpr::Try(Box::new(self.fold_expr_spanned(*inner, errors)), ty),
            TExpr::ChannelSend(target, val, ty) => TExpr::ChannelSend(
                Box::new(self.fold_expr_spanned(*target, errors)),
                Box::new(self.fold_expr_spanned(*val, errors)),
                ty,
            ),
            TExpr::ChannelRecv(target, ty) => TExpr::ChannelRecv(Box::new(self.fold_expr_spanned(*target, errors)), ty),
            TExpr::ChannelRecvNonBlock(target, ty) => TExpr::ChannelRecvNonBlock(Box::new(self.fold_expr_spanned(*target, errors)), ty),
            other => other,
        };
        (folded, span)
    }

    fn is_generic_type(&self, ty: &TypeRef) -> bool {
        match ty {
            TypeRef::Simple(n) if n.len() == 1 && n.chars().next().unwrap().is_uppercase() => true,
            TypeRef::Generic(_, args) => args.iter().any(|a| self.is_generic_type(&a.0)),
            TypeRef::Function(args, ret) => args.iter().any(|a| self.is_generic_type(&a.0)) || self.is_generic_type(&ret.0),
            TypeRef::Refined(base, _) => self.is_generic_type(&base.0),
            TypeRef::Variadic(inner) => self.is_generic_type(&inner.0),
            _ => false,
        }
    }

    fn mangle_name(&self, base_name: &str, concrete_types: &[TypeRef]) -> String {
        let mut name = base_name.to_string();
        for ty in concrete_types {
            name.push('_');
            name.push_str(&self.type_to_string(ty));
        }
        name
    }

    fn type_to_string(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Simple(n) => n.clone(),
            TypeRef::Generic(n, args) => {
                let args_str: Vec<String> = args.iter().map(|a| self.type_to_string(&a.0)).collect();
                format!("{}_{}", n, args_str.join("_"))
            }
            TypeRef::Function(_, _) => "Func".to_string(),
            TypeRef::Refined(base, _) => format!("Refined_{}", self.type_to_string(&base.0)),
            TypeRef::Variadic(inner) => format!("Var_{}", self.type_to_string(&inner.0)),
        }
    }

    fn build_mapping(&self, param: &TypeRef, concrete: &TypeRef, mapping: &mut HashMap<String, TypeRef>) {
        match (param, concrete) {
            (TypeRef::Simple(p_name), _) if p_name.len() == 1 && p_name.chars().next().unwrap().is_uppercase() => {
                mapping.insert(p_name.clone(), concrete.clone());
            }
            (TypeRef::Generic(_, p_args), TypeRef::Generic(_, c_args)) => {
                for (p_a, c_a) in p_args.iter().zip(c_args.iter()) {
                    self.build_mapping(&p_a.0, &c_a.0, mapping);
                }
            }
            (TypeRef::Function(p_args, p_ret), TypeRef::Function(c_args, c_ret)) => {
                for (p_a, c_a) in p_args.iter().zip(c_args.iter()) {
                    self.build_mapping(&p_a.0, &c_a.0, mapping);
                }
                self.build_mapping(&p_ret.0, &c_ret.0, mapping);
            }
            _ => {}
        }
    }

    fn schedule_instantiation(&mut self, original_name: &str, mangled_name: &str, concrete_types: &[TypeRef]) {
        if self.instantiations.contains_key(mangled_name) {
            return;
        }

        if let Some(template_decls) = self.templates.get(original_name).cloned() {
            let mut mapping = HashMap::new();
            for decl in &template_decls {
                if let TTopLevel::Signature(_, params, _, _) = decl {
                    for (i, param) in params.iter().enumerate() {
                        if i < concrete_types.len() {
                            self.build_mapping(&param.0, &concrete_types[i], &mut mapping);
                        }
                    }
                    break;
                }
            }

            let substituter = TypeSubstituter { mapping };
            let mut new_decls = Vec::new();

            for decl in template_decls {
                match decl {
                    TTopLevel::Signature(_, params, ret, dirs) => {
                        let new_params = params.iter().map(|(t, s)| (substituter.substitute_type(t), s.clone())).collect();
                        let new_ret = (substituter.substitute_type(&ret.0), ret.1.clone());
                        new_decls.push(TTopLevel::Signature(mangled_name.to_string(), new_params, new_ret, dirs));
                    }
                    TTopLevel::LambdaDef(params, body, with, dirs) => {
                        let new_body = substituter.substitute_expr(&body);
                        let new_with = with.iter().map(|w| substituter.substitute_expr(w)).collect();
                        new_decls.push(TTopLevel::LambdaDef(params, new_body, new_with, dirs));
                    }
                    TTopLevel::ActionDef(_, params, ret, body, dirs) => {
                        let new_params = params.iter().map(|(n, (t, s))| (n.clone(), (substituter.substitute_type(t), s.clone()))).collect();
                        let new_ret = (substituter.substitute_type(&ret.0), ret.1.clone());
                        let new_body = body.iter().map(|s| substituter.substitute_stmt(s)).collect();
                        new_decls.push(TTopLevel::ActionDef(mangled_name.to_string(), new_params, new_ret, new_body, dirs));
                    }
                    other => {
                        new_decls.push(other);
                    }
                }
            }

            self.instantiations.insert(mangled_name.to_string(), new_decls);
        }
    }
}

struct TypeSubstituter {
    mapping: HashMap<String, TypeRef>,
}

impl TypeSubstituter {
    fn substitute_type(&self, ty: &TypeRef) -> TypeRef {
        match ty {
            TypeRef::Simple(n) => {
                if let Some(concrete) = self.mapping.get(n) {
                    concrete.clone()
                } else {
                    ty.clone()
                }
            }
            TypeRef::Generic(n, args) => {
                TypeRef::Generic(n.clone(), args.iter().map(|(a, s)| (self.substitute_type(a), s.clone())).collect())
            }
            TypeRef::Function(args, ret) => {
                TypeRef::Function(
                    args.iter().map(|(a, s)| (self.substitute_type(a), s.clone())).collect(),
                    Box::new((self.substitute_type(&ret.0), ret.1.clone()))
                )
            }
            TypeRef::Refined(base, preds) => {
                TypeRef::Refined(Box::new((self.substitute_type(&base.0), base.1.clone())), preds.clone())
            }
            TypeRef::Variadic(inner) => {
                TypeRef::Variadic(Box::new((self.substitute_type(&inner.0), inner.1.clone())))
            }
        }
    }

    fn substitute_expr(&self, expr: &Spanned<TExpr>) -> Spanned<TExpr> {
        let (e, span) = expr;
        let new_e = match e {
            TExpr::Ident(name, ty) => TExpr::Ident(name.clone(), self.substitute_type(ty)),
            TExpr::Call(callee, args, ty) => {
                TExpr::Call(
                    Box::new(self.substitute_expr(callee)),
                    args.iter().map(|a| self.substitute_expr(a)).collect(),
                    self.substitute_type(ty)
                )
            }
            TExpr::Tuple(exprs, ty, alloc) => TExpr::Tuple(exprs.iter().map(|e| self.substitute_expr(e)).collect(), self.substitute_type(ty), *alloc),
            TExpr::List(exprs, ty, alloc) => TExpr::List(exprs.iter().map(|e| self.substitute_expr(e)).collect(), self.substitute_type(ty), *alloc),
            TExpr::Lambda(params, body, ty) => TExpr::Lambda(params.clone(), Box::new(self.substitute_expr(body)), self.substitute_type(ty)),
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(exprs.iter().map(|e| self.substitute_expr(e)).collect(), self.substitute_type(ty)),
            TExpr::Guard(branches, otherwise, ty) => {
                let mut new_branches = Vec::new();
                for (cond, body) in branches {
                    new_branches.push((self.substitute_expr(cond), self.substitute_expr(body)));
                }
                TExpr::Guard(new_branches, Box::new(self.substitute_expr(otherwise)), self.substitute_type(ty))
            }
            TExpr::Try(inner, ty) => TExpr::Try(Box::new(self.substitute_expr(inner)), self.substitute_type(ty)),
            TExpr::ChannelSend(target, val, ty) => TExpr::ChannelSend(
                Box::new(self.substitute_expr(target)),
                Box::new(self.substitute_expr(val)),
                self.substitute_type(ty)
            ),
            TExpr::ChannelRecv(target, ty) => TExpr::ChannelRecv(Box::new(self.substitute_expr(target)), self.substitute_type(ty)),
            TExpr::ChannelRecvNonBlock(target, ty) => TExpr::ChannelRecvNonBlock(Box::new(self.substitute_expr(target)), self.substitute_type(ty)),
            other => other.clone(),
        };
        (new_e, span.clone())
    }

    fn substitute_stmt(&self, stmt: &Spanned<TStmt>) -> Spanned<TStmt> {
        let (s, span) = stmt;
        let new_s = match s {
            TStmt::Let(pat, expr) => TStmt::Let(pat.clone(), self.substitute_expr(expr)),
            TStmt::Var(name, expr) => TStmt::Var(name.clone(), self.substitute_expr(expr)),
            TStmt::Loop(body) => TStmt::Loop(body.iter().map(|s| self.substitute_stmt(s)).collect()),
            TStmt::For(name, iter, body) => TStmt::For(name.clone(), self.substitute_expr(iter), body.iter().map(|s| self.substitute_stmt(s)).collect()),
            TStmt::Match(target, arms) => {
                let new_arms = arms.iter().map(|arm| TMatchArm {
                    pattern: arm.pattern.clone(),
                    body: arm.body.iter().map(|s| self.substitute_stmt(s)).collect(),
                }).collect();
                TStmt::Match(self.substitute_expr(target), new_arms)
            }
            TStmt::Expr(expr) => TStmt::Expr(self.substitute_expr(expr)),
            TStmt::Break => TStmt::Break,
            TStmt::Continue => TStmt::Continue,
        };
        (new_s, span.clone())
    }
}