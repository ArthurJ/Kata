use crate::type_checker::directives::KataDirective;
use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TLiteral, TStmt};
use crate::parser::ast::{Spanned, TypeRef, Pattern};
use crate::optimizer::error::OptimizerError;
use std::collections::HashMap;

pub struct TcoPass {
    pub associative_ops: HashMap<String, Option<TExpr>>,
}

#[derive(Debug, Clone, PartialEq)]
enum TcoStatus {
    Tail,
    Trma { op: String, non_rec_arg: Box<Spanned<TExpr>>, is_left: bool }, // is_left: true if op(non_rec, rec_call)
    Invalid,
}

impl TcoPass {
    pub fn new() -> Self {
        Self {
            associative_ops: HashMap::new(),
        }
    }

    fn type_to_string(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::TypeVar(n) => n.clone(),
            TypeRef::Simple(n) => n.clone(),
            TypeRef::Generic(n, args) => {
                let args_str: Vec<String> = args.iter().map(|a| self.type_to_string(&a.0)).collect();
                format!("{}_{}", n, args_str.join("_"))
            }
            TypeRef::Function(_, _) => "Func".to_string(),
            TypeRef::Refined(base, _) => format!("Refined_{}", self.type_to_string(&base.0)),
            TypeRef::Variadic(inner) => format!("Var_{}", self.type_to_string(&inner.0)),
            TypeRef::Const(expr) => match expr {
                crate::parser::ast::Expr::Int(i) => format!("ConstInt_{}", i),
                crate::parser::ast::Expr::Float(f) => format!("ConstFloat_{}", f.replace(".", "_")),
                crate::parser::ast::Expr::String(s) => format!("ConstStr_{}", s),
                crate::parser::ast::Expr::Ident(id) => format!("ConstId_{}", id),
                _ => "ConstUnknown".to_string(),
            },
        }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Enforcement de TCO e Transformação TRMA (Tail Recursion Modulo Associativity)...");

        let mut final_tast = Vec::new();
        let mut current_sig: Option<Spanned<TTopLevel>> = None;
        let mut current_lambdas: Vec<Spanned<TTopLevel>> = Vec::new();

        let process_group = |sig_opt: Option<Spanned<TTopLevel>>, lambdas: Vec<Spanned<TTopLevel>>, final_tast: &mut Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>, me: &TcoPass| {
            if let Some(sig_decl) = sig_opt {
                if lambdas.is_empty() {
                    final_tast.push(sig_decl);
                    return;
                }

                let func_name = match &sig_decl.0 {
                    TTopLevel::Signature(n, _, _, _) => n.clone(),
                    _ => unreachable!(),
                };

                let mut requires_trma = false;
                let mut trma_op = None;
                let mut is_invalid = false;
                let mut has_recursion = false;

                for lambda in &lambdas {
                    if let TTopLevel::LambdaDef(_, body, _, _) = &lambda.0 {
                        if me.has_any_recursion(body, &func_name) {
                            has_recursion = true;
                        }
                        
                        let status = me.analyze_recursion(body, &func_name, true);
                        match status {
                            TcoStatus::Invalid => { is_invalid = true; }
                            TcoStatus::Trma { op, .. } => {
                                requires_trma = true;
                                if let Some(existing_op) = &trma_op {
                                    if existing_op != &op {
                                        is_invalid = true;
                                    }
                                } else {
                                    trma_op = Some(op);
                                }
                            }
                            TcoStatus::Tail => {}
                        }
                    }
                }

                if is_invalid && has_recursion {
                    errors.push(OptimizerError::new(
                        format!("Erro Fatal de TCO: A função `{}` possui recursão que não está em posição de cauda (Ex: Fibonacci ramificado ou chamadas encadeadas complexas) e não pôde ser otimizada automaticamente. Reescreva a lógica.", func_name),
                        lambdas[0].1.clone()
                    ));
                    final_tast.push(sig_decl);
                    final_tast.extend(lambdas);
                    return;
                }

                if requires_trma && !is_invalid {
                    let op = trma_op.unwrap();
                    if let TTopLevel::Signature(_, params, ret, _) = &sig_decl.0.clone() {
                        let ret_type = &ret.0;
                        let identity_expr_opt = me.get_identity(&op, ret_type);

                        if let Some(identity_expr) = identity_expr_opt {
                            if let Some(identity) = identity_expr {
                                log::info!("Otimizando recursão de `{}` para TCO usando Acumulador (Operação: {}).", func_name, op);
                                
                                let acc_name = format!("{}_acc", func_name);
                                let mut acc_params = params.clone();
                                acc_params.push((ret_type.clone(), 0..0));
                                let acc_sig = TTopLevel::Signature(acc_name.clone(), acc_params, ret.clone(), Vec::new());
                                final_tast.push((acc_sig, 0..0));

                                for lambda in &lambdas {
                                    if let TTopLevel::LambdaDef(l_params, body, with, dirs) = &lambda.0 {
                                        let mut new_params = l_params.clone();
                                        new_params.push((Pattern::Ident("__acc".to_string()), 0..0));
                                        let new_body = me.rewrite_trma_body(body, &func_name, &acc_name, &op, ret_type);
                                        final_tast.push((TTopLevel::LambdaDef(new_params, new_body, with.clone(), dirs.clone()), lambda.1.clone()));
                                    }
                                }

                                final_tast.push(sig_decl);
                                let mut fw_params = Vec::new();
                                let mut fw_args = Vec::new();
                                for (i, (p_ty, _)) in params.iter().enumerate() {
                                    let p_name = format!("__arg{}", i);
                                    fw_params.push((Pattern::Ident(p_name.clone()), 0..0));
                                    fw_args.push((TExpr::Ident(p_name, p_ty.clone()), 0..0));
                                }
                                fw_args.push((identity.clone(), 0..0));
                                
                                let callee = Box::new((TExpr::Ident(acc_name, TypeRef::Simple("Unknown".to_string())), 0..0));
                                let fw_body = TExpr::Call(callee, fw_args, ret_type.clone());
                                final_tast.push((TTopLevel::LambdaDef(fw_params, (fw_body, 0..0), Vec::new(), Vec::new()), 0..0));
                            } else {
                                errors.push(OptimizerError::new(
                                    format!("Erro Fatal de TCO: A função `{}` falha a otimização de cauda. A operação `{}` é associativa, mas não declarou um elemento neutro na diretiva @associative. O compilador não pode injetar um acumulador de forma segura. Reescreva a função usando um laço/acumulador manual.", func_name, op),
                                    lambdas[0].1.clone()
                                ));
                                final_tast.push(sig_decl);
                                final_tast.extend(lambdas);
                            }
                        } else {
                            errors.push(OptimizerError::new(
                                format!("Erro de TRMA: A operação `{}` retorna `{:?}` na função `{}`, mas não possui a diretiva @associative declarada na sua assinatura original.", op, ret_type, func_name),
                                lambdas[0].1.clone()
                            ));
                            final_tast.push(sig_decl);
                            final_tast.extend(lambdas);
                        }
                    }
                } else {
                    final_tast.push(sig_decl);
                    final_tast.extend(lambdas);
                }
            } else {
                final_tast.extend(lambdas);
            }
        };

        let mut actions = Vec::new();

        for spanned_decl in tast {
            let (decl, _span) = &spanned_decl;
            match decl {
                TTopLevel::Signature(name, _, ret, dirs) => {
                    process_group(current_sig.take(), std::mem::take(&mut current_lambdas), &mut final_tast, errors, self);
                    
                    current_sig = Some(spanned_decl.clone());

                    for (dir, _) in dirs {
                        if let KataDirective::Associative(arg_opt) = dir {
                            let key = format!("{}_{}", name, self.type_to_string(&ret.0));
                            if let Some(arg_expr) = arg_opt {
                                let texpr = match arg_expr {
                                    crate::parser::ast::Expr::Int(i) => Some(TExpr::Literal(TLiteral::Int(i.parse().unwrap_or(0)))),
                                    crate::parser::ast::Expr::Float(f) => Some(TExpr::Literal(TLiteral::Float(f.parse().unwrap_or(0.0)))),
                                    crate::parser::ast::Expr::Ident(id) if id == "True" => Some(TExpr::Literal(TLiteral::Bool(true))),
                                    crate::parser::ast::Expr::Ident(id) if id == "False" => Some(TExpr::Literal(TLiteral::Bool(false))),
                                    crate::parser::ast::Expr::String(s) => Some(TExpr::Literal(TLiteral::String(s.clone()))),
                                    crate::parser::ast::Expr::List(l) if l.is_empty() => Some(TExpr::List(vec![], TypeRef::Simple("Unknown".to_string()), crate::type_checker::tast::AllocMode::Local)),
                                    _ => None,
                                };
                                self.associative_ops.insert(key, texpr);
                            } else {
                                self.associative_ops.insert(key, None);
                            }
                        }
                    }
                }
                TTopLevel::LambdaDef(..) => {
                    if current_sig.is_some() {
                        current_lambdas.push(spanned_decl.clone());
                    } else {
                        final_tast.push(spanned_decl.clone());
                    }
                }
                TTopLevel::ActionDef(..) => {
                    process_group(current_sig.take(), std::mem::take(&mut current_lambdas), &mut final_tast, errors, self);
                    actions.push(spanned_decl.clone());
                    final_tast.push(spanned_decl.clone());
                }
                _ => {
                    process_group(current_sig.take(), std::mem::take(&mut current_lambdas), &mut final_tast, errors, self);
                    final_tast.push(spanned_decl.clone());
                }
            }
        }
        process_group(current_sig.take(), std::mem::take(&mut current_lambdas), &mut final_tast, errors, self);

        // Fase 3: Validar Actions
        for (action, _span) in actions {
            if let TTopLevel::ActionDef(name, _, _, body, _) = &action {
                for stmt in body {
                    self.check_action_recursion(stmt, name, errors);
                }
            }
        }

        final_tast
    }

    fn has_any_recursion(&self, expr: &Spanned<TExpr>, func_name: &str) -> bool {
        let (e, _) = expr;
        match e {
            TExpr::Call(callee, args, _) => {
                if let TExpr::Ident(name, _) = &callee.0 {
                    if name == func_name { return true; }
                }
                if self.has_any_recursion(callee, func_name) { return true; }
                args.iter().any(|a| self.has_any_recursion(a, func_name))
            }
            TExpr::Tuple(es, _, _) | TExpr::List(es, _, _) | TExpr::Sequence(es, _) => es.iter().any(|e| self.has_any_recursion(e, func_name)),
            TExpr::Array(rows, _, _) => rows.iter().any(|row| row.iter().any(|e| self.has_any_recursion(e, func_name))),
            TExpr::Lambda(_, b, _, _) => self.has_any_recursion(b, func_name),
            TExpr::Guard(branches, otherwise, _) => {
                branches.iter().any(|(c, b)| self.has_any_recursion(c, func_name) || self.has_any_recursion(b, func_name)) || self.has_any_recursion(otherwise, func_name)
            }
            TExpr::Try(inner, _) => self.has_any_recursion(inner, func_name),
            TExpr::ChannelSend(t, v, _) => self.has_any_recursion(t, func_name) || self.has_any_recursion(v, func_name),
            TExpr::ChannelRecv(t, _) | TExpr::ChannelRecvNonBlock(t, _) => self.has_any_recursion(t, func_name),
            _ => false,
        }
    }

    fn analyze_recursion(&self, expr: &Spanned<TExpr>, func_name: &str, is_tail: bool) -> TcoStatus {
        let (e, _) = expr;
        match e {
            TExpr::Call(callee, args, ty) => {
                if let TExpr::Ident(name, _) = &callee.0 {
                    if name == func_name {
                        return if is_tail { TcoStatus::Tail } else { TcoStatus::Invalid };
                    }

                    // Verifica se é uma operação TRMA válida (registada via @associative)
                    let key = format!("{}_{}", name, self.type_to_string(ty));
                    let is_associative_op = self.associative_ops.contains_key(&key);
                    
                    if is_associative_op && args.len() == 2 {
                        let left_has_rec = self.has_any_recursion(&args[0], func_name);
                        let right_has_rec = self.has_any_recursion(&args[1], func_name);

                        if left_has_rec && !right_has_rec {
                            // Validar se a chamada recursiva à esquerda está em posição de cauda *relativa* a esta operação
                            let left_status = self.analyze_recursion(&args[0], func_name, true);
                            if matches!(left_status, TcoStatus::Tail) {
                                return TcoStatus::Trma { op: name.clone(), non_rec_arg: Box::new(args[1].clone()), is_left: false };
                            }
                        } else if !left_has_rec && right_has_rec {
                            let right_status = self.analyze_recursion(&args[1], func_name, true);
                            if matches!(right_status, TcoStatus::Tail) {
                                return TcoStatus::Trma { op: name.clone(), non_rec_arg: Box::new(args[0].clone()), is_left: true };
                            }
                        }
                    }
                }

                // Se não é a função atual nem uma operação TRMA válida, qualquer recursão nos argumentos é inválida.
                if self.has_any_recursion(callee, func_name) { return TcoStatus::Invalid; }
                for arg in args {
                    if self.has_any_recursion(arg, func_name) {
                        if self.analyze_recursion(arg, func_name, false) == TcoStatus::Invalid {
                            return TcoStatus::Invalid;
                        }
                        return TcoStatus::Invalid; // Força falha se tiver recursão num argumento
                    }
                }
                TcoStatus::Tail
            }
            TExpr::Sequence(exprs, _) => {
                let mut status = TcoStatus::Tail;
                for (i, ex) in exprs.iter().enumerate() {
                    let tail = is_tail && i == exprs.len() - 1;
                    if self.has_any_recursion(ex, func_name) {
                        let st = self.analyze_recursion(ex, func_name, tail);
                        if st == TcoStatus::Invalid { return TcoStatus::Invalid; }
                        status = st;
                    }
                }
                status
            }
            TExpr::Guard(branches, otherwise, _) => {
                let mut trma = None;
                for (cond, body) in branches {
                    if self.has_any_recursion(cond, func_name) { return TcoStatus::Invalid; }
                    if self.has_any_recursion(body, func_name) {
                        let st = self.analyze_recursion(body, func_name, is_tail);
                        if st == TcoStatus::Invalid { return TcoStatus::Invalid; }
                        if let TcoStatus::Trma { .. } = st { trma = Some(st); }
                    }
                }
                if self.has_any_recursion(otherwise, func_name) {
                    let st = self.analyze_recursion(otherwise, func_name, is_tail);
                    if st == TcoStatus::Invalid { return TcoStatus::Invalid; }
                    if let TcoStatus::Trma { .. } = st { trma = Some(st); }
                }
                trma.unwrap_or(TcoStatus::Tail)
            }
            TExpr::Lambda(_, body, _, _) => {
                if self.has_any_recursion(body, func_name) { return TcoStatus::Invalid; }
                TcoStatus::Tail
            }
            TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) => {
                for ex in exprs {
                    if self.has_any_recursion(ex, func_name) { return TcoStatus::Invalid; }
                }
                TcoStatus::Tail
            }
            TExpr::Array(rows, _, _) => {
                for row in rows {
                    for ex in row {
                        if self.has_any_recursion(ex, func_name) { return TcoStatus::Invalid; }
                    }
                }
                TcoStatus::Tail
            }
            TExpr::Try(inner, _) => {
                if self.has_any_recursion(inner, func_name) { return TcoStatus::Invalid; }
                TcoStatus::Tail
            }
            TExpr::ChannelSend(t, v, _) => {
                if self.has_any_recursion(t, func_name) || self.has_any_recursion(v, func_name) { return TcoStatus::Invalid; }
                TcoStatus::Tail
            }
            TExpr::ChannelRecv(t, _) | TExpr::ChannelRecvNonBlock(t, _) => {
                if self.has_any_recursion(t, func_name) { return TcoStatus::Invalid; }
                TcoStatus::Tail
            }
            _ => TcoStatus::Tail,
        }
    }

    fn rewrite_trma_body(&self, expr: &Spanned<TExpr>, func_name: &str, acc_name: &str, op: &str, ret_ty: &TypeRef) -> Spanned<TExpr> {
        let (e, span) = expr;
        
        if !self.has_any_recursion(expr, func_name) {
            // Caso Base: body vira `op(body, __acc)` ou `op(__acc, body)`
            let acc_ident = (TExpr::Ident("__acc".to_string(), ret_ty.clone()), span.clone());
            let callee = Box::new((TExpr::Ident(op.to_string(), TypeRef::Function(vec![(ret_ty.clone(), 0..0), (ret_ty.clone(), 0..0)], Box::new((ret_ty.clone(), 0..0)))), span.clone()));
            return (TExpr::Call(callee, vec![expr.clone(), acc_ident], ret_ty.clone()), span.clone());
        }

        match e {
            TExpr::Call(callee, args, ty) => {
                if let TExpr::Ident(name, _) = &callee.0 {
                    if name == op && args.len() == 2 {
                        let left_rec = self.has_any_recursion(&args[0], func_name);
                        let right_rec = self.has_any_recursion(&args[1], func_name);

                        if left_rec && !right_rec {
                            // op(rec_call, expr) => acc_func(..., op(expr, __acc))
                            let non_rec = &args[1];
                            let acc_ident = (TExpr::Ident("__acc".to_string(), ret_ty.clone()), span.clone());
                            let new_acc = (TExpr::Call(callee.clone(), vec![non_rec.clone(), acc_ident], ret_ty.clone()), span.clone());
                            
                            // A chamada recursiva à esquerda precisa ser transformada numa chamada ao acc_func
                            if let TExpr::Call(rec_callee, rec_args, rec_ty) = &args[0].0 {
                                if let TExpr::Ident(rn, _) = &rec_callee.0 {
                                    if rn == func_name {
                                        let new_rec_callee = Box::new((TExpr::Ident(acc_name.to_string(), TypeRef::Simple("Unknown".to_string())), rec_callee.1.clone()));
                                        let mut new_rec_args = rec_args.clone();
                                        new_rec_args.push(new_acc);
                                        return (TExpr::Call(new_rec_callee, new_rec_args, rec_ty.clone()), args[0].1.clone());
                                    }
                                }
                            }
                        } else if !left_rec && right_rec {
                            // op(expr, rec_call) => acc_func(..., op(expr, __acc))
                            let non_rec = &args[0];
                            let acc_ident = (TExpr::Ident("__acc".to_string(), ret_ty.clone()), span.clone());
                            let new_acc = (TExpr::Call(callee.clone(), vec![non_rec.clone(), acc_ident], ret_ty.clone()), span.clone());
                            
                            if let TExpr::Call(rec_callee, rec_args, rec_ty) = &args[1].0 {
                                if let TExpr::Ident(rn, _) = &rec_callee.0 {
                                    if rn == func_name {
                                        let new_rec_callee = Box::new((TExpr::Ident(acc_name.to_string(), TypeRef::Simple("Unknown".to_string())), rec_callee.1.clone()));
                                        let mut new_rec_args = rec_args.clone();
                                        new_rec_args.push(new_acc);
                                        return (TExpr::Call(new_rec_callee, new_rec_args, rec_ty.clone()), args[1].1.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                
                // Se chegou aqui e tem recursão, mas não é o nó do TRMA, desce
                let new_callee = Box::new(self.rewrite_trma_body(callee, func_name, acc_name, op, ret_ty));
                let new_args = args.iter().map(|a| self.rewrite_trma_body(a, func_name, acc_name, op, ret_ty)).collect();
                (TExpr::Call(new_callee, new_args, ty.clone()), span.clone())
            }
            TExpr::Sequence(exprs, ty) => {
                let mut new_exprs = Vec::new();
                for ex in exprs {
                    new_exprs.push(self.rewrite_trma_body(ex, func_name, acc_name, op, ret_ty));
                }
                (TExpr::Sequence(new_exprs, ty.clone()), span.clone())
            }
            TExpr::Guard(branches, otherwise, ty) => {
                let mut new_branches = Vec::new();
                for (cond, body) in branches {
                    new_branches.push((cond.clone(), self.rewrite_trma_body(body, func_name, acc_name, op, ret_ty)));
                }
                let new_otherwise = Box::new(self.rewrite_trma_body(otherwise, func_name, acc_name, op, ret_ty));
                (TExpr::Guard(new_branches, new_otherwise, ty.clone()), span.clone())
            }
            _ => expr.clone() // Outros nós não devem conter recursões (validadas pela Fase 2)
        }
    }

    fn get_identity(&self, op: &str, ty: &TypeRef) -> Option<&Option<TExpr>> {
        let key = format!("{}_{}", op, self.type_to_string(ty));
        self.associative_ops.get(&key)
    }

    fn check_action_recursion(&self, stmt: &Spanned<TStmt>, func_name: &str, errors: &mut Vec<OptimizerError>) {
        let (s, _span) = stmt;
        match s {
            TStmt::Let(_, expr) | TStmt::Var(_, expr) | TStmt::Expr(expr) => {
                self.check_action_expr(expr, func_name, errors);
            }
            TStmt::Loop(body) => {
                for s in body {
                    self.check_action_recursion(s, func_name, errors);
                }
            }
            TStmt::For(_, iter, body) => {
                self.check_action_expr(iter, func_name, errors);
                for s in body {
                    self.check_action_recursion(s, func_name, errors);
                }
            }
            TStmt::Match(target, arms) => {
                self.check_action_expr(target, func_name, errors);
                for arm in arms {
                    for s in &arm.body {
                        self.check_action_recursion(s, func_name, errors);
                    }
                }
            }
            TStmt::Select(arms, timeout) => {
                for arm in arms {
                    self.check_action_expr(&arm.operation, func_name, errors);
                    for s in &arm.body {
                        self.check_action_recursion(s, func_name, errors);
                    }
                }
                if let Some((e, b)) = timeout {
                    self.check_action_expr(e, func_name, errors);
                    for s in b {
                        self.check_action_recursion(s, func_name, errors);
                    }
                }
            }
            TStmt::ActionAssign(t, v) => {
                self.check_action_expr(t, func_name, errors);
                self.check_action_expr(v, func_name, errors);
            }
            TStmt::Break | TStmt::Continue | TStmt::DropShared(_) => {}
        }
    }

    fn check_action_expr(&self, expr: &Spanned<TExpr>, func_name: &str, errors: &mut Vec<OptimizerError>) {
        let (e, span) = expr;
        match e {
            TExpr::Call(callee, args, _) => {
                if let TExpr::Ident(callee_name, _) = &callee.0 {
                    if callee_name == func_name {
                        errors.push(OptimizerError::new(
                            format!("Erro Fatal de Arquitetura: A Action `{}` e recursiva. Actions compilam para Maquinas de Estado impuras e nao suportam recursao. Use loops imperativos (loop/for).", func_name),
                            span.clone()
                        ));
                    }
                }
                self.check_action_expr(callee, func_name, errors);
                for arg in args {
                    self.check_action_expr(arg, func_name, errors);
                }
            }
            TExpr::Sequence(exprs, _) | TExpr::Tuple(exprs, _, _) | TExpr::List(exprs, _, _) => {
                for expr in exprs {
                    self.check_action_expr(expr, func_name, errors);
                }
            }
            TExpr::Array(rows, _, _) => {
                for row in rows {
                    for expr in row {
                        self.check_action_expr(expr, func_name, errors);
                    }
                }
            }
            TExpr::Guard(branches, otherwise, _) => {
                for (cond, body) in branches {
                    self.check_action_expr(cond, func_name, errors);
                    self.check_action_expr(body, func_name, errors);
                }
                self.check_action_expr(otherwise, func_name, errors);
            }
            TExpr::Lambda(_, body, _, _) => {
                self.check_action_expr(body, func_name, errors);
            }
            TExpr::Try(inner, _) => self.check_action_expr(inner, func_name, errors),
            TExpr::ChannelSend(target, val, _) => {
                self.check_action_expr(target, func_name, errors);
                self.check_action_expr(val, func_name, errors);
            }
            TExpr::ChannelRecv(target, _) | TExpr::ChannelRecvNonBlock(target, _) => {
                self.check_action_expr(target, func_name, errors);
            }
            TExpr::Ident(_, _) | TExpr::Literal(_) | TExpr::Hole => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::{Expr, Directive};
use crate::type_checker::directives::KataDirective;

    // Helper to create a dummy expression
    fn dummy_ident(name: &str) -> Spanned<TExpr> {
        (TExpr::Ident(name.to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)
    }

    #[test]
    fn test_trma_success() {
        let mut pass = TcoPass::new();
        let mut errors = Vec::new();

        // fact :: Int => Int
        let sig = (TTopLevel::Signature("fact".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Int".to_string()), 0..0), Vec::new()), 0..0);
        
        // Simular a presença do operador '*' com diretiva @associative(1)
        let mul_sig = (TTopLevel::Signature("*".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0), (TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Int".to_string()), 0..0), vec![
            (KataDirective::Associative(Some(Expr::Int("1".to_string()))), 0..0)
        ]), 0..0);

        // lambda n: * n (fact (- n 1))
        let rec_call = (TExpr::Call(Box::new(dummy_ident("fact")), vec![dummy_ident("n_minus_1")], TypeRef::Simple("Int".to_string())), 0..0);
        let mul_call = (TExpr::Call(Box::new((TExpr::Ident("*".to_string(), TypeRef::Function(vec![], Box::new((TypeRef::Simple("Int".to_string()), 0..0)))), 0..0)), vec![dummy_ident("n"), rec_call], TypeRef::Simple("Int".to_string())), 0..0);
        
        let lambda = (TTopLevel::LambdaDef(vec![(Pattern::Ident("n".to_string()), 0..0)], mul_call, Vec::new(), Vec::new()), 0..0);

        let tast = vec![mul_sig, sig, lambda];
        let optimized = pass.run(tast, &mut errors);

        assert!(errors.is_empty(), "Não deveriam existir erros num TRMA válido. Erros: {:?}", errors);
        assert!(optimized.len() > 3, "Deveria ter gerado assinaturas e lambdas auxiliares (_acc).");
        
        // Verifica se a função auxiliar foi gerada
        let has_acc = optimized.iter().any(|(decl, _)| {
            if let TTopLevel::Signature(name, params, _, _) = decl {
                name == "fact_acc" && params.len() == 2 // ganhou 1 argumento (o acumulador)
            } else { false }
        });
        assert!(has_acc, "A assinatura fact_acc não foi injetada na TAST.");
    }

    #[test]
    fn test_tco_hard_error_complex_recursion() {
        let mut pass = TcoPass::new();
        let mut errors = Vec::new();

        // fib :: Int => Int
        let sig = (TTopLevel::Signature("fib".to_string(), vec![(TypeRef::Simple("Int".to_string()), 0..0)], (TypeRef::Simple("Int".to_string()), 0..0), Vec::new()), 0..0);
        
        // lambda n: + (fib a) (fib b)
        let rec_call_a = (TExpr::Call(Box::new(dummy_ident("fib")), vec![dummy_ident("a")], TypeRef::Simple("Int".to_string())), 0..0);
        let rec_call_b = (TExpr::Call(Box::new(dummy_ident("fib")), vec![dummy_ident("b")], TypeRef::Simple("Int".to_string())), 0..0);
        let sum_call = (TExpr::Call(Box::new(dummy_ident("+")), vec![rec_call_a, rec_call_b], TypeRef::Simple("Int".to_string())), 0..0);
        
        let lambda = (TTopLevel::LambdaDef(vec![(Pattern::Ident("n".to_string()), 0..0)], sum_call, Vec::new(), Vec::new()), 0..0);

        let tast = vec![sig, lambda];
        pass.run(tast, &mut errors);

        assert_eq!(errors.len(), 1, "Deveria ter disparado exatamente 1 erro de compilação.");
        assert!(errors[0].message.contains("Erro Fatal de TCO"), "A mensagem de erro não bate certo.");
    }
}