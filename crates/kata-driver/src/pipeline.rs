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
//!
//! ## Erros múltiplos
//!
//! O pipeline retorna `Result<T, Vec<Report>>` — `Vec` permite reportar
//! múltiplos erros de uma mesma fase (ex: parse com recovery) em vez de
//! abortar no primeiro. Cada fase é all-or-nothing: se parse tem erros,
//! o pipeline para e o driver imprime todos os erros de parse antes de
//! abortar. Não há continuação parcial entre fases — um module parcialmente
//! parseado produziria erros em cascata em resolve/infer.

use std::collections::HashMap;

use kata_codegen::type_table;
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex_with_recovery;
use kata_monomorph::{MonoModule, monomorphize};
use kata_optimizer::optimize;
use kata_parser::{parse_decls_only, parse_with_arity_recovery, parse_with_recovery, scan_lambdas};
use kata_resolution::{ResolvedModule, extract_arities, load_prelude, resolve_with_prelude};
use kata_tree_shaking::{tree_shake, tree_shake_preserve_tests};

use crate::imports;
use crate::{IntoReport, merge_resolved};

// ── Erros ─────────────────────────────────────────────────

/// Resultado de cada passo do pipeline. O erro é `Vec<Report>` para
/// permitir múltiplos erros por fase (principalmente parse com recovery).
type PipelineResult<T> = Result<T, Vec<miette::Report>>;

/// Cria um `Vec<Report>` com um único erro.
fn one_err(report: miette::Report) -> Vec<miette::Report> {
    vec![report]
}

/// Cria um `Vec<Report>` a partir de uma mensagem simples.
fn err(msg: impl std::fmt::Display) -> Vec<miette::Report> {
    one_err(miette::Report::msg(msg.to_string()))
}

// ── Modos configuráveis ────────────────────────────────────

/// Modo de parsing.
///
/// `TwoPass` realiza o ciclo arity-uniformization (scan_lambdas →
/// parse_decls_only → extract_arities → parse_with_arity_recovery). `Single`
/// chama `parse_with_recovery` diretamente.
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
    /// A2: TypeShapes para registrar no Runtime antes da execução.
    pub type_shapes: Vec<kata_rt::TypeShape>,
    source: String,
    file_path: Option<String>,
}

impl CompiledModule {
    /// Codegen JIT — compila e executa o entry point, retornando o valor bruto.
    pub fn jit_eval(self) -> miette::Result<i64> {
        // Runtime lifecycle: criar, executar, droppar. Valores retornados
        // que são ponteiros para a arena Bump são consumidos pelo display
        // antes do drop (o raw é lido imediatamente neste escopo).
        let rt = Box::new(kata_rt::Runtime::new());
        let rt_ptr = Box::into_raw(rt) as i64;
        let result =
            kata_codegen::jit_eval(&self.mono, &self.type_id_map, &self.type_shapes, rt_ptr)
                .map_err(|e| e.into_report_with_source(&self.source, self.file_path.as_deref()))?;
        // Droppar o Runtime após consumir o resultado. Se o valor retornado
        // é um ponteiro para arena (List, Struct), o display já aconteceu
        // ou o raw já foi lido. Para segurança, leak como antes se necessário.
        // NOTA: o driver `kata run` é efêmero — o leak é aceitável aqui também.
        std::mem::forget(unsafe { Box::from_raw(rt_ptr as *mut kata_rt::Runtime) });
        Ok(result.raw)
    }

    /// Codegen JIT para testes — compila wrappers `__kata_test_*`.
    pub fn jit_tests(
        self,
    ) -> miette::Result<(cranelift_jit::JITModule, Vec<kata_codegen::TestWrapper>)> {
        kata_codegen::jit_compile_tests(&self.mono, &self.type_id_map)
            .map_err(|e| e.into_report_with_source(&self.source, self.file_path.as_deref()))
    }

    /// Codegen AOT — emite object file (.o) bytes.
    pub fn aot_emit(self) -> miette::Result<Vec<u8>> {
        kata_codegen::aot_emit(&self.mono, &self.type_id_map)
            .map_err(|e| e.into_report_with_source(&self.source, self.file_path.as_deref()))
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
    /// Path do arquivo (para NamedSource no source context dos erros).
    /// `None` para eval/REPL.
    file_path: Option<String>,
    /// Se true, envolve o entry point com `show` após inference para que
    /// o driver possa imprimir tipos compostos (List, Tuple, Struct, Sum)
    /// como Text. Só ativado por `cmd_eval`/`cmd_run` — testes E2E e REPL
    /// não ativam (preservam o tipo original do entry point).
    display_wrap: bool,
    // Artefatos acumulados (preenchidos conforme avança):
    tokens: Option<Vec<kata_ast::TokenWithSpan>>,
    module: Option<kata_ast::Module>,
    resolved: Option<ResolvedModule>,
    /// Módulos importados (carregados em `resolve`, usados em `infer`
    /// para extrair constants exportadas e injetar no TypedModule).
    imports: Vec<kata_resolution::ImportedModule>,
    typed: Option<kata_inference::TypedModule>,
    mono: Option<MonoModule>,
}

impl Pipeline {
    /// Cria o pipeline com o código-fonte.
    pub fn new(source: impl Into<String>) -> Self {
        Pipeline {
            source: source.into(),
            file_path: None,
            display_wrap: false,
            tokens: None,
            module: None,
            resolved: None,
            imports: Vec::new(),
            typed: None,
            mono: None,
        }
    }

    /// Ativa display wrapping do entry point com `show`.
    ///
    /// Faz o entry point retornar `Text` em vez do tipo composto
    /// (List, Tuple, Struct, Sum), para que o driver possa imprimir
    /// via `TYPE_TEXT`. Primitivos não são afetados.
    pub fn with_display_wrap(mut self) -> Self {
        self.display_wrap = true;
        self
    }

    /// Define o path do arquivo para source context nos erros.
    ///
    /// Deve ser chamado antes de `lex()` para que erros léxicos também
    /// tenham o nome do arquivo. Se omitido, usa `<eval>`.
    pub fn with_file_path(mut self, file: &str) -> Self {
        self.file_path = Some(file.to_string());
        self
    }

    // ── Lex ─────────────────────────────────────────────

    /// Análise léxica com error recovery.
    ///
    /// Usa [`lex_with_recovery`] para reportar múltiplos erros léxicos
    /// em uma única passada. Se há erros, retorna `Err(Vec<Report>)`
    /// com todos os erros — o pipeline para e o driver imprime todos
    /// antes de abortar (all-or-nothing por fase).
    pub fn lex(mut self) -> PipelineResult<Self> {
        let (tokens, lex_errors) = lex_with_recovery(&self.source);
        if !lex_errors.is_empty() {
            return Err(lex_errors
                .into_iter()
                .map(|e| e.into_report_with_source(&self.source, self.file_path.as_deref()))
                .collect());
        }
        self.tokens = Some(tokens);
        Ok(self)
    }

    // ── Parse ───────────────────────────────────────────

    /// Análise sintática com error recovery.
    ///
    /// `TwoPass` realiza o ciclo arity-uniformization:
    ///   1. `scan_lambdas` (tokens) → aridades de `let f := lambda`
    ///   2. `parse_decls_only` → resolve → `extract_arities` (sobrescreve)
    ///   3. `parse_with_arity_recovery` (parse completo com recovery)
    ///
    /// `Single` chama `parse_with_recovery` diretamente.
    ///
    /// Ambos os modos usam error recovery: quando um top-level item falha,
    /// o erro é registrado e o parser skipa tokens até o próximo `StmtSep`
    /// ou `Eof`, então continua. Se houver erros, retorna `Err(Vec<Report>)`
    /// com todos os erros encontrados — o pipeline para e o driver imprime
    /// todos antes de abortar.
    pub fn parse(mut self, mode: ParseMode, file_path: Option<&str>) -> PipelineResult<Self> {
        // Armazenar file_path para source context em passos posteriores.
        self.file_path = file_path.map(|s| s.to_string());
        let tokens = self
            .tokens
            .take()
            .ok_or_else(|| err("parse chamado antes de lex"))?;

        let (module, parse_errors) = match mode {
            ParseMode::Single => parse_with_recovery(tokens),
            ParseMode::TwoPass => {
                let mut arities = scan_lambdas(&tokens);
                // Pass 1: parse_decls_only (sem recovery — só extrai assinaturas).
                // Se falhar, aborta com o erro (não há benefício em recovery aqui,
                // pois o Pass 2 precisa das aridades que o Pass 1 produz).
                let decls_module = parse_decls_only(tokens.clone())
                    .map_err(|e| one_err(e.into_report_with_source(&self.source, file_path)))?;
                let decls_resolved = quick_resolve(&decls_module, file_path)?;
                arities.extend(extract_arities(&decls_resolved.signatures));
                // Pass 2: parse completo com recovery.
                parse_with_arity_recovery(tokens, arities)
            }
        };

        if !parse_errors.is_empty() {
            return Err(parse_errors
                .into_iter()
                .map(|e| e.into_report_with_source(&self.source, file_path))
                .collect());
        }

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

        let prelude = load_prelude().map_err(|e| resolve_errors_to_reports(&e, "", None))?;

        let imports = match file_path {
            Some(file) => imports::load_module_imports(file, module).map_err(one_err)?,
            None => Vec::new(),
        };
        let imported_directives = imports::collect_imported_directives(&imports);
        let user = resolve_with_prelude(
            module,
            "__local__",
            imported_directives,
            &prelude.interface_registry,
            &prelude.directive_registry,
            &prelude.enum_registry,
        )
        .map_err(|e| {
            e.into_iter()
                .map(|re| re.into_report_with_source(&self.source, self.file_path.as_deref()))
                .collect::<Vec<_>>()
        })?;
        let mut resolved = merge_resolved(prelude, user);
        imports::merge_imports(&mut resolved, &imports);

        self.imports = imports;
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

        // Fase 4: Avaliar constants importadas (pipeline recursivo nos importados)
        // ANTES de infer_module. O type_env do importador precisa dos bindings
        // das constants importadas para que Ident("escala") resolva durante
        // a inferência. As constants já vêm avaliadas (literal/HeapSnapshot).
        let imported_constants = if !self.imports.is_empty() {
            imports::evaluate_imported_constants(&self.imports).map_err(one_err)?
        } else {
            Vec::new()
        };

        // Clonar resolved para mutar o type_env com bindings importados.
        // infer_module toma &ResolvedModule, então precisamos mutar antes.
        let mut resolved = (*resolved).clone();
        for ic in &imported_constants {
            resolved
                .type_env
                .define(&ic.name, ic.value.ty.clone(), "__module__");
        }
        let resolved = &resolved; // re-borrow como imutável

        let mut typed = infer_module(module, resolved).map_err(|e| {
            one_err(e.into_report_with_source(&self.source, self.file_path.as_deref()))
        })?;

        // Injetar constants importadas como ConstantBinding no TypedModule.
        // O valor já está avaliado (literal/snapshot) — o comptime pass
        // vai pular via is_already_evaluated e registrar no comptime_bindings.
        for ic in imported_constants {
            let dummy_span = kata_ast::Span::zero();
            typed.constants.push(kata_ast::Spanned::new(
                kata_inference::TypedExpr {
                    span: dummy_span,
                    ty: ic.value.ty.clone(),
                    tail_pos: false,
                    escape: kata_core::escape::EscapeTarget::Local,
                    kind: kata_inference::TypedExprKind::ConstantBinding {
                        name: ic.name.clone(),
                        value: Box::new(kata_ast::Spanned::new(ic.value, dummy_span)),
                    },
                },
                dummy_span,
            ));
        }

        // Display wrapping: se ativado, envolve o entry point com `show`
        // para que o driver possa imprimir tipos compostos como Text.
        if self.display_wrap {
            kata_inference::wrap_entry_with_show(&mut typed);
        }

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

        let shaken = run_comptime_pass(mono.inner, &enum_registry).map_err(|e| {
            one_err(e.into_report_with_source(&self.source, self.file_path.as_deref()))
        })?;
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

        let (type_id_map, type_shapes) =
            type_table::build_and_register_type_table(&mono, &mono.struct_registry, &enum_registry);

        Ok(CompiledModule {
            mono,
            type_id_map,
            type_shapes,
            source: self.source,
            file_path: self.file_path,
        })
    }
}

// ── Helpers ────────────────────────────────────────────────

/// Converte `Vec<ResolveError>` (de `load_prelude` ou `resolve_with_imports`)
/// em `Vec<Report>`. `source` e `file` são do módulo onde o erro ocorreu
/// (prelude não tem file_path; o módulo do usuário usa `self.source`).
fn resolve_errors_to_reports(
    errors: &[kata_resolution::ResolveError],
    source: &str,
    file: Option<&str>,
) -> Vec<miette::Report> {
    errors
        .iter()
        .cloned()
        .map(|e| e.into_report_with_source(source, file))
        .collect()
}

/// Resolve rápido para o Pass 1 do TwoPass (sem desugar).
///
/// Carrega imports (para validar diretivas importadas no Pass 1) mas
/// não faz desugar — só precisa das assinaturas para extract_arities.
fn quick_resolve(
    module: &kata_ast::Module,
    file_path: Option<&str>,
) -> PipelineResult<ResolvedModule> {
    let prelude = load_prelude().map_err(|e| resolve_errors_to_reports(&e, "", None))?;

    let imports = match file_path {
        Some(file) => imports::load_module_imports(file, module).map_err(one_err)?,
        None => Vec::new(),
    };
    let imported_directives = imports::collect_imported_directives(&imports);

    let user = resolve_with_prelude(
        module,
        "__local__",
        imported_directives,
        &prelude.interface_registry,
        &prelude.directive_registry,
        &prelude.enum_registry,
    )
    .map_err(|e| {
        e.into_iter()
            .map(|re| re.into_report_with_source("", file_path))
            .collect::<Vec<_>>()
    })?;
    let resolved = merge_resolved(prelude, user);
    Ok(resolved)
}
