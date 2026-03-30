use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt};
use crate::parser::ast::{Spanned, TypeRef, Pattern};
use crate::optimizer::error::OptimizerError;
use std::collections::HashMap;

pub struct StreamFusionPass {
    // Map of synthesized fused functions: hash of operations -> TTopLevel
    pub fused_functions: HashMap<String, TTopLevel>,
    uid_counter: usize,
}

#[derive(Debug, Clone)]
enum StreamOp {
    Map(Spanned<TExpr>),
    Filter(Spanned<TExpr>),
}

impl StreamFusionPass {
    pub fn new() -> Self {
        Self {
            fused_functions: HashMap::new(),
            uid_counter: 0,
        }
    }

    fn next_uid(&mut self) -> usize {
        self.uid_counter += 1;
        self.uid_counter
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Fusão de Fluxos (Stream-Fusion)...");

        let mut folded_tast: Vec<Spanned<TTopLevel>> = tast.into_iter()
            .map(|(decl, span)| (self.fold_toplevel(decl, errors), span))
            .collect();

        // Inject synthesized fused functions at the end
        for (_, func_decl) in self.fused_functions.drain() {
            folded_tast.push((func_decl, 0..0));
        }

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
                    .map(|arm| crate::type_checker::tast::TMatchArm {
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

        // Try to extract a stream pipeline starting from this expression
        if let Some((source_list, pipeline, return_type)) = self.extract_pipeline(&(e.clone(), span.clone())) {
            if pipeline.len() > 1 {
                // We found a chain of at least 2 operations! Let's fuse them.
                let fused_name = format!("__fused_stream_{}", self.next_uid());
                
                // Synthesize the fused function
                self.synthesize_fused_function(&fused_name, &pipeline, &return_type);

                // Replace the current expression with a call to the new fused function
                let callee = Box::new((TExpr::Ident(fused_name.clone(), TypeRef::Simple("Unknown".to_string())), span.clone()));
                // We need to pass the source list as the first argument, and any captured closures as subsequent arguments.
                // For simplicity in Phase 1 of Stream Fusion, we pass the source list and the closures themselves.
                let mut args = vec![source_list];
                for op in pipeline {
                    match op {
                        StreamOp::Map(f) => args.push(f),
                        StreamOp::Filter(f) => args.push(f),
                    }
                }

                let fused_call = TExpr::Call(callee, args, return_type);
                return (fused_call, span);
            }
        }

        let folded = match e {
            TExpr::Call(callee, args, ty) => {
                let folded_callee = Box::new(self.fold_expr_spanned(*callee, errors));
                let folded_args: Vec<_> = args.into_iter().map(|a| self.fold_expr_spanned(a, errors)).collect();
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

    /// Recursively digs down `Call` nodes to build a pipeline of operations.
    /// Returns (Source List Expression, Pipeline of Operations, Final Return Type)
    fn extract_pipeline(&self, expr: &Spanned<TExpr>) -> Option<(Spanned<TExpr>, Vec<StreamOp>, TypeRef)> {
        let (e, _) = expr;
        if let TExpr::Call(callee, args, ty) = e {
            if let TExpr::Ident(name, _) = &callee.0 {
                if args.len() == 2 {
                    if name == "map" || name == "filter" {
                        let closure = args[0].clone();
                        let target_list = args[1].clone();
                        
                        let op = if name == "map" { StreamOp::Map(closure) } else { StreamOp::Filter(closure) };
                        
                        // Recurse to see if the target list is also a map/filter
                        if let Some((source, mut inner_pipeline, _)) = self.extract_pipeline(&target_list) {
                            inner_pipeline.push(op);
                            return Some((source, inner_pipeline, ty.clone()));
                        } else {
                            // Base case: it's a map/filter, but the target is a normal variable/list
                            return Some((target_list, vec![op], ty.clone()));
                        }
                    }
                }
            }
        }
        None
    }

    fn synthesize_fused_function(&mut self, name: &str, pipeline: &[StreamOp], ret_ty: &TypeRef) {
        log::info!("Sintetizando função de fusão de fluxo: {} para {} operações encadeadas.", name, pipeline.len());
        
        let mut sig_params = vec![(TypeRef::Simple("Unknown".to_string()), 0..0)]; // A lista source
        let mut base_args = vec![(Pattern::List(vec![]), 0..0)];
        let mut rec_args = vec![(Pattern::List(vec![(Pattern::Ident("x".to_string()), 0..0), (Pattern::Ident("xs".to_string()), 0..0)]), 0..0)];
        let mut fw_args = vec![(TExpr::Ident("xs".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)];

        for (i, _op) in pipeline.iter().enumerate() {
            sig_params.push((TypeRef::Simple("Unknown".to_string()), 0..0)); // A closure
            let f_name = format!("f{}", i);
            base_args.push((Pattern::Ident("_".to_string()), 0..0));
            rec_args.push((Pattern::Ident(f_name.clone()), 0..0));
            fw_args.push((TExpr::Ident(f_name, TypeRef::Simple("Unknown".to_string())), 0..0));
        }

        let sig = TTopLevel::Signature(name.to_string(), sig_params, (ret_ty.clone(), 0..0), Vec::new());
        
        // Caso Base: lambda [] ... : []
        let base_lambda = TTopLevel::LambdaDef(
            base_args,
            (TExpr::List(vec![], ret_ty.clone(), crate::type_checker::tast::AllocMode::Local), 0..0),
            Vec::new(),
            Vec::new()
        );

        // Caso Recursivo: lambda [x:xs] f0 f1 ...
        // Chamada recursiva para a cauda: __fused_stream_X(xs, f0, f1...)
        let rec_call = (TExpr::Call(
            Box::new((TExpr::Ident(name.to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            fw_args,
            ret_ty.clone()
        ), 0..0);

        // Construir o corpo de dentro para fora
        let mut current_val = (TExpr::Ident("x".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0);
        let mut requires_guard = false;
        let mut condition = None;

        for (i, op) in pipeline.iter().enumerate() {
            let f_name = format!("f{}", i);
            let callee = Box::new((TExpr::Ident(f_name, TypeRef::Simple("Unknown".to_string())), 0..0));
            
            match op {
                StreamOp::Map(_) => {
                    current_val = (TExpr::Call(callee, vec![current_val.clone()], TypeRef::Simple("Unknown".to_string())), 0..0);
                }
                StreamOp::Filter(_) => {
                    // O filtro cria um ponto de ramificação (Guard)
                    requires_guard = true;
                    // Assumimos um único filtro por simplificação no protótipo, ou aglomeramos as condições com `and`
                    let cond_expr = (TExpr::Call(callee, vec![current_val.clone()], TypeRef::Simple("Bool".to_string())), 0..0);
                    if let Some(existing_cond) = condition {
                        let and_callee = Box::new((TExpr::Ident("and".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0));
                        condition = Some((TExpr::Call(and_callee, vec![existing_cond, cond_expr], TypeRef::Simple("Bool".to_string())), 0..0));
                    } else {
                        condition = Some(cond_expr);
                    }
                }
            }
        }

        // Concatena o elemento resultante com a chamada recursiva: + [current_val] rec_call
        let yield_list = (TExpr::List(vec![current_val], ret_ty.clone(), crate::type_checker::tast::AllocMode::Local), 0..0);
        let concat_callee = Box::new((TExpr::Ident("+".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0));
        let yield_expr = (TExpr::Call(concat_callee, vec![yield_list, rec_call.clone()], ret_ty.clone()), 0..0);

        let final_body = if requires_guard {
            // Guard: condition -> yield_expr, otherwise -> rec_call
            (TExpr::Guard(
                vec![(condition.unwrap(), yield_expr)],
                Box::new(rec_call),
                ret_ty.clone()
            ), 0..0)
        } else {
            yield_expr
        };

        let rec_lambda = TTopLevel::LambdaDef(
            rec_args,
            final_body,
            Vec::new(),
            Vec::new()
        );

        self.fused_functions.insert(name.to_string(), sig);
        self.fused_functions.insert(format!("{}_base", name), base_lambda);
        self.fused_functions.insert(format!("{}_rec", name), rec_lambda);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    

    fn dummy_ident(name: &str, ty: &str) -> Spanned<TExpr> {
        (TExpr::Ident(name.to_string(), TypeRef::Simple(ty.to_string())), 0..0)
    }

    fn dummy_call(name: &str, args: Vec<Spanned<TExpr>>, ret_ty: &str) -> Spanned<TExpr> {
        (TExpr::Call(Box::new(dummy_ident(name, "Unknown")), args, TypeRef::Simple(ret_ty.to_string())), 0..0)
    }

    #[test]
    fn test_extract_pipeline() {
        let pass = StreamFusionPass::new();

        // Construindo AST para: map f3 (filter f2 (map f1 list_a))
        let source_list = dummy_ident("list_a", "List::Int");
        
        let call_map1 = dummy_call("map", vec![dummy_ident("f1", "Func"), source_list], "List::Int");
        let call_filter = dummy_call("filter", vec![dummy_ident("f2", "Func"), call_map1], "List::Int");
        let call_map2 = dummy_call("map", vec![dummy_ident("f3", "Func"), call_filter], "List::Int");

        let pipeline_opt = pass.extract_pipeline(&call_map2);
        
        assert!(pipeline_opt.is_some(), "Deveria ter extraído um pipeline.");
        let (source, ops, _) = pipeline_opt.unwrap();

        // A fonte original (profunda) deve ser 'list_a'
        if let TExpr::Ident(name, _) = source.0 {
            assert_eq!(name, "list_a");
        } else {
            panic!("A fonte extraída está incorreta.");
        }

        // A ordem deve ser de dentro para fora: [Map(f1), Filter(f2), Map(f3)]
        assert_eq!(ops.len(), 3);
        assert!(matches!(ops[0], StreamOp::Map(_)));
        assert!(matches!(ops[1], StreamOp::Filter(_)));
        assert!(matches!(ops[2], StreamOp::Map(_)));
    }

    #[test]
    fn test_stream_fusion_transformation() {
        let mut pass = StreamFusionPass::new();
        let mut errors = Vec::new();

        // lambda lista: map f2 (filter f1 lista)
        let source_list = dummy_ident("lista", "List::Int");
        let call_filter = dummy_call("filter", vec![dummy_ident("f1", "Func"), source_list], "List::Int");
        let call_map = dummy_call("map", vec![dummy_ident("f2", "Func"), call_filter], "List::Int");
        
        let lambda_def = TTopLevel::LambdaDef(
            vec![(Pattern::Ident("lista".to_string()), 0..0)],
            call_map.clone(),
            Vec::new(),
            Vec::new()
        );

        let tast = vec![(lambda_def, 0..0)];
        let optimized = pass.run(tast, &mut errors);

        assert!(errors.is_empty(), "Não deveriam existir erros.");

        // O Lambda original foi transformado (o corpo não é mais chamadas encadeadas, mas sim __fused_stream_1)
        if let TTopLevel::LambdaDef(_, body, _, _) = &optimized[0].0 {
            if let TExpr::Call(callee, args, _) = &body.0 {
                if let TExpr::Ident(name, _) = &callee.0 {
                    assert_eq!(name, "__fused_stream_1", "O corpo do lambda não foi substituído pela função sintetizada.");
                    assert_eq!(args.len(), 3, "A chamada para a função sintetizada deve passar a fonte e as 2 closures.");
                } else {
                    panic!("Callee incorreto.");
                }
            } else {
                panic!("Corpo não foi substituído por Call.");
            }
        } else {
            panic!("Nó principal perdido.");
        }

        // Deve ter injetado as novas funções de fusão no final (Signature, Base, Rec)
        assert!(optimized.len() >= 4, "As definições da nova função não foram injetadas no nível superior da TAST.");
        
        let has_sig = optimized.iter().any(|(d, _)| matches!(d, TTopLevel::Signature(n, ..) if n == "__fused_stream_1"));
        let has_base = optimized.iter().any(|(d, _)| matches!(d, TTopLevel::LambdaDef(params, ..) if params.first().map(|(p, _)| matches!(p, Pattern::List(l) if l.is_empty())).unwrap_or(false)));
        
        assert!(has_sig, "Falta a assinatura da função sintetizada.");
        assert!(has_base, "Falta o caso base da recursão (lista vazia).");
    }
}
