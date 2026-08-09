//! Pipeline composicional do compilador Kata.
//!
//! Centraliza a sequência lex → parse → resolve → infer → monomorph →
//! optimize → tree-shake → comptime → type-table em uma única abstração,
//! eliminando a duplicação que existia entre `run_pipeline_with_file`
//! (JIT), `cmd_test` (test runner) e `cmd_build` (AOT).
//!
//! Cada passo existe uma vez. Os backends (JIT, test, AOT) só diferem no
//! codegen final — chamam `.jit_eval()`, `.jit_tests()` ou `.aot_emit()`
//! sobre o `CompiledModule` produzido pelo pipeline.

use std::collections::HashMap;

use kata_codegen::type_table;
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::{monomorphize, MonoModule};
use kata_optimizer::optimize;
use kata_parser::{parse, parse_decls_only, parse_with_arity, scan_lambdas};
use kata_resolution::{extract_arities, load_prelude, resolve_with_imports, ResolvedModule};
use kata_tree_shaking::{tree_shake, tree_shake_preserve_tests};

use crate::imports;
use crate::{merge_resolved, IntoReport};

// ── Erros ─────────────────────────────────────────────────

type PipelineResult<T> = miette::Result<T>;

fn err(msg: impl std::fmt::Display) -> miette::Report {
    miette::Report::msg(msg.to_string())
}

// ── Modos configuráveis ────────────────────────────────────

/// Modo de parsing.
///
/// `TwoPass` realiza o ciclo arity-uniformization (scan_lambdas →
/// parse_decls_only → extract_arities → parse_with_arity). `Single`
/// chama `parse` diretamente.
#[derive(Clone, Copy)]
pub enum ParseMode {
    TwoPass,
    Single,
}

/// Modo de tree-shaking.
///
/// `Default` remove funções/actions não alcançadas. `PreserveTests`
/// mantém `TypedTestSpec` para o test runner gerar wrappers.
#[derive(Clone, Copy)]
pub enum ShakeMode {
    Default,
    PreserveTests,
}

// ── Artefato final ──────────────────────────────────────────

/// Módulo compilado até o estágio de codegen (pós-comptime).
///
/// Contém o `MonoModule` e os metadados necessários para qualquer
/// backend de codegen (type_id_map, enum_registry, struct_registry).
pub struct CompiledModule {
    pub mono: MonoModule,
    pub type_id_map: HashMap<Ty, i64>,
}

impl CompiledModule {
    /// Codegen JIT — compila e executa o entry point, retornando o valor bruto.
    pub fn jit_eval(self) -> miette::Result<i64> {
        let result = kata_codegen::jit_eval(&self.mono, &self.type_id_map)
            .map_err(|e| err(format!("erro de codegen: {e}")))?;
        Ok(result.raw)
    }

    /// Codegen JIT para testes — compila wrappers `__kata_test_*`.
    pub fn jit_tests(
        self,
    ) -> miette::Result<(cranelift_jit::JITModule, Vec<kata_codegen::TestWrapper>)> {
        kata_codegen::jit_compile_tests(&self.mono, &self.type_id_map)
            .map_err(|e| err(format!("erro de codegen: {e}")))
    }

    /// Codegen AOT — emite object file (.o) bytes.
    pub fn aot_emit(self) -> miette::Result<Vec<u8>> {
        kata_codegen::aot_emit(&self.mono, &self.type_id_map)
            .map_err(|e| err(format!("erro de codegen AOT: {e}")))
    }

    /// Tipo canônico do entry point (para display e AOT type tag).
    pub fn entry_ty(&self) -> Ty {
        self.mono.entry.node.ty.clone()
    }
}

// ── Pipeline ───────────────────────────────────────────────

/// Pipeline composicional do compilador.
///
/// Carrega o contexto acumulativo (artefatos de cada fase). Cada método
/// consome `self` e produz o próximo estado, propagando erros via `?`.
///
/// Uso típico:
/// ```ignore
/// let compiled = Pipeline::new(source)
///     .parse(ParseMode::TwoPass)?
///     .resolve(Some(file))?
///     .desugar()
///     .infer()?
///     .monomorph()
///     .optimize()
///     .tree_shake(ShakeMode::Default)?
///     .comptime()?
///     .build_type_table()?;
///
/// let result = compiled.jit_eval()?;
/// ```
pub struct Pipeline {
    source: String,
    // Artefatos acumulados (preenchidos conforme avança):
    tokens: Option<Vec<kata_ast::TokenWithSpan>>,
    module: Option<kata_ast::Module>,
    resolved: Option<ResolvedModule>,
    typed: Option<kata_inference::TypedModule>,
    mono: Option<MonoModule>,
}

impl Pipeline {
    /// Cria o pipeline com o código-fonte.
    pub fn new(source: impl Into<String>) -> Self {
        Pipeline {
            source: source.into(),
            tokens: None,
            module: None,
            resolved: None,
            typed: None,
            mono: None,
        }
    }

    // ── Lex ─────────────────────────────────────────────

    /// Análise léxica.
    pub fn lex(mut self) -> PipelineResult<Self> {
        let tokens = lex(&self.source).map_err(IntoReport::into_report)?;
        self.tokens = Some(tokens);
        Ok(self)
    }

    // ── Parse ───────────────────────────────────────────

    /// Análise sintática.
    ///
    /// `TwoPass` realiza o ciclo arity-uniformization:
    ///   1. `scan_lambdas` (tokens) → aridades de `let f := lambda`
    ///   2. `parse_decls_only` → resolve → `extract_arities` (sobrescreve)
    ///   3. `parse_with_arity` (parse completo)
    ///
    /// `Single` chama `parse` diretamente.
    pub fn parse(mut self, mode: ParseMode, file_path: Option<&str>) -> PipelineResult<Self> {
        let tokens = self
            .tokens
            .take()
            .ok_or_else(|| err("parse chamado antes de lex"))?;

        let module = match mode {
            ParseMode::Single => parse(tokens).map_err(IntoReport::into_report)?,
            ParseMode::TwoPass => {
                let mut arities = scan_lambdas(&tokens);
                let decls_module =
                    parse_decls_only(tokens.clone()).map_err(IntoReport::into_report)?;
                let decls_resolved = quick_resolve(&decls_module, file_path)?;
                arities.extend(extract_arities(&decls_resolved.signatures));
                parse_with_arity(tokens, arities).map_err(IntoReport::into_report)?
            }
        };

        self.module = Some(module);
        Ok(self)
    }

    // ── Resolve ─────────────────────────────────────────

    /// Resolução: carrega prelude + imports e faz merge.
    ///
    /// `file_path` é `Some` para arquivos (carrega imports do sistema de
    /// módulos). `None` para eval (sem imports, só prelude).
    pub fn resolve(mut self, file_path: Option<&str>) -> PipelineResult<Self> {
        let module = self
            .module
            .as_ref()
            .ok_or_else(|| err("resolve chamado antes de parse"))?;

        let prelude = load_prelude()
            .map_err(|e| err(format!("erro ao carregar prelude: {}", format_err_vec(&e))))?;

        let imports = match file_path {
            Some(file) => imports::load_module_imports(file, module)?,
            None => Vec::new(),
        };
        let imported_directives = imports::collect_imported_directives(&imports);
        let user = resolve_with_imports(module, "__local__", imported_directives)
            .map_err(|e| err(format!("erro de resolução: {}", format_err_vec(&e))))?;
        let mut resolved = merge_resolved(prelude, user);
        imports::merge_imports(&mut resolved, &imports);

        self.resolved = Some(resolved);
        Ok(self)
    }

    // ── Desugar ─────────────────────────────────────────

    /// Desugar directives — inline bodies de diretivas customizadas
    /// antes do typeck, produzindo AST expandida.
    pub fn desugar(mut self) -> Self {
        if let Some(resolved) = self.resolved.as_mut() {
            kata_inference::desugar_directives::desugar_directives(resolved);
        }
        self
    }

    // ── Infer ───────────────────────────────────────────

    /// Type checking + dispatch.
    pub fn infer(mut self) -> PipelineResult<Self> {
        let module = self
            .module
            .as_ref()
            .ok_or_else(|| err("infer chamado antes de parse"))?;
        let resolved = self
            .resolved
            .as_ref()
            .ok_or_else(|| err("infer chamado antes de resolve"))?;

        let typed = infer_module(module, resolved).map_err(IntoReport::into_report)?;
        self.typed = Some(typed);
        Ok(self)
    }

    // ── Monomorph ───────────────────────────────────────

    /// Monomorfização — especializa call sites genéricos.
    pub fn monomorph(mut self) -> Self {
        let typed = self.typed.take().expect("monomorph chamado antes de infer");
        self.mono = Some(monomorphize(typed));
        self
    }

    // ── Optimize ────────────────────────────────────────

    /// Otimização (TRMA + futuros passes).
    pub fn optimize(mut self) -> Self {
        let mono = self
            .mono
            .take()
            .expect("optimize chamado antes de monomorph");
        self.mono = Some(optimize(mono));
        self
    }

    // ── Tree shake ──────────────────────────────────────

    /// Tree shaking — remove funções/actions não alcançadas.
    pub fn tree_shake(mut self, mode: ShakeMode) -> PipelineResult<Self> {
        let mono = self
            .mono
            .take()
            .ok_or_else(|| err("tree_shake chamado antes de monomorph"))?;

        let shaken = match mode {
            ShakeMode::Default => tree_shake(mono.inner),
            ShakeMode::PreserveTests => tree_shake_preserve_tests(mono.inner),
        };

        self.mono = Some(MonoModule::from(shaken));
        Ok(self)
    }

    // ── Comptime ────────────────────────────────────────

    /// Avalia expressões `@comptime` em compile-time e substitui por
    /// literais antes do codegen.
    pub fn comptime(mut self) -> PipelineResult<Self> {
        let mono = self
            .mono
            .take()
            .ok_or_else(|| err("comptime chamado antes de tree_shake"))?;
        let enum_registry = self
            .resolved
            .as_ref()
            .ok_or_else(|| err("comptime chamado antes de resolve"))?
            .enum_registry
            .clone();

        let shaken = run_comptime_pass(mono.inner, &enum_registry)
            .map_err(|e| err(format!("erro de comptime: {e}")))?;
        self.mono = Some(MonoModule::from(shaken));
        Ok(self)
    }

    // ── Type table ──────────────────────────────────────

    /// Registra TypeShapes no runtime para to_bytes/from_bytes.
    /// Devolve o `CompiledModule` pronto para codegen.
    pub fn build_type_table(self) -> PipelineResult<CompiledModule> {
        let mono = self
            .mono
            .ok_or_else(|| err("build_type_table chamado antes de comptime"))?;
        let enum_registry = self
            .resolved
            .as_ref()
            .ok_or_else(|| err("build_type_table chamado antes de resolve"))?
            .enum_registry
            .clone();

        let type_id_map =
            type_table::build_and_register_type_table(&mono, &mono.struct_registry, &enum_registry);

        Ok(CompiledModule { mono, type_id_map })
    }
}

// ── Helpers ────────────────────────────────────────────────

fn format_err_vec<E: std::fmt::Display>(errors: &[E]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Resolve rápido para o Pass 1 do TwoPass (sem desugar).
///
/// Carrega imports (para validar diretivas importadas no Pass 1) mas
/// não faz desugar — só precisa das assinaturas para extract_arities.
fn quick_resolve(
    module: &kata_ast::Module,
    file_path: Option<&str>,
) -> PipelineResult<ResolvedModule> {
    let prelude = load_prelude()
        .map_err(|e| err(format!("erro ao carregar prelude: {}", format_err_vec(&e))))?;

    let imports = match file_path {
        Some(file) => imports::load_module_imports(file, module)?,
        None => Vec::new(),
    };
    let imported_directives = imports::collect_imported_directives(&imports);

    let user = resolve_with_imports(module, "__local__", imported_directives).map_err(|e| {
        err(format!(
            "erro de resolução (Pass 1): {}",
            format_err_vec(&e)
        ))
    })?;
    let resolved = merge_resolved(prelude, user);
    Ok(resolved)
}
