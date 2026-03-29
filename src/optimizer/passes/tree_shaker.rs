use crate::type_checker::checker::TTopLevel;
use crate::type_checker::tast::{TExpr, TStmt, TMatchArm};
use crate::type_checker::environment::TypeEnv;
use crate::parser::ast::Spanned;
use std::collections::HashSet;

pub struct TreeShaker<'a> {
    pub reachable_functions: HashSet<String>,
    pub env: &'a TypeEnv,
}

impl<'a> TreeShaker<'a> {
    pub fn new(env: &'a TypeEnv) -> Self {
        Self {
            reachable_functions: HashSet::new(),
            env,
        }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Early Tree-Shaking...");
        
        // Fase 1: Identificar os pontos de entrada dinamicamente
        
        // Para Bibliotecas: Tudo o que for exportado é considerado raiz inatingível
        for export_name in &self.env.exports {
            self.reachable_functions.insert(export_name.clone());
        }

        for (decl, _) in &tast {
            if let TTopLevel::Execution(expr) = decl {
                // Para Executáveis: A execução de topo é a raiz
                self.visit_expr(expr);
            }
        }
        
        // Também devemos preservar structs, interfaces e enums para já,
        // concentrando a eliminação em funções/ações não chamadas.
        let mut progress = true;
        
        while progress {
            progress = false;
            let initial_size = self.reachable_functions.len();
            
            for (decl, _) in &tast {
                match decl {
                    TTopLevel::ActionDef(name, _, _, body, _) => {
                        if self.reachable_functions.contains(name) {
                            for stmt in body {
                                self.visit_stmt(stmt);
                            }
                        }
                    }
                    TTopLevel::LambdaDef(params, body, with, _) => {
                        // O nome não está na AST do LambdaDef (vem da Signature anterior),
                        // mas se este nó estiver ligado a uma assinatura alcançável,
                        // teríamos de o marcar. 
                        // Como a TAST separa a Signature do LambdaDef, a associação
                        // é feita posicionalmente ou pelo nome na Signature anterior.
                        // Para simplificar, o TypeChecker devia agrupar isso, mas aqui
                        // visitamos os lambdas apenas se a sua "posse" for alcançável.
                        // Numa versão inicial, marcamos todas as dependências internas.
                        self.visit_expr(body);
                        for w in with {
                            self.visit_expr(w);
                        }
                    }
                    _ => {}
                }
            }
            
            if self.reachable_functions.len() > initial_size {
                progress = true;
            }
        }

        let initial_nodes = tast.len();

        // Fase 2: Filtrar a TAST com base na alcançabilidade.
        // Precisamos rastrear a `Signature` atual para saber se os `LambdaDef`s subsequentes devem ser mantidos.
        let mut optimized = Vec::new();
        let mut current_sig_reachable = false;

        for (decl, span) in tast {
            let keep = match &decl {
                TTopLevel::Signature(name, _, _, _) => {
                    current_sig_reachable = self.reachable_functions.contains(name);
                    current_sig_reachable
                }
                TTopLevel::LambdaDef(..) => {
                    // Mantém o LambdaDef apenas se a assinatura imediatamente anterior for alcançável
                    current_sig_reachable
                }
                TTopLevel::ActionDef(name, _, _, _, _) => {
                    current_sig_reachable = false; // Quebra a associação
                    self.reachable_functions.contains(name)
                }
                TTopLevel::Execution(expr) => {
                    current_sig_reachable = false;
                    self.visit_expr(expr); // Marca de novo caso o late-shaker limpe
                    true // Sempre preserva a execução de topo
                }
                TTopLevel::Data(..) | TTopLevel::Enum(..) => {
                    current_sig_reachable = false;
                    // Por enquanto, Tipos e Enums são sempre preservados. 
                    true
                }
            };

            if keep {
                optimized.push((decl, span));
            } else if let TTopLevel::Signature(name, _, _, _) = &decl {
                log::debug!("Tree-Shaking: Removendo código morto/genérico: {}", name);
            }
        }

        log::debug!("Tree-Shaking concluído. (Iniciais: {}, Finais: {})", initial_nodes, optimized.len());
        optimized
    }

    fn visit_stmt(&mut self, stmt: &Spanned<TStmt>) {
        match &stmt.0 {
            TStmt::Let(_, expr) => self.visit_expr(expr),
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
            TStmt::Expr(expr) => self.visit_expr(expr),
            TStmt::Break | TStmt::Continue => {}
        }
    }

    fn visit_expr(&mut self, expr: &Spanned<TExpr>) {
        match &expr.0 {
            TExpr::Ident(name, _) => {
                self.reachable_functions.insert(name.clone());
            }
            TExpr::Call(callee, args, _) => {
                self.visit_expr(callee);
                for a in args {
                    self.visit_expr(a);
                }
            }
            TExpr::Tuple(exprs, _) | TExpr::List(exprs, _) | TExpr::Sequence(exprs, _) => {
                for e in exprs {
                    self.visit_expr(e);
                }
            }
            TExpr::Lambda(_, body, _) => {
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
