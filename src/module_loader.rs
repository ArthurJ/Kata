use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::{lexer, parser, type_checker};
use crate::type_checker::environment::TypeEnv;
use crate::parser::ast::TopLevel;

pub struct ModuleLoader {
    pub search_paths: Vec<PathBuf>,
    pub cache: HashMap<String, TypeEnv>,
    pub tast_cache: HashMap<String, Vec<crate::parser::ast::Spanned<crate::type_checker::checker::TTopLevel>>>,
}

impl ModuleLoader {
    pub fn new(search_paths: Vec<PathBuf>) -> Self {
        Self {
            search_paths,
            cache: HashMap::new(),
            tast_cache: HashMap::new(),
        }
    }

    /// Tenta resolver, analisar e compilar um modulo por nome (ex: `core.types` ou `utils.math`)
    pub fn load_module(&mut self, module_path: &str, current_dir: Option<&Path>) -> Result<TypeEnv, String> {
        if let Some(env) = self.cache.get(module_path) {
            return Ok(env.clone()); // Ja esta no cache, previne ciclos de imports infinitos
        }

        log::debug!("Loading module: {} (from {:?})", module_path, current_dir);

        let file_path = self.resolve_path(module_path, current_dir)
            .map_err(|_| format!("Falha: Modulo `{}` nao encontrado no File System.", module_path))?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Falha ao ler o arquivo {}: {}", file_path.display(), e))?;

        let tokens = lexer::lex(&source, lexer::LexMode::File)
            .map_err(|_| format!("Falha na analise lexica do modulo: {}", module_path))?;
        
        let source_len = source.chars().count();
        let module_ast = parser::parse_module(tokens, source_len)
            .map_err(|_| format!("Falha na analise sintatica do modulo: {}", module_path))?;

        let new_current_dir = file_path.parent();

        // 1. Processar e resolver dependencias (Imports) DENTRO deste modulo antes de checa-lo
        for (decl, _) in &module_ast.declarations {
            if let TopLevel::Import(path, _) = decl {
                self.load_module(path, new_current_dir)?;
            }
        }

        // 2. Type Checking e TAST Generation deste modulo
        let mut checker = type_checker::Checker::new();
        checker.compiled_modules = self.cache.clone();
        
        for (decl, _) in &module_ast.declarations {
            if let TopLevel::Import(path, specific) = decl {
                if let Some(target_module_name) = path.split('.').next() {
                    if let Some(target_env) = checker.compiled_modules.get(path) { // Usamos o path exato como chave do cache
                        checker.env.import_from(target_env, target_module_name, specific);
                    } else if let Some(target_env) = checker.compiled_modules.get(target_module_name) {
                        checker.env.import_from(target_env, target_module_name, specific);
                    }
                }
            }
        }

        checker.check_module(&module_ast);

        if !checker.errors.is_empty() {
            let mut err_msg = format!("Erros semanticos no modulo {}:\n", module_path);
            for e in &checker.errors {
                err_msg.push_str(&format!("  - {}\n", e.0));
            }
            return Err(err_msg);
        }

        self.cache.insert(module_path.to_string(), checker.env.clone());
        self.tast_cache.insert(module_path.to_string(), checker.tast.clone());

        for (k, v) in checker.compiled_modules {
            self.cache.insert(k, v);
        }

        Ok(checker.env)
    }

    /// Resolve o padrao de string (core.types) para um caminho real no disco (core/types.kata ou core/types/mod.kata)
    fn resolve_path(&self, module_path: &str, current_dir: Option<&Path>) -> Result<PathBuf, ()> {
        let relative_path = module_path.replace('.', std::path::MAIN_SEPARATOR_STR);
        
        let mut all_paths = Vec::new();
        
        // Se temos o diretório de quem chamou o import, prioriza a busca relativa a ele
        if let Some(dir) = current_dir {
            all_paths.push(dir.to_path_buf());
        }
        all_paths.extend(self.search_paths.clone());

        for search_path in &all_paths {
            // Check a/b/c.kata
            let file_kata = search_path.join(format!("{}.kata", relative_path));
            if file_kata.is_file() {
                return Ok(file_kata);
            }

            // Check a/b/c/mod.kata (o padrao de Agregador de Diretorio)
            let mod_kata = search_path.join(&relative_path).join("mod.kata");
            if mod_kata.is_file() {
                return Ok(mod_kata);
            }
        }

        Err(())
    }
}
