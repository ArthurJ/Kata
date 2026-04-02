use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt, TMatchArm};
use crate::parser::ast::{Spanned, Pattern, TypeRef};
use crate::type_checker::environment::TypeEnv;
use crate::optimizer::error::OptimizerError;
use std::collections::{HashSet, HashMap};

pub struct LambdaLifting<'a> {
    pub env: &'a TypeEnv,
    pub new_top_levels: Vec<Spanned<TTopLevel>>,
    uid: usize,
}

impl<'a> LambdaLifting<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self { env, new_top_levels: Vec::new(), uid: 0 }
    }

    fn next_uid(&mut self) -> usize {
        self.uid += 1;
        self.uid
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Lambda Lifting...");
        let mut final_tast = Vec::new();
        
        for (decl, span) in tast {
            let folded = self.fold_toplevel(decl, errors);
            final_tast.push((folded, span));
        }

        final_tast.extend(self.new_top_levels.drain(..));
        
        final_tast
    }

    fn fold_toplevel(&mut self, decl: TTopLevel, errors: &mut Vec<OptimizerError>) -> TTopLevel {
        match decl {
            TTopLevel::LambdaDef(params, body, with, dirs) => {
                let folded_body = self.fold_expr_spanned(body, &mut HashSet::new(), errors);
                let folded_with = with.into_iter().map(|w| self.fold_expr_spanned(w, &mut HashSet::new(), errors)).collect();
                TTopLevel::LambdaDef(params, folded_body, folded_with, dirs)
            }
            TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                let folded_body = body.into_iter().map(|s| self.fold_stmt_spanned(s, &mut HashSet::new(), errors)).collect();
                TTopLevel::ActionDef(name, params, ret, folded_body, dirs)
            }
            TTopLevel::Execution(expr) => {
                TTopLevel::Execution(self.fold_expr_spanned(expr, &mut HashSet::new(), errors))
            }
            other => other,
        }
    }

    fn extract_bound_vars(pat: &Pattern, bound_vars: &mut HashSet<String>) {
        match pat {
            Pattern::Ident(name) => {
                if name != "otherwise" {
                    bound_vars.insert(name.clone());
                }
            }
            Pattern::Tuple(pats) | Pattern::List(pats) | Pattern::Sequence(pats) => {
                for p in pats {
                    Self::extract_bound_vars(&p.0, bound_vars);
                }
            }
            _ => {}
        }
    }

    fn fold_stmt_spanned(&mut self, stmt: Spanned<TStmt>, bound_vars: &mut HashSet<String>, errors: &mut Vec<OptimizerError>) -> Spanned<TStmt> {
        let (s, span) = stmt;
        let new_s = match s {
            TStmt::Let(pat, expr) => {
                let new_expr = self.fold_expr_spanned(expr, bound_vars, errors);
                Self::extract_bound_vars(&pat.0, bound_vars);
                TStmt::Let(pat, new_expr)
            }
            TStmt::Var(name, expr) => {
                let new_expr = self.fold_expr_spanned(expr, bound_vars, errors);
                bound_vars.insert(name.clone());
                TStmt::Var(name, new_expr)
            }
            TStmt::Loop(body) => {
                let mut b_vars = bound_vars.clone();
                TStmt::Loop(body.into_iter().map(|s| self.fold_stmt_spanned(s, &mut b_vars, errors)).collect())
            }
            TStmt::For(name, iter, body) => {
                let new_iter = self.fold_expr_spanned(iter, bound_vars, errors);
                let mut b_vars = bound_vars.clone();
                b_vars.insert(name.clone());
                TStmt::For(name, new_iter, body.into_iter().map(|s| self.fold_stmt_spanned(s, &mut b_vars, errors)).collect())
            }
            TStmt::Match(target, arms) => {
                let new_target = self.fold_expr_spanned(target, bound_vars, errors);
                let new_arms = arms.into_iter().map(|arm| {
                    let mut b_vars = bound_vars.clone();
                    Self::extract_bound_vars(&arm.pattern.0, &mut b_vars);
                    TMatchArm {
                        pattern: arm.pattern,
                        body: arm.body.into_iter().map(|s| self.fold_stmt_spanned(s, &mut b_vars, errors)).collect(),
                    }
                }).collect();
                TStmt::Match(new_target, new_arms)
            }
            TStmt::Expr(expr) => TStmt::Expr(self.fold_expr_spanned(expr, bound_vars, errors)),
            other => other,
        };
        (new_s, span)
    }

    fn fold_expr_spanned(&mut self, expr: Spanned<TExpr>, bound_vars: &mut HashSet<String>, errors: &mut Vec<OptimizerError>) -> Spanned<TExpr> {
        let (e, span) = expr;
        match e {
            TExpr::Lambda(params, body, ty, alloc_mode) => {
                let mut local_bound = bound_vars.clone();
                for (p, _) in &params {
                    if let Pattern::Ident(name) = p {
                        local_bound.insert(name.clone());
                    }
                }

                let mut free_vars = HashSet::new();
                crate::codegen::free_vars::get_free_vars(&body, &mut local_bound, &mut free_vars, self.env);

                let mut free_vars_list: Vec<String> = free_vars.into_iter().collect();
                free_vars_list.sort();
                
                let mut fv_map = HashMap::new();
                for (idx, fv) in free_vars_list.iter().enumerate() {
                    fv_map.insert(fv.clone(), idx);
                }

                let lifted_name = format!("__lambda_{}", self.next_uid());
                
                let mut lifted_params = Vec::new();
                lifted_params.push((Pattern::Ident("__env_ptr".to_string()), 0..0));
                lifted_params.extend(params.clone());

                let mut lifted_sig_params = Vec::new();
                lifted_sig_params.push((TypeRef::Simple("EnvPtr".to_string()), 0..0));
                if let TypeRef::Function(args_ty, _) = &ty {
                    lifted_sig_params.extend(args_ty.clone());
                }

                let ret_ty = if let TypeRef::Function(_, ret) = &ty { ret.0.clone() } else { TypeRef::Simple("Unknown".to_string()) };

                let sig = TTopLevel::Signature(lifted_name.clone(), lifted_sig_params, (ret_ty.clone(), 0..0), Vec::new());
                self.new_top_levels.push((sig, span.clone()));

                // Fold inner expressions first
                let folded_body = self.fold_expr_spanned(*body, &mut local_bound, errors);
                
                // Rewrite Idents that are free variables to EnvLoad
                let rewritten_body = self.rewrite_fv_idents(folded_body, &fv_map);

                let lifted_def = TTopLevel::LambdaDef(lifted_params, rewritten_body, Vec::new(), Vec::new());
                self.new_top_levels.push((lifted_def, span.clone()));

                let mut captured_exprs = Vec::new();
                for fv in free_vars_list {
                    // Capture them from current scope
                    captured_exprs.push((TExpr::Ident(fv, TypeRef::Simple("Unknown".to_string()), crate::type_checker::tast::AllocMode::Local), span.clone()));
                }

                (TExpr::ClosureAlloc(lifted_name, captured_exprs, alloc_mode, ty), span)
            }
            TExpr::Call(callee, args, ty, _, _, _) => {
                let folded_callee = Box::new(self.fold_expr_spanned(*callee, bound_vars, errors));
                let folded_args = args.into_iter().map(|a| self.fold_expr_spanned(a, bound_vars, errors)).collect();
                (TExpr::Call(folded_callee, folded_args, ty), span)
            }
            TExpr::Tuple(exprs, ty, alloc) => (TExpr::Tuple(exprs.into_iter().map(|e| self.fold_expr_spanned(e, bound_vars, errors)).collect(), ty, alloc), span),
            TExpr::List(exprs, ty, alloc) => (TExpr::List(exprs.into_iter().map(|e| self.fold_expr_spanned(e, bound_vars, errors)).collect(), ty, alloc), span),
            TExpr::Sequence(exprs, ty) => (TExpr::Sequence(exprs.into_iter().map(|e| self.fold_expr_spanned(e, bound_vars, errors)).collect(), ty), span),
            TExpr::Guard(branches, otherwise, ty) => {
                let folded_branches = branches.into_iter().map(|(c, b)| (self.fold_expr_spanned(c, bound_vars, errors), self.fold_expr_spanned(b, bound_vars, errors))).collect();
                (TExpr::Guard(folded_branches, Box::new(self.fold_expr_spanned(*otherwise, bound_vars, errors)), ty), span)
            }
            TExpr::Try(inner, ty, _, _, _) => (TExpr::Try(Box::new(self.fold_expr_spanned(*inner, bound_vars, errors, _)), ty), span),
            TExpr::ChannelSend(t, v, ty) => (TExpr::ChannelSend(Box::new(self.fold_expr_spanned(*t, bound_vars, errors)), Box::new(self.fold_expr_spanned(*v, bound_vars, errors)), ty), span),
            TExpr::ChannelRecv(t, ty, _, _, _) => (TExpr::ChannelRecv(Box::new(self.fold_expr_spanned(*t, bound_vars, errors, _)), ty), span),
            TExpr::ChannelRecvNonBlock(t, ty, _, _, _) => (TExpr::ChannelRecvNonBlock(Box::new(self.fold_expr_spanned(*t, bound_vars, errors, _)), ty), span),
            other => (other, span),
        }
    }

    fn rewrite_fv_idents(&self, expr: Spanned<TExpr>, fv_map: &HashMap<String, usize>) -> Spanned<TExpr> {
        let (e, span) = expr;
        let new_e = match e {
            TExpr::Ident(name, ty, _, _) => {
                if let Some(&idx) = fv_map.get(&name) {
                    TExpr::EnvLoad(idx, ty)
                } else {
                    TExpr::Ident(name, ty, crate::type_checker::tast::AllocMode::Local)
                }
            }
            TExpr::Call(callee, args, ty, _, _) => {
                TExpr::Call(Box::new(self.rewrite_fv_idents(*callee, fv_map)), args.into_iter().map(|a| self.rewrite_fv_idents(a, fv_map)).collect(), ty)
            }
            TExpr::Tuple(exprs, ty, alloc) => TExpr::Tuple(exprs.into_iter().map(|e| self.rewrite_fv_idents(e, fv_map)).collect(), ty, alloc),
            TExpr::List(exprs, ty, alloc) => TExpr::List(exprs.into_iter().map(|e| self.rewrite_fv_idents(e, fv_map)).collect(), ty, alloc),
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(exprs.into_iter().map(|e| self.rewrite_fv_idents(e, fv_map)).collect(), ty),
            TExpr::Guard(branches, otherwise, ty) => {
                TExpr::Guard(branches.into_iter().map(|(c, b)| (self.rewrite_fv_idents(c, fv_map), self.rewrite_fv_idents(b, fv_map))).collect(), Box::new(self.rewrite_fv_idents(*otherwise, fv_map)), ty)
            }
            TExpr::Try(inner, ty, _, _) => TExpr::Try(Box::new(self.rewrite_fv_idents(*inner, fv_map)), ty),
            TExpr::ChannelSend(t, v, ty) => TExpr::ChannelSend(Box::new(self.rewrite_fv_idents(*t, fv_map)), Box::new(self.rewrite_fv_idents(*v, fv_map)), ty),
            TExpr::ChannelRecv(t, ty, _, _) => TExpr::ChannelRecv(Box::new(self.rewrite_fv_idents(*t, fv_map)), ty),
            TExpr::ChannelRecvNonBlock(t, ty, _, _) => TExpr::ChannelRecvNonBlock(Box::new(self.rewrite_fv_idents(*t, fv_map)), ty),
            // Lambda already handles its own free variables because it was folded before we rewrote it, so it's a ClosureAlloc now!
            TExpr::ClosureAlloc(name, exprs, alloc, ty) => {
                TExpr::ClosureAlloc(name, exprs.into_iter().map(|e| self.rewrite_fv_idents(e, fv_map)).collect(), alloc, ty)
            }
            other => other,
        };
        (new_e, span)
    }
}