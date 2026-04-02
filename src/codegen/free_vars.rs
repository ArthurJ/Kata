use crate::type_checker::tast::{TExpr, TStmt};
use crate::parser::ast::Spanned;
use crate::type_checker::environment::TypeEnv;
use std::collections::HashSet;

pub fn get_free_vars(expr: &Spanned<TExpr>, bound_vars: &mut HashSet<String>, free_vars: &mut HashSet<String>, env: &TypeEnv) {
    let (e, _) = expr;
    match e {
        TExpr::Ident(name, _, _) => {
            if !bound_vars.contains(name) && env.lookup_first(name).is_none() {
                free_vars.insert(name.clone());
            }
        }
        TExpr::Call(callee, args, _, _) => {
            get_free_vars(callee, bound_vars, free_vars, env);
            for arg in args {
                get_free_vars(arg, bound_vars, free_vars, env);
            }
        }
        TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) | TExpr::Sequence(exprs, _) => {
            for e in exprs {
                get_free_vars(e, bound_vars, free_vars, env);
            }
        }
        TExpr::Lambda(params, body, _, _) => {
            let mut new_bound = bound_vars.clone();
            for (p, _) in params {
                if let crate::parser::ast::Pattern::Ident(name) = p {
                    new_bound.insert(name.clone());
                }
            }
            get_free_vars(body, &mut new_bound, free_vars, env);
        }
        TExpr::Guard(branches, otherwise, _) => {
            for (cond, body) in branches {
                get_free_vars(cond, bound_vars, free_vars, env);
                get_free_vars(body, bound_vars, free_vars, env);
            }
            get_free_vars(otherwise, bound_vars, free_vars, env);
        }
        TExpr::Try(inner, _, _) => get_free_vars(inner, bound_vars, free_vars, env),
        TExpr::ChannelSend(target, val, _) => {
            get_free_vars(target, bound_vars, free_vars, env);
            get_free_vars(val, bound_vars, free_vars, env);
        }
        TExpr::ChannelRecv(target, _, _) | TExpr::ChannelRecvNonBlock(target, _, _) => {
            get_free_vars(target, bound_vars, free_vars, env);
        }
        TExpr::ClosureAlloc(_, exprs, _, _) => {
            for ex in exprs {
                get_free_vars(ex, bound_vars, free_vars, env);
            }
        }
        TExpr::Literal(_) | TExpr::Hole | TExpr::EnvLoad(_, _) => {}
    }
}

fn extract_bound_vars(pat: &crate::parser::ast::Pattern, bound_vars: &mut HashSet<String>) {
    match pat {
        crate::parser::ast::Pattern::Ident(name) => {
            if name != "otherwise" {
                bound_vars.insert(name.clone());
            }
        }
        crate::parser::ast::Pattern::Tuple(pats) | crate::parser::ast::Pattern::List(pats) | crate::parser::ast::Pattern::Sequence(pats) => {
            for p in pats {
                extract_bound_vars(&p.0, bound_vars);
            }
        }
        _ => {}
    }
}

pub fn get_free_vars_stmt(stmt: &Spanned<TStmt>, bound_vars: &mut HashSet<String>, free_vars: &mut HashSet<String>, env: &TypeEnv) {
    let (s, _) = stmt;
    match s {
        TStmt::Let(pat, expr) => {
            get_free_vars(expr, bound_vars, free_vars, env);
            extract_bound_vars(&pat.0, bound_vars);
        }
        TStmt::Var(name, expr) => {
            get_free_vars(expr, bound_vars, free_vars, env);
            bound_vars.insert(name.clone());
        }
        TStmt::Loop(body) => {
            let mut new_bound = bound_vars.clone();
            for s in body {
                get_free_vars_stmt(s, &mut new_bound, free_vars, env);
            }
        }
        TStmt::For(name, iter, body) => {
            get_free_vars(iter, bound_vars, free_vars, env);
            let mut new_bound = bound_vars.clone();
            new_bound.insert(name.clone());
            for s in body {
                get_free_vars_stmt(s, &mut new_bound, free_vars, env);
            }
        }
        TStmt::Match(target, arms) => {
            get_free_vars(target, bound_vars, free_vars, env);
            for arm in arms {
                let mut new_bound = bound_vars.clone();
                extract_bound_vars(&arm.pattern.0, &mut new_bound);
                for s in &arm.body {
                    get_free_vars_stmt(s, &mut new_bound, free_vars, env);
                }
            }
        }
        TStmt::Expr(expr) => get_free_vars(expr, bound_vars, free_vars, env),
        TStmt::Select(arms, timeout) => {
            for arm in arms {
                get_free_vars(&arm.operation, bound_vars, free_vars, env);
                let mut new_bound = bound_vars.clone();
                if let Some(ref b) = arm.binding {
                    extract_bound_vars(&b.0, &mut new_bound);
                }
                for s in &arm.body {
                    get_free_vars_stmt(s, &mut new_bound, free_vars, env);
                }
            }
            if let Some((t_expr, t_stmts)) = timeout {
                get_free_vars(t_expr, bound_vars, free_vars, env);
                let mut new_bound = bound_vars.clone();
                for s in t_stmts {
                    get_free_vars_stmt(s, &mut new_bound, free_vars, env);
                }
            }
        }
        TStmt::ActionAssign(t, v) => {
            get_free_vars(t, bound_vars, free_vars, env);
            get_free_vars(v, bound_vars, free_vars, env);
        }
        TStmt::Break | TStmt::Continue | TStmt::DropShared(_) => {}
    }
}