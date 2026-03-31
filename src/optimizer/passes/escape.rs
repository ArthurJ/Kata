use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt, TMatchArm, AllocMode};
use crate::parser::ast::{Spanned, Pattern};
use crate::optimizer::error::OptimizerError;
use std::collections::{HashMap, HashSet};

pub struct EscapeAnalysis;

impl EscapeAnalysis {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, _errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Escape Analysis...");
        tast.into_iter()
            .map(|(decl, span)| (self.analyze_toplevel(decl), span))
            .collect()
    }

    fn analyze_toplevel(&mut self, decl: TTopLevel) -> TTopLevel {
        match decl {
            TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                // Pass 1: Find escaping variables and dependencies
                let mut escaping_vars = HashSet::new();
                let mut deps: HashMap<String, Vec<String>> = HashMap::new();

                for stmt in &body {
                    self.collect_dependencies(stmt, &mut deps, &mut escaping_vars);
                }

                // Propagate escapes
                let mut changed = true;
                while changed {
                    changed = false;
                    let current_escaping: Vec<String> = escaping_vars.iter().cloned().collect();
                    for var in current_escaping {
                        if let Some(dependencies) = deps.get(&var) {
                            for dep in dependencies {
                                if escaping_vars.insert(dep.clone()) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }

                // Pass 2: Rewrite AST
                let new_body = body.into_iter()
                    .map(|stmt| self.rewrite_stmt(stmt, &escaping_vars, false))
                    .collect();

                TTopLevel::ActionDef(name, params, ret, new_body, dirs)
            }
            TTopLevel::LambdaDef(params, body, with, dirs) => {
                // Pure functions don't have ChannelSends natively, but they can be passed an allocator context in advanced implementations.
                // For V1, we just recursively rewrite without forcing shared mode.
                let new_body = self.rewrite_expr(body, &HashSet::new(), false);
                let new_with = with.into_iter().map(|w| self.rewrite_expr(w, &HashSet::new(), false)).collect();
                TTopLevel::LambdaDef(params, new_body, new_with, dirs)
            }
            other => other,
        }
    }

    fn collect_dependencies(&self, stmt: &Spanned<TStmt>, deps: &mut HashMap<String, Vec<String>>, escaping: &mut HashSet<String>) {
        let (s, _) = stmt;
        match s {
            TStmt::Let(pat, expr) => {
                let mut rhs_vars = HashSet::new();
                self.extract_idents(expr, &mut rhs_vars);
                if let Pattern::Ident(name) = &pat.0 {
                    deps.insert(name.clone(), rhs_vars.into_iter().collect());
                }
                self.extract_channel_sends(expr, escaping);
            }
            TStmt::Var(name, expr) => {
                let mut rhs_vars = HashSet::new();
                self.extract_idents(expr, &mut rhs_vars);
                deps.insert(name.clone(), rhs_vars.into_iter().collect());
                self.extract_channel_sends(expr, escaping);
            }
            TStmt::Loop(body) => {
                for s in body { self.collect_dependencies(s, deps, escaping); }
            }
            TStmt::For(name, iter, body) => {
                let mut iter_vars = HashSet::new();
                self.extract_idents(iter, &mut iter_vars);
                deps.insert(name.clone(), iter_vars.into_iter().collect());

                self.extract_channel_sends(iter, escaping);
                for s in body { self.collect_dependencies(s, deps, escaping); }
            }
            TStmt::Match(target, arms) => {
                self.extract_channel_sends(target, escaping);
                for arm in arms {
                    for s in &arm.body {
                        self.collect_dependencies(s, deps, escaping);
                    }
                }
            }
            TStmt::Expr(expr) => {
                self.extract_channel_sends(expr, escaping);
            }
            TStmt::Break | TStmt::Continue => {}
        }
    }

    fn extract_idents(&self, expr: &Spanned<TExpr>, vars: &mut HashSet<String>) {
        let (e, _) = expr;
        match e {
            TExpr::Ident(name, _) => { vars.insert(name.clone()); }
            TExpr::Call(callee, args, _) => {
                self.extract_idents(callee, vars);
                for arg in args { self.extract_idents(arg, vars); }
            }
            TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) | TExpr::Sequence(exprs, _) => {
                for ex in exprs { self.extract_idents(ex, vars); }
            }
            TExpr::Lambda(_, body, _, _) => self.extract_idents(body, vars),
            TExpr::Guard(branches, otherwise, _) => {
                for (cond, body) in branches {
                    self.extract_idents(cond, vars);
                    self.extract_idents(body, vars);
                }
                self.extract_idents(otherwise, vars);
            }
            TExpr::Try(inner, _) => self.extract_idents(inner, vars),
            TExpr::ChannelSend(target, val, _) => {
                self.extract_idents(target, vars);
                self.extract_idents(val, vars);
            }
            TExpr::ChannelRecv(target, _) | TExpr::ChannelRecvNonBlock(target, _) => {
                self.extract_idents(target, vars);
            }
            TExpr::Literal(_) | TExpr::Hole => {}
        }
    }

    fn extract_channel_sends(&self, expr: &Spanned<TExpr>, escaping: &mut HashSet<String>) {
        let (e, _) = expr;
        match e {
            TExpr::ChannelSend(_, val, _) => {
                self.extract_idents(val, escaping);
            }
            TExpr::Call(callee, args, _) => {
                self.extract_channel_sends(callee, escaping);
                for arg in args { self.extract_channel_sends(arg, escaping); }
            }
            TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) | TExpr::Sequence(exprs, _) => {
                for ex in exprs { self.extract_channel_sends(ex, escaping); }
            }
            TExpr::Lambda(_, body, _, _) => self.extract_channel_sends(body, escaping),
            TExpr::Guard(branches, otherwise, _) => {
                for (cond, body) in branches {
                    self.extract_channel_sends(cond, escaping);
                    self.extract_channel_sends(body, escaping);
                }
                self.extract_channel_sends(otherwise, escaping);
            }
            TExpr::Try(inner, _) => self.extract_channel_sends(inner, escaping),
            TExpr::ChannelRecv(target, _) | TExpr::ChannelRecvNonBlock(target, _) => {
                self.extract_channel_sends(target, escaping);
            }
            _ => {}
        }
    }

    fn rewrite_stmt(&self, stmt: Spanned<TStmt>, escaping: &HashSet<String>, force_shared: bool) -> Spanned<TStmt> {
        let (s, span) = stmt;
        let new_s = match s {
            TStmt::Let(pat, expr) => {
                let mut is_shared = force_shared;
                if let Pattern::Ident(name) = &pat.0 {
                    if escaping.contains(name) { is_shared = true; }
                }
                TStmt::Let(pat, self.rewrite_expr(expr, escaping, is_shared))
            }
            TStmt::Var(name, expr) => {
                let mut is_shared = force_shared;
                if escaping.contains(&name) { is_shared = true; }
                TStmt::Var(name, self.rewrite_expr(expr, escaping, is_shared))
            }
            TStmt::Loop(body) => TStmt::Loop(body.into_iter().map(|s| self.rewrite_stmt(s, escaping, force_shared)).collect()),
            TStmt::For(name, iter, body) => {
                let mut is_shared = force_shared;
                if escaping.contains(&name) { is_shared = true; }
                TStmt::For(
                    name, 
                    self.rewrite_expr(iter, escaping, is_shared),
                    body.into_iter().map(|s| self.rewrite_stmt(s, escaping, force_shared)).collect()
                )
            }
            TStmt::Match(target, arms) => {
                TStmt::Match(
                    self.rewrite_expr(target, escaping, force_shared),
                    arms.into_iter().map(|arm| TMatchArm {
                        pattern: arm.pattern,
                        body: arm.body.into_iter().map(|s| self.rewrite_stmt(s, escaping, force_shared)).collect(),
                    }).collect()
                )
            }
            TStmt::Expr(expr) => TStmt::Expr(self.rewrite_expr(expr, escaping, force_shared)),
            other => other,
        };
        (new_s, span)
    }

    fn rewrite_expr(&self, expr: Spanned<TExpr>, escaping: &HashSet<String>, force_shared: bool) -> Spanned<TExpr> {
        let (e, span) = expr;
        let new_e = match e {
            TExpr::Tuple(exprs, ty, alloc) => {
                let new_alloc = if force_shared { AllocMode::Shared } else { alloc };
                TExpr::Tuple(exprs.into_iter().map(|e| self.rewrite_expr(e, escaping, force_shared)).collect(), ty, new_alloc)
            }
            TExpr::List(exprs, ty, alloc) => {
                let new_alloc = if force_shared { AllocMode::Shared } else { alloc };
                TExpr::List(exprs.into_iter().map(|e| self.rewrite_expr(e, escaping, force_shared)).collect(), ty, new_alloc)
            }
            TExpr::Call(callee, args, ty) => {
                TExpr::Call(
                    Box::new(self.rewrite_expr(*callee, escaping, false)),
                    args.into_iter().map(|a| self.rewrite_expr(a, escaping, false)).collect(),
                    ty
                )
            }
            TExpr::ChannelSend(target, val, ty) => {
                TExpr::ChannelSend(
                    Box::new(self.rewrite_expr(*target, escaping, false)),
                    Box::new(self.rewrite_expr(*val, escaping, true)), // The value sent escapes!
                    ty
                )
            }
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(exprs.into_iter().map(|e| self.rewrite_expr(e, escaping, force_shared)).collect(), ty),
            TExpr::Lambda(params, body, ty, alloc) => {
                let new_alloc = if force_shared { AllocMode::Shared } else { alloc };
                TExpr::Lambda(params, Box::new(self.rewrite_expr(*body, escaping, false)), ty, new_alloc)
            },
            TExpr::Guard(branches, otherwise, ty) => {
                let new_branches = branches.into_iter()
                    .map(|(c, b)| (self.rewrite_expr(c, escaping, false), self.rewrite_expr(b, escaping, force_shared)))
                    .collect();
                TExpr::Guard(new_branches, Box::new(self.rewrite_expr(*otherwise, escaping, force_shared)), ty)
            }
            TExpr::Try(inner, ty) => TExpr::Try(Box::new(self.rewrite_expr(*inner, escaping, force_shared)), ty),
            TExpr::ChannelRecv(target, ty) => TExpr::ChannelRecv(Box::new(self.rewrite_expr(*target, escaping, false)), ty),
            TExpr::ChannelRecvNonBlock(target, ty) => TExpr::ChannelRecvNonBlock(Box::new(self.rewrite_expr(*target, escaping, false)), ty),
            other => other,
        };
        (new_e, span)
    }
}
