use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TLiteral, TMatchArm, TStmt};
use crate::parser::ast::{Spanned, Expr, TypeRef};
use crate::type_checker::environment::TypeEnv;
use crate::optimizer::error::OptimizerError;

pub struct ComptimeEval<'a> {
    pub env: &'a TypeEnv,
}

impl<'a> ComptimeEval<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self { env }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Comptime Eval (@comptime)...");
        tast.into_iter()
            .map(|(decl, span)| (self.fold_toplevel(decl, errors), span))
            .collect()
    }

    fn fold_toplevel(&self, decl: TTopLevel, errors: &mut Vec<OptimizerError>) -> TTopLevel {
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

    fn fold_stmt_spanned(&self, stmt: Spanned<TStmt>, errors: &mut Vec<OptimizerError>) -> Spanned<TStmt> {
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

    fn fold_expr_spanned(&self, expr: Spanned<TExpr>, errors: &mut Vec<OptimizerError>) -> Spanned<TExpr> {
        let (e, span) = expr;
        let folded = match e {
            TExpr::Call(callee, args, ty) => {
                let folded_callee = Box::new(self.fold_expr_spanned(*callee, errors));
                let folded_args: Vec<_> = args.into_iter().map(|a| self.fold_expr_spanned(a, errors)).collect();

                // Intercept smart constructor calls
                if let TExpr::Ident(name, _) = &folded_callee.0 {
                    if let Some(refined_info) = self.env.lookup_refined(name) {
                        if folded_args.len() == 1 {
                            if let TExpr::Literal(lit) = &folded_args[0].0 {
                                let mut all_passed = true;
                                for pred in &refined_info.predicates {
                                    match self.evaluate_predicate(pred, lit) {
                                        Ok(true) => {} // Passed
                                        Ok(false) => {
                                            all_passed = false;
                                            errors.push(OptimizerError::new(
                                                format!("Comptime Error: Literal {:?} não satisfaz o predicado do tipo refinado `{}`.", lit, name),
                                                span.clone(),
                                            ));
                                            break;
                                        }
                                        Err(msg) => {
                                            all_passed = false;
                                            errors.push(OptimizerError::new(
                                                format!("Comptime Error ao avaliar predicado para `{}`: {}", name, msg),
                                                span.clone(),
                                            ));
                                            break;
                                        }
                                    }
                                }

                                if all_passed {
                                    // Remove the Result wrapping completely! 
                                    // The compiler accepts it statically as a native Refined Type.
                                    return (TExpr::Literal(lit.clone()), span);
                                }
                            }
                        }
                    }
                }

                TExpr::Call(folded_callee, folded_args, ty)
            }
            TExpr::Tuple(exprs, ty) => TExpr::Tuple(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(),
                ty,
            ),
            TExpr::List(exprs, ty) => TExpr::List(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(),
                ty,
            ),
            TExpr::Lambda(params, body, ty) => {
                TExpr::Lambda(params, Box::new(self.fold_expr_spanned(*body, errors)), ty)
            }
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e, errors)).collect(),
                ty,
            ),
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
            TExpr::ChannelRecv(target, ty) => {
                TExpr::ChannelRecv(Box::new(self.fold_expr_spanned(*target, errors)), ty)
            }
            TExpr::ChannelRecvNonBlock(target, ty) => {
                TExpr::ChannelRecvNonBlock(Box::new(self.fold_expr_spanned(*target, errors)), ty)
            }
            other => other,
        };
        (folded, span)
    }

    fn evaluate_predicate(&self, pred: &Expr, literal: &TLiteral) -> Result<bool, String> {
        match pred {
            Expr::Sequence(seq) => {
                if seq.len() == 3 {
                    if let (Expr::Ident(op), Expr::Hole, rhs) = (&seq[0].0, &seq[1].0, &seq[2].0) {
                        let rhs_lit = match rhs {
                            Expr::Int(i) => TLiteral::Int(i.parse().unwrap_or(0)),
                            Expr::Float(f) => TLiteral::Float(f.parse().unwrap_or(0.0)),
                            Expr::String(s) => TLiteral::String(s.clone()),
                            _ => return Err("Operando direito complexo em predicado não suportado no comptime.".into()),
                        };

                        match (op.as_str(), literal, &rhs_lit) {
                            (">", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a > b),
                            (">=", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a >= b),
                            ("<", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a < b),
                            ("<=", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a <= b),
                            ("!=", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a != b),
                            ("=", TLiteral::Int(a), TLiteral::Int(b)) => return Ok(a == b),
                            
                            (">", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a > b),
                            (">=", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a >= b),
                            ("<", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a < b),
                            ("<=", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a <= b),
                            ("!=", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a != b),
                            ("=", TLiteral::Float(a), TLiteral::Float(b)) => return Ok(a == b),

                            _ => return Err(format!("Operação `{}` não suportada estaticamente para estes tipos.", op)),
                        }
                    }
                }
                Err("Formato de predicado não suportado.".into())
            }
            _ => Err("Predicado não é uma Sequence.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::Span;

    #[test]
    fn test_comptime_refined_pass() {
        let mut env = TypeEnv::new();
        env.define_refined(
            "PositiveInt".to_string(),
            TypeRef::Simple("Int".to_string()),
            vec![
                Expr::Sequence(vec![
                    (Expr::Ident(">".to_string()), 0..0),
                    (Expr::Hole, 0..0),
                    (Expr::Int("0".to_string()), 0..0),
                ])
            ]
        );

        let mut eval = ComptimeEval::new(&env);
        let mut errors = Vec::new();

        let call = TExpr::Call(
            Box::new((TExpr::Ident("PositiveInt".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![(TExpr::Literal(TLiteral::Int(10)), 0..0)],
            TypeRef::Generic("Result".to_string(), vec![(TypeRef::Simple("PositiveInt".to_string()), 0..0)])
        );

        let folded = eval.fold_expr_spanned((call, 0..0), &mut errors);

        assert!(errors.is_empty(), "Não deveriam haver erros.");
        assert_eq!(folded.0, TExpr::Literal(TLiteral::Int(10)), "O smart constructor deveria ter sido otimizado e retornado apenas o literal puro.");
    }

    #[test]
    fn test_comptime_refined_fail() {
        let mut env = TypeEnv::new();
        env.define_refined(
            "PositiveInt".to_string(),
            TypeRef::Simple("Int".to_string()),
            vec![
                Expr::Sequence(vec![
                    (Expr::Ident(">".to_string()), 0..0),
                    (Expr::Hole, 0..0),
                    (Expr::Int("0".to_string()), 0..0),
                ])
            ]
        );

        let mut eval = ComptimeEval::new(&env);
        let mut errors = Vec::new();

        let call = TExpr::Call(
            Box::new((TExpr::Ident("PositiveInt".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![(TExpr::Literal(TLiteral::Int(-5)), 0..0)],
            TypeRef::Generic("Result".to_string(), vec![(TypeRef::Simple("PositiveInt".to_string()), 0..0)])
        );

        eval.fold_expr_spanned((call, 0..0), &mut errors);

        assert_eq!(errors.len(), 1, "Deveria haver exatamente 1 erro.");
        assert!(errors[0].message.contains("não satisfaz o predicado"), "Mensagem de erro incorreta: {}", errors[0].message);
    }
}
