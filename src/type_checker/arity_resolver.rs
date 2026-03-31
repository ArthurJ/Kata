use crate::parser::ast::{Expr, Spanned, TypeRef};
use crate::type_checker::environment::TypeEnv;
use crate::type_checker::tast::{TExpr, TLiteral};

pub struct ArityResolver<'a> {
    pub env: &'a TypeEnv,
    pub local_vars: std::cell::RefCell<std::collections::HashMap<String, TypeRef>>,
    pub constraints: std::cell::RefCell<std::collections::HashMap<String, String>>,
    pub errors: std::cell::RefCell<Vec<(String, crate::parser::ast::Span)>>,
    pub pure_context: std::cell::Cell<bool>,
    pub current_action: std::cell::RefCell<Option<String>>,
}

impl<'a> ArityResolver<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self { 
            env, 
            local_vars: std::cell::RefCell::new(std::collections::HashMap::new()), 
            constraints: std::cell::RefCell::new(std::collections::HashMap::new()),
            errors: std::cell::RefCell::new(Vec::new()),
            pure_context: std::cell::Cell::new(true),
            current_action: std::cell::RefCell::new(None),
        }
    }

    pub fn declare_local(&self, name: String, ty: TypeRef) {
        self.local_vars.borrow_mut().insert(name, ty);
    }

    pub fn get_expr_type(expr: &TExpr) -> TypeRef {
        match expr {
            TExpr::Literal(TLiteral::Int(_)) => TypeRef::Simple("Int".to_string()),
            TExpr::Literal(TLiteral::Float(_)) => TypeRef::Simple("Float".to_string()),
            TExpr::Literal(TLiteral::String(_)) => TypeRef::Simple("Text".to_string()),
            TExpr::Literal(TLiteral::Bool(_)) => TypeRef::Simple("Bool".to_string()),
            TExpr::Literal(TLiteral::Unit) => TypeRef::Simple("()".to_string()),
            TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty, _) | TExpr::Sequence(_, ty) | TExpr::Guard(_, _, ty) | TExpr::Try(_, ty) | TExpr::ChannelSend(_, _, ty) | TExpr::ChannelRecv(_, ty) | TExpr::ChannelRecvNonBlock(_, ty) => ty.clone(),
            TExpr::Hole => TypeRef::Simple("Unknown".to_string()),
        }
    }

    pub fn types_compatible(&self, arg: &TypeRef, param: &TypeRef) -> bool {
        match (arg, param) {
            (TypeRef::Generic(n, args), TypeRef::Simple(p)) if n == "Tuple" && args.is_empty() && p == "()" => true,
            (TypeRef::Simple(a), TypeRef::Generic(n, args)) if a == "()" && n == "Tuple" && args.is_empty() => true,
            (TypeRef::Simple(a), TypeRef::Simple(p)) => {
                if a == p || a == "Unknown" || p == "Unknown" {
                    true
                } else if p.len() == 1 && p.chars().next().unwrap().is_uppercase() {
                    // param is an unbound generic parameter of the called function (e.g. A in map)
                    true
                } else if a.len() == 1 && a.chars().next().unwrap().is_uppercase() {
                    // arg is a generic type variable from the current function's constraints
                    if let Some(constraint) = self.constraints.borrow().get(a) {
                        self.env.implements(constraint, p)
                    } else {
                        false
                    }
                } else if let Some(refined_p) = self.env.lookup_refined(p) {
                    self.types_compatible(arg, &refined_p.base)
                } else if let Some(refined_a) = self.env.lookup_refined(a) {
                    // Refined matches its base
                    self.types_compatible(&refined_a.base, param)
                } else {
                    self.env.implements(a, p)
                }
            },
            (TypeRef::Refined(base, _), p) => {
                self.types_compatible(&base.0, p)
            },
            (arg, TypeRef::Refined(base, _)) => {
                self.types_compatible(arg, &base.0)
            },
            (TypeRef::Generic(a_name, a_args), TypeRef::Generic(p_name, p_args)) => {
                if a_name != p_name {
                    return false;
                }
                
                // Variadic check specifically for Tuples
                if p_name == "Tuple" && p_args.len() > 0 {
                    let mut a_idx = 0;
                    for p_arg in p_args {
                        match &p_arg.0 {
                            TypeRef::Variadic(inner_p) => {
                                // Consume remaining args as long as they match inner_p
                                while a_idx < a_args.len() {
                                    if !self.types_compatible(&a_args[a_idx].0, &inner_p.0) {
                                        return false;
                                    }
                                    a_idx += 1;
                                }
                            }
                            other_p => {
                                if a_idx >= a_args.len() || !self.types_compatible(&a_args[a_idx].0, other_p) {
                                    return false;
                                }
                                a_idx += 1;
                            }
                        }
                    }
                    return a_idx == a_args.len();
                }

                if a_args.len() != p_args.len() {
                    return false;
                }
                for (aa, pp) in a_args.iter().zip(p_args.iter()) {
                    if !self.types_compatible(&aa.0, &pp.0) {
                        return false;
                    }
                }
                true
            }
            (TypeRef::Function(a_args, a_ret), TypeRef::Function(p_args, p_ret)) => {
                if a_args.len() != p_args.len() || !self.types_compatible(&a_ret.0, &p_ret.0) {
                    return false;
                }
                for (aa, pp) in a_args.iter().zip(p_args.iter()) {
                    if !self.types_compatible(&aa.0, &pp.0) {
                        return false;
                    }
                }
                true
            }
            (_, TypeRef::Simple(p)) if p.len() == 1 && p.chars().next().unwrap().is_uppercase() => true, 
            _ => false,
        }
    }

    fn check_recursion(&self, name: &str, span: &crate::parser::ast::Span) {
        if let Some(curr) = &*self.current_action.borrow() {
            if curr == name {
                self.errors.borrow_mut().push((format!(
                    "Erro Semantico: Recursao proibida na Action `{}`. Use loops (for/loop) para iteracoes impuras.",
                    name
                ), span.clone()));
            }
        }
    }

    fn resolve_next_expr(&self, iter: &mut std::iter::Peekable<std::slice::Iter<'_, Spanned<Expr>>>) -> Option<Spanned<TExpr>> {
        let (expr, span) = iter.next()?;
        match expr {
            Expr::Ident(_) | Expr::ChannelSend | Expr::ChannelRecv | Expr::ChannelRecvNonBlock => {
                let (name, mut arity, is_action) = match expr {
                    Expr::Ident(n) => {
                        if let Some(infos) = self.env.lookup_all(n) {
                            (n.clone(), infos[0].arity, infos[0].is_action)
                        } else {
                            (n.clone(), 0, false)
                        }
                    }
                    Expr::ChannelSend => ("!>".to_string(), 2, true),
                    Expr::ChannelRecv => ("<!".to_string(), 1, true),
                    Expr::ChannelRecvNonBlock => ("<!?".to_string(), 1, true),
                    _ => unreachable!(),
                };

                if let Some(ty) = self.local_vars.borrow().get(&name).cloned() {
                    return Some((TExpr::Ident(name.clone(), ty), span.clone()));
                } else {
                    let mut args = Vec::new();
                    let mut best_match = None;
                    let mut best_score = -1;
                    let mut args_were_swapped = false;

                    if let Some(infos) = if matches!(expr, Expr::ChannelSend | Expr::ChannelRecv | Expr::ChannelRecvNonBlock) { None } else { self.env.lookup_all(&name) } {
                        let first_info = &infos[0];
                        arity = first_info.arity;
                        let limit = arity;
                        
                        let mut count = 0;
                        while count < limit {
                            if let Some(next_arg) = self.resolve_next_expr(iter) {
                                args.push(next_arg);
                                count += 1;
                            } else {
                                break;
                            }
                        }

                        // Múltiplo Despacho
                        let mut is_ambiguous = false;
                        for info in infos {
                            if let TypeRef::Function(params, _) = &info.type_info {
                                if params.len() == args.len() {
                                    let mut all_match = true;
                                    let mut score = 0;
                                    for (i, arg) in args.iter().enumerate() {
                                        let arg_type = Self::get_expr_type(&arg.0);
                                        let param_type = &params[i].0;
                                        if !self.types_compatible(&arg_type, param_type) { all_match = false; break; }
                                        score += match (&arg_type, param_type) {
                                            (TypeRef::Simple(a), TypeRef::Simple(p)) if a == p => 10,
                                            (_, TypeRef::Simple(p)) if p.len() == 1 => 1,
                                            _ => 5,
                                        };
                                    }
                                    if all_match {
                                        if score > best_score { best_score = score; best_match = Some(info); is_ambiguous = false; args_were_swapped = false; }
                                        else if score == best_score && best_match != Some(info) { is_ambiguous = true; }
                                    }

                                    if info.is_commutative && args.len() == 2 {
                                        let mut all_match_swapped = true;
                                        let mut score_swapped = 0;
                                        let swapped_args = [&args[1], &args[0]];
                                        for (i, arg) in swapped_args.iter().enumerate() {
                                            let arg_type = Self::get_expr_type(&arg.0);
                                            let param_type = &params[i].0;
                                            if !self.types_compatible(&arg_type, param_type) { all_match_swapped = false; break; }
                                            score_swapped += match (&arg_type, param_type) {
                                                (TypeRef::Simple(a), TypeRef::Simple(p)) if a == p => 10,
                                                (_, TypeRef::Simple(p)) if p.len() == 1 => 1,
                                                _ => 5,
                                            };
                                        }
                                        if all_match_swapped {
                                            if score_swapped > best_score { best_score = score_swapped; best_match = Some(info); is_ambiguous = false; args_were_swapped = true; }
                                            else if score_swapped == best_score && best_match != Some(info) { is_ambiguous = true; }
                                        }
                                    }
                                }
                            }
                        }

                        if is_ambiguous {
                            self.errors.borrow_mut().push((format!("Erro Semantico (Ambiguidade): `{}`", name), span.clone()));
                        } else if best_match.is_none() && !first_info.is_action {
                            self.errors.borrow_mut().push((format!("Erro de Tipo: `{}`", name), span.clone()));
                        }
                    } else {
                        // Operadores de canal ou ident desconhecido
                        let limit = arity;
                        let mut count = 0;
                        while count < limit {
                            if let Some(next_arg) = self.resolve_next_expr(iter) {
                                args.push(next_arg);
                                count += 1;
                            } else {
                                break;
                            }
                        }
                    }

                    let final_info = best_match;
                    let mut final_args = args;
                    if args_were_swapped { final_args.swap(0, 1); }

                    if self.pure_context.get() && is_action {
                        self.errors.borrow_mut().push((format!("Erro de Pureza: `{}`", name), span.clone()));
                    }
                    self.check_recursion(&name, span);

                    let mut return_type = if let Some(info) = final_info {
                        match &info.type_info {
                            TypeRef::Function(_, ret) => ret.0.clone(),
                            other => other.clone(),
                        }
                    } else {
                        match expr {
                            Expr::ChannelSend => TypeRef::Simple("()".into()),
                            Expr::ChannelRecv | Expr::ChannelRecvNonBlock => TypeRef::Simple("Unknown".into()),
                            _ => TypeRef::Simple("Unknown".into()),
                        }
                    };

                    // Herança de Predicados
                    let mut inherited_predicates = Vec::new();
                    let max_possible_score = final_args.len() as i32 * 10;
                    if best_score >= 0 && best_score < max_possible_score {
                        for arg in &final_args {
                            match Self::get_expr_type(&arg.0) {
                                TypeRef::Simple(n) => if let Some(r) = self.env.lookup_refined(&n) { inherited_predicates.extend(r.predicates.iter().map(|p| (p.clone(), 0..0))); }
                                TypeRef::Refined(_, preds) => inherited_predicates.extend(preds.clone()),
                                _ => {}
                            }
                        }
                    }
                    if !inherited_predicates.is_empty() {
                        let refined = TypeRef::Refined(Box::new((return_type.clone(), 0..0)), inherited_predicates);
                        return_type = TypeRef::Generic("Result".into(), vec![(refined, 0..0), (TypeRef::Simple("Text".into()), 0..0)]);
                    }

                    match expr {
                        Expr::ChannelSend if final_args.len() == 2 => {
                            let target = Box::new(final_args.remove(0));
                            let val = Box::new(final_args.remove(0));
                            return Some((TExpr::ChannelSend(target, val, return_type), span.clone()));
                        }
                        Expr::ChannelRecv if final_args.len() == 1 => {
                            let target = Box::new(final_args.remove(0));
                            return Some((TExpr::ChannelRecv(target, return_type), span.clone()));
                        }
                        Expr::ChannelRecvNonBlock if final_args.len() == 1 => {
                            let target = Box::new(final_args.remove(0));
                            return Some((TExpr::ChannelRecvNonBlock(target, return_type), span.clone()));
                        }
                        _ => {
                            let has_holes = final_args.iter().any(|(a, _)| matches!(a, TExpr::Hole));
                            if has_holes {
                                // Currying via Synthetic Lambda Injection
                                let mut lambda_params = Vec::new();
                                let mut lambda_args = Vec::new();
                                let mut hole_counter = 0;
                                let mut call_args = Vec::new();
                                
                                for (arg_expr, arg_span) in final_args {
                                    if matches!(arg_expr, TExpr::Hole) {
                                        let hole_name = format!("__hole_{}", hole_counter);
                                        hole_counter += 1;
                                        
                                        let hole_ty = TypeRef::Simple("Unknown".into());
                                        lambda_params.push((crate::parser::ast::Pattern::Ident(hole_name.clone()), arg_span.clone()));
                                        lambda_args.push((hole_ty.clone(), 0..0));
                                        
                                        call_args.push((TExpr::Ident(hole_name, hole_ty), arg_span.clone()));
                                    } else {
                                        call_args.push((arg_expr, arg_span));
                                    }
                                }
                                
                                let callee = Box::new((TExpr::Ident(name, final_info.map(|i| i.type_info.clone()).unwrap_or(TypeRef::Simple("Unknown".into()))), span.clone()));
                                let inner_call = (TExpr::Call(callee, call_args, return_type.clone()), span.clone());
                                
                                let lambda_type = TypeRef::Function(lambda_args, Box::new((return_type, span.clone())));
                                
                                return Some((TExpr::Lambda(lambda_params, Box::new(inner_call), lambda_type, crate::type_checker::tast::AllocMode::Local), span.clone()));
                            } else {
                                let callee = Box::new((TExpr::Ident(name, final_info.map(|i| i.type_info.clone()).unwrap_or(TypeRef::Simple("Unknown".into()))), span.clone()));
                                return Some((TExpr::Call(callee, final_args, return_type), span.clone()));
                            }
                        }
                    }
                }
            }
            _ => {
                return Some(self.resolve_expr(&(expr.clone(), span.clone())));
            }
        }
    }

    pub fn resolve_sequence(&self, exprs: &[Spanned<Expr>]) -> Vec<Spanned<TExpr>> {
        let mut result = Vec::new();
        let mut iter = exprs.iter().peekable();
        while let Some(res) = self.resolve_next_expr(&mut iter) {
            result.push(res);
        }
        result
    }


    pub fn resolve_expr(&self, spanned: &Spanned<Expr>) -> Spanned<TExpr> {
        let (expr, span) = spanned;
        match expr {
            Expr::Int(v) => (TExpr::Literal(TLiteral::Int(v.parse().unwrap_or(0))), span.clone()),
            Expr::Float(v) => (TExpr::Literal(TLiteral::Float(v.parse().unwrap_or(0.0))), span.clone()),
            Expr::String(v) => (TExpr::Literal(TLiteral::String(v.clone())), span.clone()),
            Expr::Sequence(seq) => {
                let resolved = self.resolve_sequence(seq);
                let ty = if let Some(last) = resolved.last() { Self::get_expr_type(&last.0) } else { TypeRef::Simple("()".to_string()) };
                (TExpr::Sequence(resolved, ty), span.clone())
            }
            Expr::Guard(branches, otherwise) => {
                let mut resolved_branches = Vec::new();
                for (cond_name, body) in branches {
                    let cond_ty = self.local_vars.borrow().get(cond_name).cloned().unwrap_or(TypeRef::Simple("Unknown".into()));
                    let cond_expr = (TExpr::Ident(cond_name.clone(), cond_ty), span.clone());
                    let resolved_body = self.resolve_expr(body);
                    resolved_branches.push((cond_expr, resolved_body));
                }
                let resolved_otherwise = self.resolve_expr(otherwise);
                let ty = Self::get_expr_type(&resolved_otherwise.0);
                (TExpr::Guard(resolved_branches, Box::new(resolved_otherwise), ty), span.clone())
            }
            Expr::Hole => (TExpr::Hole, span.clone()),
            Expr::Try(inner) => {
                let t_inner = self.resolve_expr(inner);
                let ty = Self::get_expr_type(&t_inner.0);
                // Unwrap Result or Optional
                let inner_ty = if let TypeRef::Generic(name, args) = ty {
                    if (name == "Result" || name == "Optional") && !args.is_empty() {
                        args[0].0.clone()
                    } else { TypeRef::Simple("Unknown".into()) }
                } else { TypeRef::Simple("Unknown".into()) };
                (TExpr::Try(Box::new(t_inner), inner_ty), span.clone())
            }
            Expr::Tuple(es) => {
                let mut resolved = Vec::new();
                let mut types = Vec::new();
                for e in es {
                    let t_e = self.resolve_expr(e);
                    types.push((Self::get_expr_type(&t_e.0), 0..0));
                    resolved.push(t_e);
                }
                (TExpr::Tuple(resolved, TypeRef::Generic("Tuple".into(), types), crate::type_checker::tast::AllocMode::Local), span.clone())
            }
            Expr::List(es) => {
                let mut resolved = Vec::new();
                let mut elem_ty = TypeRef::Simple("Unknown".into());
                for e in es {
                    let t_e = self.resolve_expr(e);
                    elem_ty = Self::get_expr_type(&t_e.0);
                    resolved.push(t_e);
                }
                (TExpr::List(resolved, TypeRef::Generic("List".into(), vec![(elem_ty, 0..0)]), crate::type_checker::tast::AllocMode::Local), span.clone())
            }
            Expr::ExplicitApp(inner) => {
                let (e, s) = &**inner;
                match e {
                    Expr::Tuple(es) | Expr::Sequence(es) => {
                        let resolved = self.resolve_sequence(es);
                        if resolved.len() == 1 { resolved[0].clone() } 
                        else {
                            let ty = if let Some(last) = resolved.last() { Self::get_expr_type(&last.0) } else { TypeRef::Simple("()".to_string()) };
                            (TExpr::Sequence(resolved, ty), s.clone())
                        }
                    }
                    _ => self.resolve_expr(inner),
                }
            }
            Expr::Ident(name) => {
                if let Some(ty) = self.local_vars.borrow().get(name) { (TExpr::Ident(name.clone(), ty.clone()), span.clone()) } 
                else if let Some(info) = self.env.lookup_first(name) {
                    if self.pure_context.get() && info.is_action { self.errors.borrow_mut().push((format!("Erro de Pureza: `{}`", name), span.clone())); }
                    self.check_recursion(name, span);
                    (TExpr::Ident(name.clone(), info.type_info.clone()), span.clone())
                } else {
                    self.check_recursion(name, span);
                    (TExpr::Ident(name.clone(), TypeRef::Simple("Unknown".to_string())), span.clone())
                }
            }
            Expr::ActionCall(name, args) => {
                if self.pure_context.get() {
                    self.errors.borrow_mut().push((format!("Erro de Pureza: `{}`", name), span.clone()));
                }
                self.check_recursion(name, span);
                let mut resolved_args = Vec::new();
                for arg in args {
                    resolved_args.push(self.resolve_expr(arg));
                }

                if let Some(infos) = self.env.lookup_all(name) {
                    if let Some(info) = infos.iter().find(|i| i.is_action) {
                        if let TypeRef::Function(params, ret) = &info.type_info {
                            let mut all_match = true;
                            let mut p_idx = 0;
                            let mut a_idx = 0;
                            while a_idx < resolved_args.len() {
                                if p_idx >= params.len() {
                                    all_match = false; break;
                                }
                                match &params[p_idx].0 {
                                    TypeRef::Variadic(inner_p) => {
                                        let arg_ty = Self::get_expr_type(&resolved_args[a_idx].0);
                                        if self.types_compatible(&arg_ty, &inner_p.0) {
                                            // Greedy consume
                                            a_idx += 1;
                                        } else {
                                            // Move to next parameter and try matching current argument again
                                            p_idx += 1;
                                        }
                                    }
                                    other_p => {
                                        let arg_ty = Self::get_expr_type(&resolved_args[a_idx].0);
                                        if self.types_compatible(&arg_ty, other_p) {
                                            a_idx += 1;
                                            p_idx += 1;
                                        } else {
                                            all_match = false; break;
                                        }
                                    }
                                }
                            }
                            // Check if remaining parameters are optional (variadic)
                            while p_idx < params.len() {
                                if !matches!(params[p_idx].0, TypeRef::Variadic(_)) {
                                    all_match = false; break;
                                }
                                p_idx += 1;
                            }
                            
                            if !all_match {
                                self.errors.borrow_mut().push((format!("Erro de Tipo na chamada da Action `{}`", name), span.clone()));
                            }
                            return (TExpr::Call(
                                Box::new((TExpr::Ident(name.clone(), info.type_info.clone()), span.clone())),
                                resolved_args,
                                ret.0.clone()
                            ), span.clone());
                        }
                    }
                }
                
                self.errors.borrow_mut().push((format!("Action desconhecida: `{}`", name), span.clone()));
                (TExpr::Literal(TLiteral::Unit), span.clone())
            }
            Expr::ChannelSend => (TExpr::Ident("!>".into(), TypeRef::Simple("Unknown".into())), span.clone()),
            Expr::ChannelRecv => (TExpr::Ident("<!".into(), TypeRef::Simple("Unknown".into())), span.clone()),
            Expr::ChannelRecvNonBlock => (TExpr::Ident("<!?".into(), TypeRef::Simple("Unknown".into())), span.clone()),
            Expr::WithDecl(name, expr) => {
                let resolved = self.resolve_expr(expr);
                let ty = Self::get_expr_type(&resolved.0);
                self.declare_local(name.clone(), ty);
                resolved
            }
            Expr::Lambda(args, body, with) => {
                let mut resolved_with = Vec::new();
                for w in with {
                    resolved_with.push(self.resolve_expr(w));
                }
                let resolved_body = self.resolve_expr(body);
                let ret_ty = Self::get_expr_type(&resolved_body.0);
                let mut arg_types = Vec::new();
                for _ in args {
                    arg_types.push((TypeRef::Simple("Unknown".into()), 0..0));
                }
                let ty = TypeRef::Function(arg_types, Box::new((ret_ty, span.clone())));
                (TExpr::Lambda(args.clone(), Box::new(resolved_body), ty, crate::type_checker::tast::AllocMode::Local), span.clone())
            }
            _ => (TExpr::Literal(TLiteral::Unit), span.clone()), 
        }
    }
}
