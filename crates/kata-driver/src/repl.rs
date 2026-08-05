//! REPL interativo — `kata repl`.
//!
//! Mantém `Vec<Spanned<Item>>` acumulados entre expressões. A cada input:
//! lex → parse → adiciona item à lista → resolve + merge + infer_module →
//! monomorphize → optimize → jit_eval → display.
//!
//! Se a expressão é um `let`, o binding persiste porque o item fica na lista
//! e é re-processado na próxima iteração. O `TypeEnv` não é mutado diretamente
//! — a persistência é estrutural (items acumulados).
//!
//! Erros não abortam a sessão: o item adicionado é removido (rollback) e o
//! usuário pode corrigir e reintentar.

use std::path::PathBuf;

use kata_ast::{Expr, Item, Module, Spanned};
use kata_codegen::jit_eval;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_tree_shaking::tree_shake;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;

use crate::display;
use crate::highlight::KataHelper;
use crate::merge_resolved;

/// Sessão REPL — acumula items do usuário entre expressões.
pub(crate) struct ReplSession {
    /// Items top-level acumulados (let bindings, sigs, data, enum, etc.).
    items: Vec<Spanned<Item>>,
    /// Prelude resolvido (recarregado em `:reset`).
    prelude: ResolvedModule,
    /// Histórico rustyline.
    history_path: PathBuf,
}

impl ReplSession {
    /// Cria nova sessão carregando o prelude.
    pub fn new() -> Result<Self, String> {
        let prelude = load_prelude().map_err(|e| format!("erro ao carregar prelude: {e:?}"))?;
        let history_path = dirs();
        Ok(Self {
            items: Vec::new(),
            prelude,
            history_path,
        })
    }

    /// Reseta a sessão — limpa items e recarrega prelude.
    pub fn reset(&mut self) -> Result<(), String> {
        self.items.clear();
        self.prelude = load_prelude().map_err(|e| format!("erro ao carregar prelude: {e:?}"))?;
        Ok(())
    }

    /// Processa um input: expressão ou comando `:`.
    /// Retorna `true` se deve continuar, `false` se deve sair (`:quit`).
    pub fn handle(&mut self, input: &str) -> Result<bool, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(true);
        }

        // Comandos `:`
        if let Some(rest) = trimmed.strip_prefix(':') {
            return self.handle_command(rest);
        }

        // Expressão — processa pelo pipeline completo.
        self.eval_expr(input)?;
        Ok(true)
    }

    /// Processa um comando `:`.
    fn handle_command(&mut self, rest: &str) -> Result<bool, String> {
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

    /// Avalia uma expressão pelo pipeline completo.
    ///
    /// Se o input contém apenas declarações (Sig, DataDecl, EnumDecl, etc.)
    /// sem EntryExpr, apenas adiciona à lista — não executa. Se contém
    /// EntryExpr, executa o pipeline completo.
    fn eval_expr(&mut self, input: &str) -> Result<(), String> {
        // Parse para descobrir o(s) item(s).
        let tokens = lex(input).map_err(|e| format!("erro léxico: {e:?}"))?;
        let module = parse(tokens).map_err(|e| format!("erro de parse: {e:?}"))?;

        if module.items.is_empty() {
            return Ok(());
        }

        // Verifica se há EntryExpr no input.
        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));

        // Snapshot para rollback em caso de erro.
        let snapshot_len = self.items.len();
        self.items.extend(module.items);

        if !has_entry {
            // Apenas declarações — não executa, apenas adiciona à lista.
            // Valida com pipeline_typed para verificar consistência.
            let full_module = Module {
                items: self.items.clone(),
            };
            match self.run_pipeline_typed_for_decls(&full_module) {
                Ok(_) => Ok(()),
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    Err(e)
                }
            }
        } else {
            // Constrói Module com todos os items acumulados.
            let full_module = Module {
                items: self.items.clone(),
            };

            match self.run_pipeline_eval(&full_module) {
                Ok(result) => {
                    display::print_result(result.raw, &result.ty);
                    // Remove EntryExpr que não são bindings — expressões
                    // puras (ex: `5`, `echo!(5)`, `g 5`) são "avaliar e
                    // esquecer". Apenas `Let`/`LetDestruct` persistem.
                    self.items.retain(|item| {
                        match &item.node {
                            Item::EntryExpr(expr) => {
                                matches!(expr.node, Expr::Let { .. } | Expr::LetDestruct { .. })
                            }
                            _ => true,
                        }
                    });
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    Err(e)
                }
            }
        }
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

        let tokens = lex(&source).map_err(|e| format!("erro léxico: {e:?}"))?;
        let module = parse(tokens).map_err(|e| format!("erro de parse: {e:?}"))?;

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
                    display::print_result(result.raw, &result.ty);
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    Err(e)
                }
            }
        }
    }

    /// Constrói um Module temporário com items acumulados + nova expressão.
    fn build_module(&self, expr_str: &str) -> Module {
        let mut items = self.items.clone();
        if !expr_str.is_empty() {
            let tokens = lex(expr_str).ok();
            if let Some(tokens) = tokens
                && let Ok(module) = parse(tokens)
            {
                items.extend(module.items);
            }
        }
        Module { items }
    }

    /// Constrói Module para `:env` — items acumulados + entry sintético `0`.
    ///
    /// Se há apenas `let` bindings sem entry point, `infer_module` falha.
    /// Injetamos `0` (IntLit) como entry para satisfazer o pipeline.
    /// O tipo do entry não importa — só queremos os `pre_entry`.
    fn build_module_for_env(&self) -> Module {
        let mut items = self.items.clone();
        // Sempre adiciona entry sintético `0` se o último item é EntryExpr
        // (para que lets virem pre_entry, não entry) ou se não há entry.
        let needs_entry = match items.last() {
            None => true,
            Some(item) => matches!(&item.node, Item::EntryExpr(_)),
        };
        if needs_entry {
            let zero = kata_ast::Expr::IntLit {
                text: "0".to_string(),
            };
            let spanned = Spanned::new(zero, kata_ast::Span::synthetic());
            items.push(Spanned::new(
                Item::EntryExpr(spanned),
                kata_ast::Span::synthetic(),
            ));
        }
        Module { items }
    }

    /// Roda o pipeline até TypedModule (para `:type`).
    fn run_pipeline_typed(&self, module: &Module) -> Result<kata_inference::TypedModule, String> {
        let user = resolve(module).map_err(|e| format!("erro de resolução: {e:?}"))?;
        let resolved = merge_resolved(self.prelude.clone(), user);
        let typed = infer_module(module, &resolved).map_err(|e| format!("erro de tipo: {e:?}"))?;
        Ok(typed)
    }

    /// Valida declarações sem entry point (para `eval_expr` sem EntryExpr).
    ///
    /// Como `infer_module` exige um entry point, injeta `0` (IntLit) sintético
    /// se o module não tem EntryExpr. Não executa — só valida typeck.
    fn run_pipeline_typed_for_decls(&self, module: &Module) -> Result<(), String> {
        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));
        let module = if has_entry {
            module.clone()
        } else {
            let mut items = module.items.clone();
            let zero = kata_ast::Expr::IntLit {
                text: "0".to_string(),
            };
            let spanned = Spanned::new(zero, kata_ast::Span::synthetic());
            items.push(Spanned::new(
                Item::EntryExpr(spanned),
                kata_ast::Span::synthetic(),
            ));
            Module { items }
        };
        self.run_pipeline_typed(&module)?;
        Ok(())
    }

    /// Roda o pipeline completo até execução JIT.
    fn run_pipeline_eval(&self, module: &Module) -> Result<crate::ExecResult, String> {
        let user = resolve(module).map_err(|e| format!("erro de resolução: {e:?}"))?;
        let resolved = merge_resolved(self.prelude.clone(), user);
        let typed = infer_module(module, &resolved).map_err(|e| format!("erro de tipo: {e:?}"))?;
        let mono = monomorphize(typed);
        let mono = optimize(mono);
        let mono = kata_monomorph::MonoModule::from(tree_shake(mono.inner));
        let jit =
            jit_eval(&mono, &Default::default()).map_err(|e| format!("erro de codegen: {e:?}"))?;
        Ok(crate::ExecResult {
            raw: jit.raw,
            ty: jit.ty,
        })
    }
}

impl ReplSession {
    /// Tenta parsear o input e retorna true se está incompleto (erro de EOF).
    ///
    /// Usado pelo loop principal para decidir se deve continuar lendo
    /// linhas (multiline). Se o parse falha com "encontrado `<EOF>`",
    /// o input precisa de mais linhas para ser completo.
    fn is_input_incomplete(input: &str) -> bool {
        let tokens = match lex(input) {
            Ok(t) => t,
            Err(_) => return false,
        };
        match parse(tokens) {
            Ok(_) => false,
            Err(e) => format!("{e:?}").contains("<EOF>"),
        }
    }
}

/// Caminho do arquivo de histórico do REPL.
fn dirs() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kata_repl_history")
}

/// Executa o subcomando `kata repl`.
pub(crate) fn cmd_repl() -> miette::Result<()> {
    let mut session = ReplSession::new().map_err(miette::Report::msg)?;

    // Configurar rustyline com cores forçadas.
    //
    // ColorMode::Forced garante colorização mesmo em terminais que não
    // reportam capacidade de cor. O eco duplo que víamos antes era
    // causado pelo HistoryHinter (sugestões inline com ANSI), não pelo
    // ColorMode — agora o hinter retorna None.
    let config = rustyline::config::Builder::new()
        .color_mode(rustyline::config::ColorMode::Forced)
        .build();
    let mut rl = Editor::<KataHelper, DefaultHistory>::with_config(config)
        .map_err(|e| miette::Report::msg(format!("erro ao iniciar rustyline: {e}")))?;
    rl.set_helper(Some(KataHelper::default()));

    // Carregar histórico.
    let _ = rl.load_history(&session.history_path);

    println!("Kata REPL — digite :help para comandos, :quit para sair");

    loop {
        // Lê a primeira linha.
        let first = match rl.readline("kata> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("erro de leitura: {e}");
                break;
            }
        };

        // Comandos `:` são processados imediatamente (sem multiline).
        let trimmed = first.trim();
        if trimmed.starts_with(':') || trimmed.is_empty() {
            let _ = rl.add_history_entry(&first);
            match session.handle(&first) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => eprintln!("{e}"),
            }
            continue;
        }
        // Expressão: acumula linhas até o input ser completo.
        // Heurística multiline:
        //   1. Se o parse falha com "<EOF>", o input está incompleto —
        //      continuar lendo (ex: `lambda n:`, `match True`).
        //   2. Se a primeira linha é uma assinatura de função (`nome :: ... => T`)
        //      sem `@ffi`, ativar modo multiline — acumular até linha em
        //      branco (cláusulas lambda indentadas seguem).
        //   3. Se a primeira linha termina com `=>` (action sem tipo de
        //      retorno), body indentado pode seguir.
        //   4. Se a primeira linha inicia um bloco indentado — `match`,
        //      `enum`, `implements` — ativar modo multiline (break on
        //      non-indented line), igual à Sig.
        let first_trimmed = first.trim_end();
        let multiline_sig = first_trimmed.contains("::")
            && first_trimmed.contains("=>")
            && !first_trimmed.contains("@ffi");
        let multiline_action = first_trimmed.ends_with("=>");
        let first_token = first_trimmed.split_whitespace().next().unwrap_or("");
        let multiline_indent = matches!(first_token, "match" | "enum" | "implements" | "interface");

        let mut buffer = first.clone();
        let in_multiline = multiline_sig || multiline_action || multiline_indent;

        loop {
            if !in_multiline {
                // Verifica se o parse falha com <EOF> (input incompleto).
                if !ReplSession::is_input_incomplete(&buffer) {
                    break;
                }
            }

            // Lê próxima linha com prompt de continuação.
            match rl.readline("   ... ") {
                Ok(line) => {
                    if line.trim().is_empty() {
                        // Linha vazia termina o bloco multiline.
                        break;
                    }
                    buffer.push('\n');
                    buffer.push_str(&line);
                    // Se estávamos em modo multiline_sig ou multiline_indent
                    // e a nova linha não é indentada nem começa com
                    // `lambda`/`λ`, o bloco terminou.
                    if (multiline_sig || multiline_indent)
                        && !line.starts_with(' ')
                        && !line.starts_with('\t')
                        && !line.trim_start().starts_with("lambda")
                        && !line.trim_start().starts_with("λ")
                    {
                        break;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    buffer.clear();
                    break;
                }
                Err(ReadlineError::Eof) => {
                    break;
                }
                Err(e) => {
                    eprintln!("erro de leitura: {e}");
                    break;
                }
            }
        }

        let _ = rl.add_history_entry(&buffer);
        match session.handle(&buffer) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => eprintln!("{e}"),
        }
    }

    // Salvar histórico.
    let _ = rl.save_history(&session.history_path);
    Ok(())
}
