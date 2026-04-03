use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt};
use crate::type_checker::environment::TypeEnv;
use crate::parser::ast::{Spanned, TypeRef, Expr};
use std::collections::HashSet;

pub struct TreeShaker<'a> {
    pub reachable_functions: HashSet<String>,
    pub reachable_types: HashSet<String>,
    pub env: &'a TypeEnv,
}

impl<'a> TreeShaker<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self {
            reachable_functions: HashSet::new(),
            reachable_types: HashSet::new(),
            env,
        }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Early Tree-Shaking...");
        
        for export_name in &self.env.exports {
            self.reachable_functions.insert(export_name.clone());
            self.reachable_types.insert(export_name.clone());
        }

        for (decl, _) in &tast {
            if let TTopLevel::Execution(expr) = decl {
                self.visit_expr(expr);
            }
        }
        
        let mut progress = true;
        while progress {
            progress = false;
            let initial_funcs = self.reachable_functions.len();
            let initial_types = self.reachable_types.len();
            
            let mut current_sig_reachable = false;

            for (decl, _) in &tast {
                match decl {
                    TTopLevel::Signature(name, params, ret, _) => {
                        current_sig_reachable = self.reachable_functions.contains(name);
                        if current_sig_reachable {
                            for (ty, _) in params { self.visit_type(ty); }
                            self.visit_type(&ret.0);
                        }
                    }
                    TTopLevel::LambdaDef(_params, body, with, _) => {
                        if current_sig_reachable {
                            self.visit_expr(body);
                            for w in with {
                                self.visit_expr(w);
                            }
                        }
                    }
                    TTopLevel::ActionDef(name, params, ret, body, _) => {
                        current_sig_reachable = false;
                        if self.reachable_functions.contains(name) {
                            for (_, (ty, _)) in params { self.visit_type(ty); }
                            self.visit_type(&ret.0);
                            for stmt in body {
                                self.visit_stmt(stmt);
                            }
                        }
                    }
                    TTopLevel::Data(name, def, _) => {
                        current_sig_reachable = false;
                        if self.reachable_types.contains(name) || self.reachable_functions.contains(name) {
                            self.reachable_types.insert(name.clone());
                            self.reachable_functions.insert(name.clone());
                            match def {
                                crate::parser::ast::DataDef::Struct(_) => {}
                                crate::parser::ast::DataDef::Refined((base, _), preds) => {
                                    self.visit_type(base);
                                    for (p, _) in preds {
                                        self.visit_ast_expr(p);
                                    }
                                }
                            }
                        }
                    }
                    TTopLevel::Enum(name, variants, _) => {
                        current_sig_reachable = false;
                        let mut enum_reachable = self.reachable_types.contains(name) || self.reachable_functions.contains(name);
                        if !enum_reachable {
                            for v in variants {
                                if self.reachable_functions.contains(&v.name) {
                                    enum_reachable = true;
                                    break;
                                }
                            }
                        }
                        
                        if enum_reachable {
                            self.reachable_types.insert(name.clone());
                            for v in variants {
                                self.reachable_functions.insert(v.name.clone());
                                match &v.data {
                                    crate::parser::ast::VariantData::Type(t) => self.visit_type(&t.0),
                                    crate::parser::ast::VariantData::FixedValue(e) => self.visit_ast_expr(e),
                                    crate::parser::ast::VariantData::Predicate(e) => self.visit_ast_expr(e),
                                    crate::parser::ast::VariantData::Unit => {}
                                }
                            }
                        }
                    }
                    TTopLevel::Execution(_) => {
                        current_sig_reachable = false;
                    }
                }
            }

            if self.reachable_functions.len() > initial_funcs || self.reachable_types.len() > initial_types {
                progress = true;
            }
        }

        let initial_nodes = tast.len();

        let mut optimized = Vec::new();
        let mut current_sig_reachable = false;

        for (decl, span) in tast {
            let keep = match &decl {
                TTopLevel::Signature(name, _, _, _) => {
                    current_sig_reachable = self.reachable_functions.contains(name);
                    current_sig_reachable
                }
                TTopLevel::LambdaDef(..) => {
                    current_sig_reachable
                }
                TTopLevel::ActionDef(name, _, _, _, _) => {
                    current_sig_reachable = false;
                    self.reachable_functions.contains(name)
                }
                TTopLevel::Execution(expr) => {
                    current_sig_reachable = false;
                    self.visit_expr(expr); 
                    true 
                }
                TTopLevel::Data(name, _, _) => {
                    current_sig_reachable = false;
                    self.reachable_types.contains(name) || self.reachable_functions.contains(name)
                }
                TTopLevel::Enum(name, variants, _) => {
                    current_sig_reachable = false;
                    self.reachable_types.contains(name) || self.reachable_functions.contains(name) || variants.iter().any(|v| self.reachable_functions.contains(&v.name))
                }
            };

            if keep {
                optimized.push((decl, span));
            } else if let TTopLevel::Signature(name, _, _, _) = &decl {
                log::debug!("Tree-Shaking: Removendo código morto/genérico: {}", name);
            } else if let TTopLevel::Data(name, _, _) = &decl {
                log::debug!("Tree-Shaking: Removendo Tipo inativo: {}", name);
            } else if let TTopLevel::Enum(name, _, _) = &decl {
                log::debug!("Tree-Shaking: Removendo Enum inativo: {}", name);
            }
        }

        log::debug!("Tree-Shaking concluído. (Iniciais: {}, Finais: {})", initial_nodes, optimized.len());
        optimized
    }

    fn visit_type(&mut self, ty: &TypeRef) {
        match ty {
            TypeRef::TypeVar(n) => {
                self.reachable_types.insert(n.clone());
            }
            TypeRef::Simple(n) => {
                self.reachable_types.insert(n.clone());
            }
            TypeRef::Generic(n, args) => {
                self.reachable_types.insert(n.clone());
                for (a, _) in args {
                    self.visit_type(a);
                }
            }
            TypeRef::Function(args, ret) => {
                for (a, _) in args {
                    self.visit_type(a);
                }
                self.visit_type(&ret.0);
            }
            TypeRef::Refined(base, preds) => {
                self.visit_type(&base.0);
                for (p, _) in preds {
                    self.visit_ast_expr(p);
                }
            }
            TypeRef::Variadic(inner) => {
                self.visit_type(&inner.0);
            }
            TypeRef::Const(expr) => {
                self.visit_ast_expr(expr);
            }
        }
    }

    fn visit_ast_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(n) => {
                self.reachable_functions.insert(n.clone());
                self.reachable_types.insert(n.clone());
            }
            Expr::ActionCall(n, args) => {
                self.reachable_functions.insert(n.clone());
                for (a, _) in args { self.visit_ast_expr(a); }
            }
            Expr::Try(inner) | Expr::ExplicitApp(inner) => self.visit_ast_expr(&inner.0),
            Expr::Pipe(l, r) => { self.visit_ast_expr(&l.0); self.visit_ast_expr(&r.0); }
            Expr::Tuple(es) | Expr::List(es) | Expr::Sequence(es) => {
                for (e, _) in es { self.visit_ast_expr(e); }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for (e, _) in row { self.visit_ast_expr(e); }
                }
            }
            Expr::Guard(branches, otherwise) => {
                for (c, b) in branches { self.reachable_functions.insert(c.clone()); self.reachable_types.insert(c.clone()); self.visit_ast_expr(&b.0); }
                self.visit_ast_expr(&otherwise.0);
            }
            Expr::Lambda(_, body, with) => {
                self.visit_ast_expr(&body.0);
                for (w, _) in with { self.visit_ast_expr(w); }
            }
            Expr::WithDecl(n, e) => {
                self.reachable_types.insert(n.clone());
                self.visit_ast_expr(&e.0);
            }
            _ => {}
        }
    }

    fn visit_stmt(&mut self, stmt: &Spanned<TStmt>) {
        match &stmt.0 {
            TStmt::Let(_, expr) => {
                self.visit_expr(expr);
            }
            TStmt::Var(_, expr) => self.visit_expr(expr),
            TStmt::Loop(body) => {
                for s in body {
                    self.visit_stmt(s);
                }
            }
            TStmt::For(_, iter, body) => {
                self.visit_expr(iter);
                for s in body {
                    self.visit_stmt(s);
                }
            }
            TStmt::Match(target, arms) => {
                self.visit_expr(target);
                for arm in arms {
                    for s in &arm.body {
                        self.visit_stmt(s);
                    }
                }
            }
            TStmt::Select(arms, timeout) => {
                for arm in arms {
                    self.visit_expr(&arm.operation);
                    for s in &arm.body {
                        self.visit_stmt(s);
                    }
                }
                if let Some((e, b)) = timeout {
                    self.visit_expr(e);
                    for s in b {
                        self.visit_stmt(s);
                    }
                }
            }
            TStmt::ActionAssign(t, v) => {
                self.visit_expr(t);
                self.visit_expr(v);
            }
            TStmt::Expr(expr) => self.visit_expr(expr),
            TStmt::Break | TStmt::Continue | TStmt::DropShared(_) => {}
        }
    }

    fn visit_expr(&mut self, expr: &Spanned<TExpr>) {
        let ty = match &expr.0 {
            TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Array(_, ty, _) | TExpr::Lambda(_, _, ty, _) | TExpr::Sequence(_, ty) | TExpr::Guard(_, _, ty) | TExpr::Try(_, ty) | TExpr::ChannelSend(_, _, ty) | TExpr::ChannelRecv(_, ty) | TExpr::ChannelRecvNonBlock(_, ty) => Some(ty.clone()),
            _ => None,
        };
        
        if let Some(t) = ty {
            self.visit_type(&t);
        }

        match &expr.0 {
            TExpr::Ident(name, _) => {
                self.reachable_functions.insert(name.clone());
                self.reachable_types.insert(name.clone());
            }
            TExpr::Call(callee, args, _) => {
                self.visit_expr(callee);
                for a in args {
                    self.visit_expr(a);
                }
            }
            TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) | TExpr::Sequence(exprs, _) => {
                for e in exprs {
                    self.visit_expr(e);
                }
            }
            TExpr::Array(rows, _, _) => {
                for row in rows {
                    for e in row {
                        self.visit_expr(e);
                    }
                }
            }
            TExpr::Lambda(_, body, _, _) => {
                self.visit_expr(body);
            }
            TExpr::Guard(branches, otherwise, _) => {
                for (cond, body) in branches {
                    self.visit_expr(cond);
                    self.visit_expr(body);
                }
                self.visit_expr(otherwise);
            }
            TExpr::Try(inner, _) => self.visit_expr(inner),
            TExpr::ChannelSend(target, val, _) => {
                self.visit_expr(target);
                self.visit_expr(val);
            }
            TExpr::ChannelRecv(target, _) | TExpr::ChannelRecvNonBlock(target, _) => {
                self.visit_expr(target);
            }
            TExpr::Literal(_) | TExpr::Hole => {}
        }
    }
}
