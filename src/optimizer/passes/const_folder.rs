use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TLiteral, TMatchArm, TStmt};
use crate::parser::ast::Spanned;

pub struct ConstantFolder;

impl ConstantFolder {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Constant Folding...");
        tast.into_iter()
            .map(|(decl, span)| (self.fold_toplevel(decl), span))
            .collect()
    }

    fn fold_toplevel(&mut self, decl: TTopLevel) -> TTopLevel {
        match decl {
            TTopLevel::LambdaDef(params, body, with, dirs) => {
                let folded_body = self.fold_expr_spanned(body);
                let folded_with = with.into_iter().map(|w| self.fold_expr_spanned(w)).collect();
                TTopLevel::LambdaDef(params, folded_body, folded_with, dirs)
            }
            TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                let folded_body = body.into_iter().map(|s| self.fold_stmt_spanned(s)).collect();
                TTopLevel::ActionDef(name, params, ret, folded_body, dirs)
            }
            TTopLevel::Execution(expr) => {
                TTopLevel::Execution(self.fold_expr_spanned(expr))
            }
            // As declarações Data, Enum e Signature permanecem inalteradas
            other => other,
        }
    }

    fn fold_stmt_spanned(&mut self, stmt: Spanned<TStmt>) -> Spanned<TStmt> {
        let (s, span) = stmt;
        let folded = match s {
            TStmt::Let(pat, expr) => TStmt::Let(pat, self.fold_expr_spanned(expr)),
            TStmt::Var(name, expr) => TStmt::Var(name, self.fold_expr_spanned(expr)),
            TStmt::Loop(body) => TStmt::Loop(body.into_iter().map(|s| self.fold_stmt_spanned(s)).collect()),
            TStmt::For(name, iter, body) => TStmt::For(
                name,
                self.fold_expr_spanned(iter),
                body.into_iter().map(|s| self.fold_stmt_spanned(s)).collect(),
            ),
            TStmt::Match(target, arms) => {
                let folded_target = self.fold_expr_spanned(target);
                let folded_arms = arms
                    .into_iter()
                    .map(|arm| TMatchArm {
                        pattern: arm.pattern,
                        body: arm.body.into_iter().map(|s| self.fold_stmt_spanned(s)).collect(),
                    })
                    .collect();
                TStmt::Match(folded_target, folded_arms)
            }
            TStmt::Expr(expr) => TStmt::Expr(self.fold_expr_spanned(expr)),
            TStmt::Break => TStmt::Break,
            TStmt::Continue => TStmt::Continue,
        };
        (folded, span)
    }

    fn fold_expr_spanned(&mut self, expr: Spanned<TExpr>) -> Spanned<TExpr> {
        let (e, span) = expr;
        let folded = match e {
            TExpr::Call(callee, args, ty) => {
                let folded_callee = Box::new(self.fold_expr_spanned(*callee));
                let folded_args: Vec<_> = args.into_iter().map(|a| self.fold_expr_spanned(a)).collect();

                // Tenta calcular o resultado caso a função seja nativa e os argumentos sejam literais
                if let TExpr::Ident(name, _) = &folded_callee.0 {
                    if let Some(lit) = self.try_fold_call(name, &folded_args) {
                        return (TExpr::Literal(lit), span);
                    }
                }

                TExpr::Call(folded_callee, folded_args, ty)
            }
            TExpr::Tuple(exprs, ty) => TExpr::Tuple(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e)).collect(),
                ty,
            ),
            TExpr::List(exprs, ty) => TExpr::List(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e)).collect(),
                ty,
            ),
            TExpr::Lambda(params, body, ty) => {
                TExpr::Lambda(params, Box::new(self.fold_expr_spanned(*body)), ty)
            }
            TExpr::Sequence(exprs, ty) => TExpr::Sequence(
                exprs.into_iter().map(|e| self.fold_expr_spanned(e)).collect(),
                ty,
            ),
            TExpr::Guard(branches, otherwise, ty) => {
                let mut folded_branches = Vec::new();
                for (cond, body) in branches {
                    let folded_cond = self.fold_expr_spanned(cond);
                    let folded_body = self.fold_expr_spanned(body);

                    // Curto-circuito estático de Guards (Eliminação de ramos mortos/True branches)
                    if let TExpr::Literal(TLiteral::Bool(b)) = &folded_cond.0 {
                        if *b {
                            // Se a condição é True, ignoramos o resto dos ramos e do fallback
                            return folded_body;
                        } else {
                            // Condição sempre False: descartamos completamente esse ramo
                            continue;
                        }
                    }
                    folded_branches.push((folded_cond, folded_body));
                }

                let folded_otherwise = self.fold_expr_spanned(*otherwise);

                // Se todas as condições foram resolvidas como False, devolvemos apenas o otherwise
                if folded_branches.is_empty() {
                    return folded_otherwise;
                }

                TExpr::Guard(folded_branches, Box::new(folded_otherwise), ty)
            }
            TExpr::Try(inner, ty) => TExpr::Try(Box::new(self.fold_expr_spanned(*inner)), ty),
            TExpr::ChannelSend(target, val, ty) => TExpr::ChannelSend(
                Box::new(self.fold_expr_spanned(*target)),
                Box::new(self.fold_expr_spanned(*val)),
                ty,
            ),
            TExpr::ChannelRecv(target, ty) => {
                TExpr::ChannelRecv(Box::new(self.fold_expr_spanned(*target)), ty)
            }
            TExpr::ChannelRecvNonBlock(target, ty) => {
                TExpr::ChannelRecvNonBlock(Box::new(self.fold_expr_spanned(*target)), ty)
            }
            // Literal, Ident, Hole mantêm-se iguais
            other => other,
        };
        (folded, span)
    }

    fn try_fold_call(&self, name: &str, args: &[Spanned<TExpr>]) -> Option<TLiteral> {
        // Apenas dobra se TODOS os argumentos forem literais
        let all_literals = args.iter().all(|(e, _)| matches!(e, TExpr::Literal(_)));
        if !all_literals {
            return None;
        }

        let lits: Vec<&TLiteral> = args
            .iter()
            .map(|(e, _)| match e {
                TExpr::Literal(l) => l,
                _ => unreachable!(),
            })
            .collect();

        match (name, lits.as_slice()) {
            // Operações com Inteiros
            ("+", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Int(a + b)),
            ("-", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Int(a - b)),
            ("*", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Int(a * b)),
            ("**", [TLiteral::Int(a), TLiteral::Int(b)]) => {
                if *b >= 0 {
                    Some(TLiteral::Int(a.pow(*b as u32)))
                } else {
                    None // Evita pânico por expoente negativo num literal inteiro
                }
            }
            ("/", [TLiteral::Int(a), TLiteral::Int(b)]) if *b != 0 => {
                Some(TLiteral::Float(*a as f64 / *b as f64))
            }
            ("//", [TLiteral::Int(a), TLiteral::Int(b)]) if *b != 0 => Some(TLiteral::Int(a / b)),
            ("mod", [TLiteral::Int(a), TLiteral::Int(b)]) if *b != 0 => Some(TLiteral::Int(a % b)),

            // Operações com Floats
            ("+", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Float(a + b)),
            ("-", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Float(a - b)),
            ("*", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Float(a * b)),
            ("**", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Float(a.powf(*b))),
            ("/", [TLiteral::Float(a), TLiteral::Float(b)]) if *b != 0.0 => {
                Some(TLiteral::Float(a / b))
            }
            ("//", [TLiteral::Float(a), TLiteral::Float(b)]) if *b != 0.0 => {
                Some(TLiteral::Int((*a / *b) as i64))
            }
            ("mod", [TLiteral::Float(a), TLiteral::Float(b)]) if *b != 0.0 => {
                Some(TLiteral::Float(a % b))
            }

            // Comparações de Inteiros
            ("=", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a == b)),
            ("!=", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a != b)),
            (">", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a > b)),
            (">=", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a >= b)),
            ("<", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a < b)),
            ("<=", [TLiteral::Int(a), TLiteral::Int(b)]) => Some(TLiteral::Bool(a <= b)),

            // Comparações de Floats
            ("=", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a == b)),
            ("!=", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a != b)),
            (">", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a > b)),
            (">=", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a >= b)),
            ("<", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a < b)),
            ("<=", [TLiteral::Float(a), TLiteral::Float(b)]) => Some(TLiteral::Bool(a <= b)),

            // Comparações de Strings
            ("=", [TLiteral::String(a), TLiteral::String(b)]) => Some(TLiteral::Bool(a == b)),
            ("!=", [TLiteral::String(a), TLiteral::String(b)]) => Some(TLiteral::Bool(a != b)),

            // Lógica Booleana
            ("and", [TLiteral::Bool(a), TLiteral::Bool(b)]) => Some(TLiteral::Bool(*a && *b)),
            ("or", [TLiteral::Bool(a), TLiteral::Bool(b)]) => Some(TLiteral::Bool(*a || *b)),
            ("xor", [TLiteral::Bool(a), TLiteral::Bool(b)]) => Some(TLiteral::Bool(*a ^ *b)),
            ("not", [TLiteral::Bool(a)]) => Some(TLiteral::Bool(!*a)),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::TypeRef;

    #[test]
    fn test_constant_folding_math() {
        let mut folder = ConstantFolder::new();
        
        let call = TExpr::Call(
            Box::new((TExpr::Ident("+".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![
                (TExpr::Literal(TLiteral::Int(10)), 0..0),
                (TExpr::Literal(TLiteral::Int(5)), 0..0),
            ],
            TypeRef::Simple("Int".to_string())
        );

        let folded = folder.fold_expr_spanned((call, 0..0));
        
        assert_eq!(folded.0, TExpr::Literal(TLiteral::Int(15)));
    }

    #[test]
    fn test_constant_folding_nested() {
        let mut folder = ConstantFolder::new();
        
        // + $(+ 2 3) $(* 4 5) ou + + 2 3 * 4 5
        let inner1 = (TExpr::Call(
            Box::new((TExpr::Ident("+".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![
                (TExpr::Literal(TLiteral::Int(2)), 0..0),
                (TExpr::Literal(TLiteral::Int(3)), 0..0),
            ],
            TypeRef::Simple("Int".to_string())
        ), 0..0);

        let inner2 = (TExpr::Call(
            Box::new((TExpr::Ident("*".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![
                (TExpr::Literal(TLiteral::Int(4)), 0..0),
                (TExpr::Literal(TLiteral::Int(5)), 0..0),
            ],
            TypeRef::Simple("Int".to_string())
        ), 0..0);

        let outer = TExpr::Call(
            Box::new((TExpr::Ident("+".to_string(), TypeRef::Simple("Unknown".to_string())), 0..0)),
            vec![inner1, inner2],
            TypeRef::Simple("Int".to_string())
        );

        let folded = folder.fold_expr_spanned((outer, 0..0));
        
        // 5 + 20 = 25
        assert_eq!(folded.0, TExpr::Literal(TLiteral::Int(25)));
    }

    #[test]
    fn test_constant_folding_guard_short_circuit() {
        let mut folder = ConstantFolder::new();
        
        let branches = vec![
            (
                (TExpr::Literal(TLiteral::Bool(false)), 0..0),
                (TExpr::Literal(TLiteral::Int(1)), 0..0)
            ),
            (
                (TExpr::Literal(TLiteral::Bool(true)), 0..0),
                (TExpr::Literal(TLiteral::Int(2)), 0..0) // Este deve ser o ramo sobrevivente!
            ),
            (
                (TExpr::Ident("condicao_incognita".into(), TypeRef::Simple("Bool".into())), 0..0),
                (TExpr::Literal(TLiteral::Int(3)), 0..0)
            ),
        ];

        let otherwise = (TExpr::Literal(TLiteral::Int(4)), 0..0);

        let guard = TExpr::Guard(branches, Box::new(otherwise), TypeRef::Simple("Int".to_string()));
        let folded = folder.fold_expr_spanned((guard, 0..0));
        
        // O `True` em 2 deve ter abortado o resto e devolvido diretamente o Int(2).
        assert_eq!(folded.0, TExpr::Literal(TLiteral::Int(2)));
    }
}
