//! Entry points de comandos `:` do REPL — `:help`, `:type`, `:env`, `:load`.

use super::ReplSession;
use crate::display;
use crate::merge_resolved;
use kata_ast::{Expr, Item, Module};
use kata_core::ty::Ty;
use kata_lexer::lex;
use kata_parser::{parse_decls_only, parse_with_arity, scan_lambdas};
use kata_resolution::{extract_arities, resolve};

impl ReplSession {
    /// Processa um comando `:`.
    pub(crate) fn handle_command(&mut self, rest: &str) -> Result<bool, String> {
        let (cmd, arg) = match rest.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (rest, ""),
        };
        match cmd {
            "quit" | "exit" => Ok(false),
            "help" => {
                self.cmd_help();
                Ok(true)
            }
            "reset" => {
                self.reset()?;
                println!("sessão resetada — prelude recarregado");
                Ok(true)
            }
            "env" => {
                self.cmd_env();
                Ok(true)
            }
            "type" => {
                if arg.is_empty() {
                    eprintln!("uso: :type <expr>");
                } else {
                    self.cmd_type(arg)?;
                }
                Ok(true)
            }
            "load" => {
                if arg.is_empty() {
                    eprintln!("uso: :load <file>");
                } else {
                    self.cmd_load(arg)?;
                }
                Ok(true)
            }
            _ => {
                eprintln!("comando desconhecido: :{cmd} (use :help)");
                Ok(true)
            }
        }
    }

    /// `:help` — lista comandos disponíveis.
    fn cmd_help(&self) {
        println!("comandos:");
        println!("  :help          mostra esta mensagem");
        println!("  :type <expr>   mostra o tipo de <expr> sem executar");
        println!("  :env           mostra bindings e tipos no TypeEnv atual");
        println!("  :load <file>   carrega arquivo .kata (items entram no env)");
        println!("  :reset         limpa bindings, recarrega prelude");
        println!("  :quit          sai do REPL");
    }

    /// `:env` — mostra bindings do TypeEnv atual.
    fn cmd_env(&self) {
        // Roda o pipeline até TypedModule para obter tipos dos bindings.
        // `build_module_for_env` injeta `0` (IntLit) como entry sintético
        // quando necessário, para que `let`s virem `pre_entry` (e não entry).
        let temp_module = self.build_module_for_env();
        let typed = match self.run_pipeline_typed(&temp_module) {
            Ok(t) => t,
            Err(e) => {
                // Fallback: listar nomes sem tipos.
                let mut shown = false;
                for item in &self.items {
                    if let Item::EntryExpr(expr) = &item.node
                        && let Expr::Let { name, .. } = &expr.node
                    {
                        println!("  {name}");
                        shown = true;
                    }
                }
                if !shown {
                    println!("(nenhum binding)");
                }
                eprintln!("{e}");
                return;
            }
        };

        let mut shown = false;
        for expr in &typed.pre_entry {
            if let kata_inference::TypedExprKind::Let { name, value } = &expr.node.kind {
                println!("  {name}: {}", value.node.ty);
                shown = true;
            }
        }
        if !shown {
            println!("(nenhum binding)");
        }
    }

    /// `:type <expr>` — infere e mostra o tipo sem executar.
    fn cmd_type(&self, expr_str: &str) -> Result<(), String> {
        let temp_module = self.build_module(expr_str);
        let typed = self.run_pipeline_typed(&temp_module)?;
        println!("{}", typed.entry.node.ty);
        Ok(())
    }

    /// `:load <file>` — carrega arquivo .kata, adiciona items à lista.
    ///
    /// Processa o arquivo como um todo: lex → parse → adicionar todos os
    /// items. Se houver EntryExpr, executa e mostra o resultado. Se não
    /// houver (apenas declarações), apenas adiciona à lista. Rollback
    /// em caso de erro.
    fn cmd_load(&mut self, path: &str) -> Result<(), String> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("não foi possível ler `{path}`: {e}"))?;

        let tokens = lex(&source).map_err(|e| format!("erro léxico: {e}"))?;

        // Pass 0: scan_lambdas — aridades de `let f := lambda ...`
        let mut arities = scan_lambdas(&tokens);

        // Pass 1: parse_decls_only → resolve → extract_arities
        // Signatures definem a aridade padrão; lambdas com mesmo nome são
        // overloads non-default.
        let decls_module =
            parse_decls_only(tokens.clone()).map_err(|e| format!("erro de parse (Pass 1): {e}"))?;
        let decls_user = resolve(&decls_module).map_err(|e| {
            format!(
                "erro de resolução (Pass 1): {}",
                crate::format_error_vec(&e)
            )
        })?;
        let decls_resolved = merge_resolved(self.prelude.clone(), decls_user);
        arities.extend(extract_arities(&decls_resolved.signatures));

        // Pass 2: parse_with_arity (completo)
        let module =
            parse_with_arity(tokens, arities).map_err(|e| format!("erro de parse: {e}"))?;

        if module.items.is_empty() {
            eprintln!("arquivo `{path}` não contém items");
            return Ok(());
        }

        // Verifica se há EntryExpr no arquivo.
        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));

        // Snapshot para rollback em caso de erro.
        let snapshot_len = self.items.len();
        self.items.extend(module.items);

        if !has_entry {
            // Apenas declarações — valida com typeck sem executar.
            let full_module = Module {
                items: self.items.clone(),
            };
            match self.run_pipeline_typed_for_decls(&full_module) {
                Ok(_) => {
                    eprintln!("carregado: {path}");
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    Err(e)
                }
            }
        } else {
            // Constrói Module com todos os items acumulados (incluindo os do arquivo).
            let full_module = Module {
                items: self.items.clone(),
            };

            match self.run_pipeline_eval(&full_module) {
                Ok(result) => {
                    if !matches!(result.ty, Ty::Unit) {
                        display::print_result(result.raw, &result.ty);
                    }
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    Err(e)
                }
            }
        }
    }
}
