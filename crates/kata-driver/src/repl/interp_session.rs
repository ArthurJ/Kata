//! InterpReplSession — sessão REPL do interpretador tree-walking.
//!
//! Diferente do `ReplSession` (JIT), o `InterpReplSession`:
//! - Mantém `Env` persistente com bindings `let` acumulados (valores i64)
//! - Mantém Runtime persistente (arena Bump para valores compostos)
//! - Não congela bindings (não precisa — o Env persiste os valores)
//! - Não gerencia snapshots (não precisa — valores ficam na arena)
//! - Não persiste function pointers (funções são reavaliadas a cada linha)
//!
//! A cada linha: lex → parse → resolve → desugar → infer → monomorph →
//! optimize → interpret_with_env. O TypedModule é recompilado a cada linha
//! com todos os items acumulados, mas apenas o entry point é avaliado
//! (pre_entry bindings que já estão no Env são pulados).

use std::collections::HashSet;
use std::path::PathBuf;

use kata_ast::{Expr, Item, Module, Span, Spanned};
use kata_core::ty::Ty;
use kata_inference::{TypedExprKind, TypedModule, infer_module};
use kata_interp::Env;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::{parse_repl_decls_only, parse_repl_with_arity, scan_lambdas};
use kata_resolution::{ModuleLoader, ResolvedModule, extract_arities, resolve};

use crate::display;
use crate::merge_resolved;

/// Carrega a stdlib (core → core_internals) — mesma do ReplSession.
fn load_stdlib() -> Result<ResolvedModule, String> {
    let mut loader = ModuleLoader::new(Vec::new());
    let stdlib = loader
        .load(&["stdlib".into(), "core".into()], std::path::Path::new("."))
        .map_err(|e| format!("erro ao carregar stdlib: {e}"))?;
    Ok((*stdlib).clone())
}

pub(crate) struct InterpReplSession {
    /// Items top-level acumulados (let bindings, sigs, data, enum, etc.).
    items: Vec<Spanned<Item>>,
    /// Módulos importados via `import` no REPL — cacheados entre linhas.
    imports: Vec<kata_resolution::ImportedModule>,
    /// Prelude resolvido (recarregado em `:reset`).
    prelude: ResolvedModule,
    /// Runtime persistente — vive entre avaliações para preservar valores
    /// na arena Bump entre linhas.
    rt_ptr: i64,
    /// Env persistente — bindings `let` acumulados entre linhas.
    env: Env,
    /// Nomes de bindings já avaliados (para pular pre_entry reavaliação).
    evaluated_bindings: HashSet<String>,
    /// Histórico rustyline.
    pub(crate) history_path: PathBuf,
}

impl InterpReplSession {
    /// Cria nova sessão carregando o prelude.
    pub fn new() -> Result<Self, String> {
        let prelude = load_stdlib().map_err(|e| format!("erro ao carregar prelude: {e}"))?;
        let history_path = dirs();
        let rt = Box::new(kata_rt::Runtime::new());
        let rt_ptr = Box::into_raw(rt) as i64;
        Ok(Self {
            items: Vec::new(),
            imports: Vec::new(),
            prelude,
            rt_ptr,
            env: Env::new(),
            evaluated_bindings: HashSet::new(),
            history_path,
        })
    }

    /// Reseta a sessão — limpa items, env, recarrega prelude, recria Runtime.
    pub fn reset(&mut self) -> Result<(), String> {
        self.items.clear();
        self.imports.clear();
        self.evaluated_bindings.clear();
        self.prelude = load_stdlib().map_err(|e| format!("erro ao carregar prelude: {e}"))?;
        // Recriar Runtime — descarta o antigo e cria um novo limpo.
        let _ = unsafe { Box::from_raw(self.rt_ptr as *mut kata_rt::Runtime) };
        let rt = Box::new(kata_rt::Runtime::new());
        self.rt_ptr = Box::into_raw(rt) as i64;
        // Recriar Env
        self.env = Env::new();
        Ok(())
    }

    /// Processa um input: expressão ou comando `:`.
    /// Retorna `true` se deve continuar, `false` se deve sair (`:quit`).
    pub fn handle(&mut self, input: &str) -> Result<bool, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(true);
        }
        if let Some(rest) = trimmed.strip_prefix(':') {
            return self.handle_command(rest);
        }
        self.eval_expr(input)?;
        Ok(true)
    }

    /// Avalia uma expressão pelo pipeline completo (interpretador).
    fn eval_expr(&mut self, input: &str) -> Result<(), String> {
        let tokens = lex(input).map_err(|e| format!("erro léxico: {e}"))?;

        let mut arities = scan_lambdas(&tokens);

        // Pass 1: parse_decls_only → resolve → extract_arities
        let decls_module = parse_repl_decls_only(tokens.clone())
            .map_err(|e| format!("erro de parse (Pass 1): {e}"))?;
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
            parse_repl_with_arity(tokens, arities).map_err(|e| format!("erro de parse: {e}"))?;

        if module.items.is_empty() {
            return Ok(());
        }

        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));

        let snapshot_len = self.items.len();
        let snapshot_imports_len = self.imports.len();

        // Substituir `let` bindings redefinidos (igual ao JIT REPL).
        let new_let_names: HashSet<String> = module
            .items
            .iter()
            .filter_map(|i| {
                if let Item::EntryExpr(ref expr) = i.node
                    && let Expr::Let { ref name, .. } = expr.node
                {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        if !new_let_names.is_empty() {
            self.items.retain(|i| {
                if let Item::EntryExpr(ref expr) = i.node
                    && let Expr::Let { ref name, .. } = expr.node
                {
                    !new_let_names.contains(name)
                } else {
                    true
                }
            });
            // Remover do evaluated_bindings para reavaliar na próxima linha
            for name in &new_let_names {
                self.evaluated_bindings.remove(name);
            }
        }

        // Substituir constantes redefinidas (igual ao JIT REPL).
        let new_constant_names: HashSet<String> = module
            .items
            .iter()
            .filter_map(|i| {
                if let Item::ConstantDecl { name, .. } = &i.node {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        if !new_constant_names.is_empty() {
            self.items.retain(|i| {
                if let Item::ConstantDecl { name, .. } = &i.node {
                    !new_constant_names.contains(name)
                } else {
                    true
                }
            });
        }

        self.items.extend(module.items);

        // Carregar imports
        let import_module = Module {
            items: self.items.clone(),
        };
        let has_imports = import_module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::ImportDecl { .. }));
        if has_imports {
            self.imports = crate::imports::load_repl_imports(&import_module)
                .map_err(|e| format!("erro ao carregar imports: {e}"))?;
        }

        if !has_entry {
            // Apenas declarações — valida com typeck sem executar.
            let full_module = Module {
                items: self.items.clone(),
            };
            match self.run_pipeline_typed_for_decls(&full_module) {
                Ok(_) => Ok(()),
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    self.imports.truncate(snapshot_imports_len);
                    Err(e)
                }
            }
        } else {
            let full_module = Module {
                items: self.items.clone(),
            };
            match self.run_pipeline_interp(&full_module) {
                Ok(result) => {
                    // Suprimir Unit.
                    if !matches!(result.ty, Ty::Unit) {
                        display::print_result(result.raw, &result.ty);
                    }
                    // Remover EntryExpr que não são bindings — expressões
                    // puras são "avaliar e esquecer". Apenas Let persiste.
                    self.items.retain(|item| match &item.node {
                        Item::EntryExpr(expr) => {
                            matches!(expr.node, Expr::Let { .. } | Expr::LetDestruct { .. })
                        }
                        _ => true,
                    });
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    self.imports.truncate(snapshot_imports_len);
                    Err(e)
                }
            }
        }
    }

    /// Roda o pipeline até TypedModule (para validação de declarações).
    fn run_pipeline_typed(&self, module: &Module) -> Result<TypedModule, String> {
        let user = resolve(module)
            .map_err(|e| format!("erro de resolução: {}", crate::format_error_vec(&e)))?;
        let mut resolved = merge_resolved(self.prelude.clone(), user);
        if !self.imports.is_empty() {
            kata_resolution::merge_imports(&mut resolved, &self.imports);
        }
        let typed = infer_module(module, &resolved).map_err(|e| format!("erro de tipo: {e}"))?;
        Ok(typed)
    }

    /// Valida declarações sem entry point.
    fn run_pipeline_typed_for_decls(&self, module: &Module) -> Result<(), String> {
        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));
        let items = if has_entry {
            module.items.clone()
        } else {
            let mut items = module.items.clone();
            let zero = Expr::IntLit {
                text: "0".to_string(),
            };
            let spanned = Spanned::new(zero, Span::synthetic());
            items.push(Spanned::new(Item::EntryExpr(spanned), Span::synthetic()));
            items
        };
        let module = Module { items };
        self.run_pipeline_typed(&module)?;
        Ok(())
    }

    /// Roda o pipeline completo até interpretação.
    fn run_pipeline_interp(&mut self, module: &Module) -> Result<crate::ExecResult, String> {
        let user = resolve(module)
            .map_err(|e| format!("erro de resolução: {}", crate::format_error_vec(&e)))?;
        let mut resolved = merge_resolved(self.prelude.clone(), user);

        // Merge de imports cacheados + avaliação de constants importadas.
        let imported_constants = if !self.imports.is_empty() {
            kata_resolution::merge_imports(&mut resolved, &self.imports);
            let ics = crate::imports::evaluate_imported_constants(&self.imports)
                .map_err(|e| format!("erro ao avaliar constants importadas: {e}"))?;
            for ic in &ics {
                resolved
                    .type_env
                    .define(&ic.name, ic.value.ty.clone(), "__module__");
            }
            ics
        } else {
            Vec::new()
        };

        let typed = infer_module(module, &resolved).map_err(|e| format!("erro de tipo: {e}"))?;

        // Comptime pass: avalia constants.
        let mut typed =
            kata_comptime::run_comptime_pass(typed, &resolved.enum_registry, self.rt_ptr)
                .map_err(|e| format!("erro de comptime: {e}"))?;

        // Injetar constants importadas como ConstantBinding.
        for ic in imported_constants {
            let dummy_span = kata_ast::Span::zero();
            typed.constants.push(Spanned::new(
                kata_inference::TypedExpr {
                    span: dummy_span,
                    ty: ic.value.ty.clone(),
                    tail_pos: false,
                    escape: kata_core::escape::EscapeTarget::Local,
                    kind: TypedExprKind::ConstantBinding {
                        name: ic.name.clone(),
                        value: Box::new(Spanned::new(ic.value, dummy_span)),
                    },
                },
                dummy_span,
            ));
        }

        let mono = monomorphize(typed);
        let mono = optimize(mono);
        let mono = kata_monomorph::MonoModule::from(kata_tree_shaking::tree_shake(mono.inner));

        // Interpretar com Env persistente.
        let typed_module = mono.inner;
        let ty = typed_module.entry.node.ty.clone();
        // Extrair nomes de pre_entry antes de mover typed_module.
        let pre_entry_names: Vec<String> = typed_module
            .pre_entry
            .iter()
            .filter_map(|pre| {
                if let TypedExprKind::Let { name, .. } = &pre.node.kind {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        let raw = kata_interp::interpret_with_env(typed_module, self.rt_ptr, &mut self.env)
            .map_err(|e| format!("erro de interpretação: {e}"))?
            .raw;

        // Registrar bindings avaliados (pre_entry Let bindings).
        for name in &pre_entry_names {
            self.evaluated_bindings.insert(name.clone());
        }

        Ok(crate::ExecResult { raw, ty })
    }

    /// Tenta parsear o input e retorna true se está incompleto (erro de EOF).
    pub(crate) fn is_input_incomplete(input: &str) -> bool {
        let tokens = match lex(input) {
            Ok(t) => t,
            Err(_) => return false,
        };
        match kata_parser::parse(tokens) {
            Ok(_) => false,
            Err(e) => {
                let msg = format!("{e}");
                msg.contains("<EOF>") || msg.contains("<DEDENT>")
            }
        }
    }
}

// ── Comandos `:` ──────────────────────────────────────────

impl InterpReplSession {
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
        let temp_module = self.build_module_for_env();
        match self.run_pipeline_typed(&temp_module) {
            Ok(typed) => {
                let mut shown = false;
                for expr in &typed.pre_entry {
                    if let TypedExprKind::Let { name, value } = &expr.node.kind {
                        println!("  {name}: {}", value.node.ty);
                        shown = true;
                    }
                }
                if !shown {
                    println!("(nenhum binding)");
                }
            }
            Err(e) => {
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
            }
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
    fn cmd_load(&mut self, path: &str) -> Result<(), String> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("não foi possível ler `{path}`: {e}"))?;

        let tokens = lex(&source).map_err(|e| format!("erro léxico: {e}"))?;
        let mut arities = scan_lambdas(&tokens);

        let decls_module = parse_repl_decls_only(tokens.clone())
            .map_err(|e| format!("erro de parse (Pass 1): {e}"))?;
        let decls_user = resolve(&decls_module).map_err(|e| {
            format!(
                "erro de resolução (Pass 1): {}",
                crate::format_error_vec(&e)
            )
        })?;
        let decls_resolved = merge_resolved(self.prelude.clone(), decls_user);
        arities.extend(extract_arities(&decls_resolved.signatures));

        let module =
            parse_repl_with_arity(tokens, arities).map_err(|e| format!("erro de parse: {e}"))?;

        if module.items.is_empty() {
            eprintln!("arquivo `{path}` não contém items");
            return Ok(());
        }

        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));

        let snapshot_len = self.items.len();
        let snapshot_imports_len = self.imports.len();
        self.items.extend(module.items);

        let import_module = Module {
            items: self.items.clone(),
        };
        let has_imports = import_module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::ImportDecl { .. }));
        if has_imports {
            self.imports = crate::imports::load_repl_imports(&import_module).map_err(|e| {
                self.items.truncate(snapshot_len);
                format!("erro ao carregar imports: {e}")
            })?;
        }

        if !has_entry {
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
                    self.imports.truncate(snapshot_imports_len);
                    Err(e)
                }
            }
        } else {
            let full_module = Module {
                items: self.items.clone(),
            };
            match self.run_pipeline_interp(&full_module) {
                Ok(result) => {
                    if !matches!(result.ty, Ty::Unit) {
                        display::print_result(result.raw, &result.ty);
                    }
                    Ok(())
                }
                Err(e) => {
                    self.items.truncate(snapshot_len);
                    self.imports.truncate(snapshot_imports_len);
                    Err(e)
                }
            }
        }
    }

    /// Constrói Module com items acumulados + nova expressão.
    fn build_module(&self, expr_str: &str) -> Module {
        let mut items = self.items.clone();
        if !expr_str.is_empty() {
            let tokens = lex(expr_str).ok();
            if let Some(tokens) = tokens
                && let Ok(module) = kata_parser::parse(tokens)
            {
                items.extend(module.items);
            }
        }
        Module { items }
    }

    /// Constrói Module para `:env` — items acumulados + entry sintético `0`.
    fn build_module_for_env(&self) -> Module {
        let mut items = self.items.clone();
        let needs_entry = match items.last() {
            None => true,
            Some(item) => matches!(&item.node, Item::EntryExpr(_)),
        };
        if needs_entry {
            let zero = Expr::IntLit {
                text: "0".to_string(),
            };
            let spanned = Spanned::new(zero, Span::synthetic());
            items.push(Spanned::new(Item::EntryExpr(spanned), Span::synthetic()));
        }
        Module { items }
    }
}

/// Caminho do arquivo de histórico do REPL.
fn dirs() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kata_repl_history")
}
