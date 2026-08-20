//! REPL interativo — `kata repl`.
//!
//! Mantém `Vec<Spanned<Item>>` acumulados entre expressões. A cada input:
//! lex → parse → adiciona item à lista → resolve + merge + infer_module →
//! monomorphize → optimize → jit_eval → display.
//!
//! Bindings `let` escalares (Int, Float, Boolean, Unit) são congelados após
//! a primeira avaliação: o valor é guardado como literal e injetado nas
//! próximas linhas como `let x := <literal>` em vez de reavaliar a expressão
//! original. Isto evita reexecução de computação cara em cada linha.
//!
//! Erros não abortam a sessão: o item adicionado é removido (rollback) e o
//! usuário pode corrigir e reintentar.

mod commands;
mod repl_loop;

use std::collections::HashMap;
use std::path::PathBuf;

use kata_ast::{Expr, Item, Module, Span, Spanned};
use kata_codegen::jit_eval_repl;
use kata_comptime::run_comptime_pass;
use kata_core::ty::{PrimTy, Ty};
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::{parse, parse_repl_decls_only, parse_repl_with_arity, scan_lambdas};
use kata_resolution::{ResolvedModule, extract_arities, load_prelude, resolve};
use kata_tree_shaking::tree_shake;

use crate::display;
use crate::merge_resolved;

/// Sessão REPL — acumula items do usuário entre expressões.
pub(crate) struct ReplSession {
    /// Items top-level acumulados (let bindings, sigs, data, enum, etc.).
    pub(crate) items: Vec<Spanned<Item>>,
    /// Bindings `let` escalares congelados — mapa nome → literal AST.
    /// Após avaliar `let x := <expr>` e obter um valor escalar, o valor
    /// é guardado aqui como literal. Nas próximas linhas, o item `let`
    /// nos items acumulados é substituído pelo literal antes do pipeline.
    pub(crate) frozen_bindings: HashMap<String, Expr>,
    /// Bindings `let` complexos congelados — mapa nome → (snapshot_id, Ty).
    /// Para tipos que não podem ser expressos como literais AST (List,
    /// Struct, Tuple, Text, Sum), o valor é serializado como snapshot
    /// e persistido na root_arena. O snapshot_id é global na sessão.
    pub(crate) snapshot_bindings: HashMap<String, (u32, Ty)>,
    /// Snapshots acumulados — indexados por snapshot_id global.
    pub(crate) snapshots: Vec<kata_core::snapshot::HeapSnapshotData>,
    /// Funções nomeadas persistidas entre linhas — mapeia fn_hash →
    /// (cranelift_name, fn_ptr). O fn_ptr é absoluto e permanece válido
    /// porque o JITModule anterior é leaked (páginas de código mapeadas).
    /// Na próxima linha, estes símbolos são registrados no JITBuilder e
    /// declarados como Linkage::Import — o corpo não é recompilado.
    pub(crate) function_table: kata_codegen::PrevFuncMap,
    /// Módulos importados via `import` no REPL — cacheados entre linhas.
    /// Carregados uma vez quando o usuário digita `import MOD.(items)`,
    /// reusados em todas as avaliações subsequentes via `merge_imports`.
    pub(crate) imports: Vec<kata_resolution::ImportedModule>,
    /// Prelude resolvido (recarregado em `:reset`).
    pub(crate) prelude: ResolvedModule,
    /// Runtime persistente — vive entre avaliações para preservar valores
    /// na arena Bump e type table entre linhas.
    pub(crate) rt_ptr: i64,
    /// Histórico rustyline.
    pub(crate) history_path: PathBuf,
}

impl ReplSession {
    /// Cria nova sessão carregando o prelude.
    pub fn new() -> Result<Self, String> {
        let prelude = load_prelude()
            .map_err(|e| format!("erro ao carregar prelude: {}", crate::format_error_vec(&e)))?;
        let history_path = dirs();
        // Runtime persistente: vive entre avaliações. Leak intencional —
        // o REPL é de longa duração e valores na arena devem persistir.
        let rt = Box::new(kata_rt::Runtime::new());
        let rt_ptr = Box::into_raw(rt) as i64;
        Ok(Self {
            items: Vec::new(),
            frozen_bindings: HashMap::new(),
            snapshot_bindings: HashMap::new(),
            snapshots: Vec::new(),
            function_table: HashMap::new(),
            imports: Vec::new(),
            prelude,
            rt_ptr,
            history_path,
        })
    }

    /// Reseta a sessão — limpa items e recarrega prelude.
    pub fn reset(&mut self) -> Result<(), String> {
        self.items.clear();
        self.frozen_bindings.clear();
        self.snapshot_bindings.clear();
        self.snapshots.clear();
        self.function_table.clear();
        self.imports.clear();
        self.prelude = load_prelude()
            .map_err(|e| format!("erro ao carregar prelude: {}", crate::format_error_vec(&e)))?;
        // Recriar Runtime — descarta o antigo e cria um novo limpo.
        let _ = unsafe { Box::from_raw(self.rt_ptr as *mut kata_rt::Runtime) };
        let rt = Box::new(kata_rt::Runtime::new());
        self.rt_ptr = Box::into_raw(rt) as i64;
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

    /// Avalia uma expressão pelo pipeline completo.
    ///
    /// Se o input contém apenas declarações (Sig, DataDecl, EnumDecl, etc.)
    /// sem EntryExpr, apenas adiciona à lista — não executa. Se contém
    /// EntryExpr, executa o pipeline completo.
    fn eval_expr(&mut self, input: &str) -> Result<(), String> {
        // Lex
        let tokens = lex(input).map_err(|e| format!("erro léxico: {e}"))?;

        // Pass 0: scan_lambdas — aridades de `let f := lambda ...`
        let mut arities = scan_lambdas(&tokens);

        // Pass 1: parse_decls_only → resolve → extract_arities
        // Signatures definem a aridade padrão; lambdas com mesmo nome são
        // overloads non-default.
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

        // Verifica se há EntryExpr no input.
        let has_entry = module
            .items
            .iter()
            .any(|i| matches!(i.node, Item::EntryExpr(_)));

        // Guarda os items do input antes de movê-los para self.items.
        // Necessário para congelar bindings após avaliação.
        let input_items = module.items.clone();

        // Snapshot para rollback em caso de erro.
        let snapshot_len = self.items.len();
        let snapshot_imports_len = self.imports.len();

        // Substitui constants redefinidas: se o novo input contém
        // `ConstantDecl { name }`, remove qualquer `ConstantDecl` anterior
        // com o mesmo nome de self.items. A inference rejeita duplicatas
        // — o REPL resolve redefinições antes de chegar lá.
        let new_constant_names: std::collections::HashSet<String> = module
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

        // Substitui `let` bindings redefinidos: se o novo input contém
        // `let x := ...` (como EntryExpr), remove qualquer `let x` anterior
        // de self.items. O REPL permite re-declarar `let` entre linhas
        // (sessão iterativa) — a inference rejeita duplicatas no mesmo
        // escopo, então o REPL resolve antes. Closures que capturaram
        // o binding anterior perdem a referência — re-declarar `let`
        // no REPL é re-definir, não shadow.
        let new_let_names: std::collections::HashSet<String> = module
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
        }

        self.items.extend(module.items);

        // Carregar imports do módulo: se há ImportDecl nos items (novos ou
        // acumulados), carregar todos os módulos importados e atualizar o
        // cache. Isto é feito a cada eval porque imports podem aparecer em
        // qualquer linha, e o ModuleLoader é idempotente (cache interno).
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
            // Apenas declarações — não executa, apenas adiciona à lista.
            // Valida com pipeline_typed para verificar consistência.
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
            // Constrói Module com todos os items acumulados, substituindo
            // bindings congelados por literais.
            let full_module = self.build_eval_module(&self.items);

            match self.run_pipeline_eval(&full_module) {
                Ok(result) => {
                    // Congelar bindings escalares: se a entrada é um
                    // `let x := <expr>` isolado (único EntryExpr do input),
                    // avaliar o valor do binding separadamente para obter
                    // seu tipo e valor, e guardar como literal.
                    if input_items.len() == 1
                        && let Item::EntryExpr(ref expr) = input_items[0].node
                        && let Expr::Let {
                            ref name,
                            ref value,
                        } = expr.node
                    {
                        // Avaliar só o valor do let para obter tipo
                        // e valor corretos (o entry retorna Unit).
                        // Inclui items acumulados (bindings anteriores)
                        // para que o valor possa referenciá-los.
                        let mut val_items = self.items.clone();
                        // Substitui bindings congelados por literais.
                        val_items = self.build_eval_items(&val_items);
                        val_items.push(Spanned::new(
                            Item::EntryExpr(*value.clone()),
                            Span::synthetic(),
                        ));
                        let val_module = Module { items: val_items };
                        if let Ok(val_result) = self.run_pipeline_eval(&val_module) {
                            // Tentar congelar como literal escalar.
                            if let Some(literal) =
                                Self::decode_to_literal(val_result.raw, &val_result.ty)
                            {
                                self.frozen_bindings.insert(name.clone(), literal);
                            } else {
                                // Tipo complexo — serializar como snapshot.
                                // Obter registries do resolved da avaliação.
                                let val_user = resolve(&val_module).map_err(|e| {
                                    format!("erro de resolução: {}", crate::format_error_vec(&e))
                                })?;
                                let val_resolved = merge_resolved(self.prelude.clone(), val_user);
                                match kata_comptime::serialize_value(
                                    val_result.raw,
                                    &val_result.ty,
                                    &val_resolved.struct_registry,
                                    &val_resolved.enum_registry,
                                ) {
                                    Ok(snap) => {
                                        let snapshot_id = self.snapshots.len() as u32;
                                        self.snapshots.push(snap);
                                        self.snapshot_bindings
                                            .insert(name.clone(), (snapshot_id, val_result.ty));
                                    }
                                    Err(e) => {
                                        // Não consegue serializar — deixar como item
                                        // acumulado (reprocessa a cada linha).
                                        eprintln!(
                                            "aviso: não foi possível congelar binding '{name}': {e}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // Suprimir Unit — igual a cmd_run. Declaracoes (let,
                    // constant, Sig, Data, Enum) retornam Unit e não devem
                    // imprimir `()` no REPL. echo! já produziu seu output.
                    if !matches!(result.ty, Ty::Unit) {
                        display::print_result(result.raw, &result.ty);
                    }
                    // Remove EntryExpr que não são bindings — expressões
                    // puras (ex: `5`, `echo!(5)`, `g 5`) são "avaliar e
                    // esquecer". Apenas `Let`/`LetDestruct` persistem.
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
        let mut items = self.build_eval_items(&self.items);
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

    /// Roda o pipeline até TypedModule (para `:type`).
    fn run_pipeline_typed(&self, module: &Module) -> Result<kata_inference::TypedModule, String> {
        let user = resolve(module)
            .map_err(|e| format!("erro de resolução: {}", crate::format_error_vec(&e)))?;
        let mut resolved = merge_resolved(self.prelude.clone(), user);
        // Merge de imports cacheados — traz signatures/functions/actions
        // dos módulos importados para o escopo do REPL.
        if !self.imports.is_empty() {
            kata_resolution::merge_imports(&mut resolved, &self.imports);
        }
        let typed = infer_module(module, &resolved).map_err(|e| format!("erro de tipo: {e}"))?;
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
        let items = if has_entry {
            self.build_eval_items(&module.items)
        } else {
            let mut items = self.build_eval_items(&module.items);
            let zero = kata_ast::Expr::IntLit {
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

    /// Roda o pipeline completo até execução JIT.
    ///
    /// Após infer_module, injeta bindings complexos congelados como
    /// HeapSnapshot no pre_entry da TAST, e inclui os snapshots no
    /// TypedModule.snapshots para que o codegen emita load_snapshot.
    fn run_pipeline_eval(&mut self, module: &Module) -> Result<crate::ExecResult, String> {
        let user = resolve(module)
            .map_err(|e| format!("erro de resolução: {}", crate::format_error_vec(&e)))?;
        let mut resolved = merge_resolved(self.prelude.clone(), user);

        // Merge de imports cacheados + avaliação de constants importadas.
        // Isto traz signatures/functions/actions dos módulos importados e
        // injeta constants exportadas no type_env antes da inferência.
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

        // Comptime pass: avalia constants (JIT-executa), substitui por
        // literais/snapshots, e roda constant_fold (substitui Ident de
        // constants nos corpos de functions e actions). Isto é essencial
        // para que functions e actions definidas no REPL possam referenciar
        // constants — sem o fold, o var_map do FunctionBuilder delas não
        // tem acesso às constants (que só existem no var_map do entry point).
        let mut typed = run_comptime_pass(typed, &resolved.enum_registry)
            .map_err(|e| format!("erro de comptime: {e}"))?;

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

        // Injetar bindings complexos congelados como HeapSnapshot no pre_entry.
        if !self.snapshot_bindings.is_empty() {
            self.inject_snapshot_bindings(
                &mut typed,
                &resolved.struct_registry,
                &resolved.enum_registry,
            );
        }

        let mono = monomorphize(typed);
        let mono = optimize(mono);
        let mono = kata_monomorph::MonoModule::from(tree_shake(mono.inner));
        let repl_result = jit_eval_repl(
            &mono,
            &Default::default(),
            &[],
            self.rt_ptr,
            &self.function_table,
        )
        .map_err(|e| format!("erro de codegen: {e}"))?;

        // Persistir function pointers das funções nomeadas recém-compiladas.
        for (fn_hash, cranelift_name, fn_ptr) in repl_result.new_funcs {
            self.function_table
                .insert(fn_hash, (cranelift_name, fn_ptr));
        }

        Ok(crate::ExecResult {
            raw: repl_result.jit.raw,
            ty: repl_result.jit.ty,
        })
    }

    /// Substitui valores de bindings complexos no pre_entry da TAST por
    /// HeapSnapshot. Para cada `Let { name, value }` no pre_entry onde
    /// `name` está em `snapshot_bindings`, substitui `value` por um
    /// TypedExpr com `kind: HeapSnapshot { snapshot_id, ty }`.
    ///
    /// Também inclui os snapshots correspondentes em `typed.snapshots`
    /// para que o codegen emita `kata_rt_load_snapshot` no prólogo.
    fn inject_snapshot_bindings(
        &self,
        typed: &mut kata_inference::TypedModule,
        _struct_registry: &kata_core::StructRegistry,
        _enum_registry: &kata_core::EnumRegistry,
    ) {
        use kata_core::escape::EscapeTarget;
        use kata_inference::{TypedExpr, TypedExprKind};

        // Para o REPL, typed.snapshots começa vazio (sem comptime pass).
        // Incluímos TODOS os snapshots da sessão em typed.snapshots, na
        // ordem global. O snapshot_id no HeapSnapshot (TAST) e no
        // load_snapshot (codegen) é o índice em typed.snapshots, que
        // corresponde ao global_id.
        let existing_count = typed.snapshots.len() as u32;

        // Adicionar todos os snapshots da sessão que ainda não estão
        // em typed.snapshots.
        for (i, snap) in self.snapshots.iter().enumerate() {
            let global_id = i as u32;
            if global_id >= existing_count {
                typed.snapshots.push(snap.clone());
            }
        }

        // Substituir o value do Let no pre_entry por HeapSnapshot.
        // Para shadowing: o último Let com um nome dado usa o snapshot
        // mais recente (snapshot_bindings[name] = (global_id, ty)).
        for (name, (global_id, ty)) in &self.snapshot_bindings {
            let snap_expr = TypedExpr {
                span: Span::synthetic(),
                ty: ty.clone(),
                tail_pos: false,
                escape: EscapeTarget::Caller,
                kind: TypedExprKind::HeapSnapshot {
                    snapshot_id: *global_id,
                    ty: ty.clone(),
                },
            };

            // Substituir apenas o último Let com este nome (shadowing).
            // O último Let no pre_entry com este nome é o ativo.
            let mut last_idx: Option<usize> = None;
            for (i, pre) in typed.pre_entry.iter().enumerate() {
                if let TypedExprKind::Let { name: n, .. } = &pre.node.kind
                    && n == name
                {
                    last_idx = Some(i);
                }
            }
            if let Some(i) = last_idx
                && let TypedExprKind::Let { value, .. } = &mut typed.pre_entry[i].node.kind
            {
                **value = Spanned::new(snap_expr, Span::synthetic());
            }
        }
    }

    /// Constrói a lista de items substituindo bindings congelados por literais.
    fn build_eval_items(&self, items: &[Spanned<Item>]) -> Vec<Spanned<Item>> {
        if self.frozen_bindings.is_empty() {
            return items.to_vec();
        }
        items
            .iter()
            .map(|item| {
                if let Item::EntryExpr(ref expr) = item.node
                    && let Expr::Let { ref name, .. } = expr.node
                    && let Some(literal) = self.frozen_bindings.get(name)
                {
                    let new_expr = Expr::Let {
                        name: name.clone(),
                        value: Box::new(Spanned::new(literal.clone(), Span::synthetic())),
                    };
                    return Spanned::new(
                        Item::EntryExpr(Spanned::new(new_expr, Span::synthetic())),
                        Span::synthetic(),
                    );
                }
                item.clone()
            })
            .collect()
    }

    /// Constrói o Module para avaliação, substituindo bindings congelados
    /// por literais.
    fn build_eval_module(&self, items: &[Spanned<Item>]) -> Module {
        Module {
            items: self.build_eval_items(items),
        }
    }

    /// Decodifica um JitResult (valor bruto + tipo) em um literal AST.
    /// Apenas para escalares: Int (SMI), Float, Boolean, Unit.
    /// Retorna None para tipos complexos (List, Struct, Text, etc.) —
    /// estes não podem ser expressos como literais na AST.
    fn decode_to_literal(raw: i64, ty: &Ty) -> Option<Expr> {
        match ty {
            // Int SMI: LSB=1 → value = (raw - 1) >> 1
            Ty::Prim(PrimTy::Int) => {
                if (raw as u64) & 1 == 1 {
                    let value = (raw - 1) >> 1;
                    Some(Expr::IntLit {
                        text: format!("{value}"),
                    })
                } else {
                    // BigInt — LSB=0, ponteiro. Não suportado nesta fase.
                    None
                }
            }
            // Float: raw é f64 reinterpretado como i64
            Ty::Prim(PrimTy::Float) => {
                let f = f64::from_bits(raw as u64);
                Some(Expr::FloatLit {
                    text: format!("{f}"),
                })
            }
            // Boolean: True = SMI 1, False = SMI 0
            Ty::Sum(name) if name == "Boolean" => {
                let variant = if raw != 0 { "True" } else { "False" };
                Some(Expr::VariantQual {
                    enum_name: "Boolean".to_string(),
                    variant: variant.to_string(),
                    module_path: None,
                })
            }
            Ty::Unit => Some(Expr::Unit),
            _ => None,
        }
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
            Err(e) => format!("{e}").contains("<EOF>"),
        }
    }
}

/// Caminho do arquivo de histórico do REPL.
fn dirs() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".kata_repl_history")
}

pub(crate) use repl_loop::cmd_repl;
