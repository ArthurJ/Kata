//! Action Step Compiler - Gera código Cranelift para Actions como máquinas de estado
//!
//! Este módulo compila Actions não como funções síncronas simples, mas como
//! state machines que podem suspender e retomar execução em pontos de I/O.

use crate::codegen::action_state::{ActionStateLayout, ActionAnalyzer, SuspensionPoint};
use crate::codegen::context::CodegenContext;
use crate::parser::ast::{Spanned, TypeRef};
use crate::type_checker::tast::{TStmt, TExpr, TLiteral};
use cranelift_codegen::ir::{InstBuilder, Value, types, MemFlags, Block};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Module, FuncId, Linkage};
use std::collections::HashMap;

/// Constantes para valores especiais de StepResult
pub const STEP_DONE: i64 = 0;
pub const STEP_PENDING: i64 = 1;
pub const STEP_ERROR: i64 = 2;

/// Compilador de Actions como máquinas de estado
pub struct ActionStepCompiler<'a> {
    pub ctx: &'a mut CodegenContext,
    pub env: &'a crate::type_checker::environment::TypeEnv,
    builder_context: FunctionBuilderContext,
}

impl<'a> ActionStepCompiler<'a> {
    pub fn new(ctx: &'a mut CodegenContext, env: &'a crate::type_checker::environment::TypeEnv) -> Self {
        Self {
            ctx,
            env,
            builder_context: FunctionBuilderContext::new(),
        }
    }

    /// Compila uma Action como uma função step
    ///
    /// A função step gerada tem a assinatura:
    /// `step_fn(state_ptr: *mut u8, waker: *const WakerRaw) -> StepResult`
    #[allow(dead_code)]
    pub fn compile_action(
        &mut self,
        name: &str,
        params: &[(String, Spanned<TypeRef>)],
        body: &[Spanned<TStmt>],
        _ret_ty: &TypeRef,
    ) -> Result<FuncId, String> {
        // 1. Analisa a Action para encontrar pontos de suspensão
        let analyzer = Self::analyze_suspension_points_static(body)?;

        // 2. Cria o layout do estado
        let layout = self.create_layout(name, params, &analyzer)?;

        // 3. Cria a assinatura da função step
        let ptr_type = self.ctx.module.target_config().pointer_type();
        let mut sig = self.ctx.module.make_signature();
        sig.params.push(cranelift_codegen::ir::AbiParam::new(ptr_type)); // state_ptr
        sig.params.push(cranelift_codegen::ir::AbiParam::new(ptr_type)); // waker
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64)); // StepResult (enum)

        // 4. Declara a função step
        let step_name = format!("{}_step", name);
        let func_id = self.ctx.module
            .declare_function(&step_name, Linkage::Export, &sig)
            .map_err(|e| format!("Falha ao declarar step {}: {}", name, e))?;

        // 5. Compila o corpo
        let mut ctx = Context::new();
        ctx.func.signature = sig;

        let mut builder_context = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_context);

        // Cria bloco de entrada
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Parâmetros: state_ptr, waker
        let state_ptr = builder.block_params(entry_block)[0];

        // Carrega o stage atual
        let stage_offset = layout.stage_offset() as i32;
        let stage_val = builder.ins().load(types::I8, MemFlags::new(), state_ptr, stage_offset);
        let stage_i64 = builder.ins().uextend(types::I64, stage_val);

        // Cria blocos para cada stage
        let mut stage_blocks: HashMap<usize, Block> = HashMap::new();
        for stage in 0..layout.num_stages {
            let block = builder.create_block();
            stage_blocks.insert(stage, block);
        }

        // Cria bloco de saída
        let done_block = builder.create_block();
        builder.append_block_param(done_block, types::I64); // valor de retorno

        // Cria bloco de pending
        let pending_block = builder.create_block();

        // Switch no stage - cria branch para stage 0
        let block_0 = stage_blocks[&0];
        let stage_is_zero = builder.ins().icmp_imm(cranelift_codegen::ir::condcodes::IntCC::Equal, stage_i64, 0);
        let default_block = if layout.num_stages > 1 { stage_blocks[&1] } else { pending_block };
        builder.ins().brif(stage_is_zero, block_0, &[], default_block, &[]);

        // Compila blocos de cada stage
        for stage in 0..layout.num_stages {
            let stage_block = stage_blocks[&stage];
            builder.switch_to_block(stage_block);
            builder.seal_block(stage_block);

            // Encontra o ponto de suspensão correspondente
            let suspension = analyzer.suspension_points.iter().find(|s| {
                matches!(s, SuspensionPoint::ChannelRecv { stage_before, .. } if *stage_before == stage)
                    || matches!(s, SuspensionPoint::ChannelSend { stage_before, .. } if *stage_before == stage)
                    || matches!(s, SuspensionPoint::Sleep { stage_before, .. } if *stage_before == stage)
            });

            // Compila as instruções do stage
            if stage == 0 {
                // Primeiro stage: inicializa variáveis e processa até primeira suspensão
                Self::compile_stage_code_static(&mut builder, body, &layout, suspension, state_ptr, done_block, pending_block)?;
            }
            // Stages subsequentes seriam implementados com retomada do estado
        }

        // Bloco pending: salva state e retorna PENDING
        builder.switch_to_block(pending_block);
        builder.seal_block(pending_block);
        let pending_result = builder.ins().iconst(types::I64, STEP_PENDING);
        builder.ins().return_(&[pending_result]);

        // Bloco done: retorna o valor
        builder.switch_to_block(done_block);
        builder.seal_block(done_block);
        let ret_val = builder.block_params(done_block)[0];
        builder.ins().return_(&[ret_val]);

        builder.finalize();

        // Define a função
        self.ctx.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("Falha ao definir step {}: {}", name, e))?;

        Ok(func_id)
    }

    /// Analisa os pontos de suspensão no corpo de uma Action (versão estática)
    fn analyze_suspension_points_static(body: &[Spanned<TStmt>]) -> Result<ActionAnalyzer, String> {
        let mut analyzer = ActionAnalyzer::new();

        for stmt in body {
            Self::find_suspensions_in_stmt_static(&stmt.0, &mut analyzer)?;
        }

        Ok(analyzer)
    }

    /// Encontra pontos de suspensão em um statement (versão estática)
    fn find_suspensions_in_stmt_static(stmt: &TStmt, analyzer: &mut ActionAnalyzer) -> Result<(), String> {
        match stmt {
            TStmt::Let(_, expr) | TStmt::Var(_, expr) | TStmt::Expr(expr) => {
                Self::find_suspensions_in_expr_static(&expr.0, analyzer)
            }
            TStmt::Match(_, arms) => {
                for arm in arms {
                    for s in &arm.body {
                        Self::find_suspensions_in_stmt_static(&s.0, analyzer)?;
                    }
                }
                Ok(())
            }
            TStmt::Loop(body) | TStmt::For(_, _, body) => {
                for s in body {
                    Self::find_suspensions_in_stmt_static(&s.0, analyzer)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Encontra pontos de suspensão em uma expressão (versão estática)
    fn find_suspensions_in_expr_static(expr: &TExpr, analyzer: &mut ActionAnalyzer) -> Result<(), String> {
        match expr {
            TExpr::ChannelRecv(channel, _) => {
                let stage_before = analyzer.stage();
                let stage_after = analyzer.next_stage();
                analyzer.suspension_points.push(SuspensionPoint::ChannelRecv {
                    var_name: String::new(),
                    channel_expr: format!("{:?}", channel.0),
                    stage_before,
                    stage_after,
                });
                Ok(())
            }
            TExpr::ChannelSend(channel, value, _) => {
                let stage_before = analyzer.stage();
                let stage_after = analyzer.next_stage();
                analyzer.suspension_points.push(SuspensionPoint::ChannelSend {
                    channel_expr: format!("{:?}", channel.0),
                    value_expr: format!("{:?}", value.0),
                    stage_before,
                    stage_after,
                });
                Ok(())
            }
            TExpr::Call(callee, args, _) => {
                // Verifica se é sleep! ou outra Action
                if let TExpr::Ident(name, _) = &callee.0 {
                    if name == "sleep!" {
                        let stage_before = analyzer.stage();
                        let stage_after = analyzer.next_stage();
                        analyzer.suspension_points.push(SuspensionPoint::Sleep {
                            millis_expr: args.first().map(|a| format!("{:?}", a.0)).unwrap_or_default(),
                            stage_before,
                            stage_after,
                        });
                        return Ok(());
                    }
                }
                // Verifica argumentos recursivamente
                for arg in args {
                    Self::find_suspensions_in_expr_static(&arg.0, analyzer)?;
                }
                Ok(())
            }
            TExpr::Sequence(exprs, _) => {
                for e in exprs {
                    Self::find_suspensions_in_expr_static(&e.0, analyzer)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Compila o código de um stage específico (versão estática)
    fn compile_stage_code_static(
        builder: &mut FunctionBuilder,
        body: &[Spanned<TStmt>],
        layout: &ActionStateLayout,
        suspension: Option<&SuspensionPoint>,
        state_ptr: Value,
        done_block: Block,
        pending_block: Block,
    ) -> Result<(), String> {
        // Variáveis locais
        let mut variables: HashMap<String, Value> = HashMap::new();

        // Compila cada statement
        for stmt in body {
            match &stmt.0 {
                TStmt::Let(name, expr) => {
                    let val = Self::translate_expr_simple_static(builder, expr, &variables)?;
                    // Extrai nome da variável
                    let var_name = match &name.0 {
                        crate::parser::ast::Pattern::Ident(n) => n.clone(),
                        _ => format!("var_{}", variables.len()),
                    };
                    // Salva no estado
                    if let Some(offset) = layout.get_var_offset(&var_name) {
                        builder.ins().store(MemFlags::new(), val, state_ptr, offset as i32);
                        variables.insert(var_name, val);
                    }
                }
                TStmt::Expr(expr) => {
                    Self::translate_expr_simple_static(builder, expr, &variables)?;

                    // Verifica se chegamos no ponto de suspensão
                    if let Some(susp) = suspension {
                        if Self::is_at_suspension_point_static(expr, susp) {
                            // Salva o próximo stage no estado
                            let next_stage = susp.stage_after();
                            let stage_val = builder.ins().iconst(types::I8, next_stage as i64);
                            builder.ins().store(MemFlags::new(), stage_val, state_ptr, 0);

                            // Salta para pending
                            builder.ins().jump(pending_block, &[]);
                            return Ok(());
                        }
                    }
                }
                _ => {}
            }
        }

        // Se não há suspensão, continua para o done_block
        let result = builder.ins().iconst(types::I64, 0); // Unit
        builder.ins().jump(done_block, &[result]);

        Ok(())
    }

    /// Traduz uma expressão de forma simples (versão estática)
    fn translate_expr_simple_static(
        builder: &mut FunctionBuilder,
        expr: &Spanned<TExpr>,
        _variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        match &expr.0 {
            TExpr::Literal(TLiteral::Int(i)) => {
                Ok(builder.ins().iconst(types::I64, *i as i64))
            }
            TExpr::Literal(TLiteral::Bool(b)) => {
                Ok(builder.ins().iconst(types::I8, if *b { 1 } else { 0 }))
            }
            TExpr::Literal(TLiteral::Unit) => {
                Ok(builder.ins().iconst(types::I32, 0))
            }
            // TODO: Implementar outras expressões
            _ => Err(format!("Expressão não suportada em Action: {:?}", expr.0))
        }
    }

    /// Verifica se a expressão é um ponto de suspensão (versão estática)
    fn is_at_suspension_point_static(expr: &Spanned<TExpr>, susp: &SuspensionPoint) -> bool {
        match (&expr.0, susp) {
            (TExpr::ChannelRecv(_, _), SuspensionPoint::ChannelRecv { .. }) => true,
            (TExpr::ChannelSend(_, _, _), SuspensionPoint::ChannelSend { .. }) => true,
            _ => false,
        }
    }

    /// Compila o código de um stage específico
    fn compile_stage_code(
        &self,
        builder: &mut FunctionBuilder,
        body: &[Spanned<TStmt>],
        layout: &ActionStateLayout,
        stage: usize,
        suspension: Option<&SuspensionPoint>,
        state_ptr: Value,
        done_block: Block,
        pending_block: Block,
    ) -> Result<(), String> {
        // Variáveis locais
        let mut variables: HashMap<String, Value> = HashMap::new();

        // Compila cada statement
        for stmt in body {
            match &stmt.0 {
                TStmt::Let(name, expr) => {
                    let val = self.translate_expr_simple(builder, expr, &variables)?;
                    // Extrai nome da variável
                    let var_name = match &name.0 {
                        crate::parser::ast::Pattern::Ident(n) => n.clone(),
                        _ => format!("var_{}", variables.len()),
                    };
                    // Salva no estado
                    if let Some(offset) = layout.get_var_offset(&var_name) {
                        builder.ins().store(MemFlags::new(), val, state_ptr, offset as i32);
                        variables.insert(var_name, val);
                    }
                }
                TStmt::Expr(expr) => {
                    self.translate_expr_simple(builder, expr, &variables)?;

                    // Verifica se chegamos no ponto de suspensão
                    if let Some(susp) = suspension {
                        if self.is_at_suspension_point(expr, susp) {
                            // Salva o próximo stage no estado
                            let next_stage = susp.stage_after();
                            let stage_val = builder.ins().iconst(types::I8, next_stage as i64);
                            builder.ins().store(MemFlags::new(), stage_val, state_ptr, 0);

                            // Salta para pending
                            builder.ins().jump(pending_block, &[]);
                            return Ok(());
                        }
                    }
                }
                _ => {}
            }
        }

        // Se não há suspensão, continua para o done_block
        let result = builder.ins().iconst(types::I64, 0); // Unit
        builder.ins().jump(done_block, &[result]);

        Ok(())
    }

    /// Traduz uma expressão de forma simples (sem suspensão)
    fn translate_expr_simple(
        &self,
        builder: &mut FunctionBuilder,
        expr: &Spanned<TExpr>,
        _variables: &HashMap<String, Value>,
    ) -> Result<Value, String> {
        match &expr.0 {
            TExpr::Literal(TLiteral::Int(i)) => {
                Ok(builder.ins().iconst(types::I64, *i as i64))
            }
            TExpr::Literal(TLiteral::Bool(b)) => {
                Ok(builder.ins().iconst(types::I8, if *b { 1 } else { 0 }))
            }
            TExpr::Literal(TLiteral::Unit) => {
                Ok(builder.ins().iconst(types::I32, 0))
            }
            // TODO: Implementar outras expressões
            _ => Err(format!("Expressão não suportada em Action: {:?}", expr.0))
        }
    }

    /// Verifica se a expressão é um ponto de suspensão
    fn is_at_suspension_point(&self, expr: &Spanned<TExpr>, susp: &SuspensionPoint) -> bool {
        match (&expr.0, susp) {
            (TExpr::ChannelRecv(_, _), SuspensionPoint::ChannelRecv { .. }) => true,
            (TExpr::ChannelSend(_, _, _), SuspensionPoint::ChannelSend { .. }) => true,
            _ => false,
        }
    }

    /// Analisa os pontos de suspensão no corpo de uma Action
    fn analyze_suspension_points(&self, body: &[Spanned<TStmt>]) -> Result<ActionAnalyzer, String> {
        let mut analyzer = ActionAnalyzer::new();

        for stmt in body {
            self.find_suspensions_in_stmt(&stmt.0, &mut analyzer)?;
        }

        Ok(analyzer)
    }

    /// Encontra pontos de suspensão em um statement
    fn find_suspensions_in_stmt(&self, stmt: &TStmt, analyzer: &mut ActionAnalyzer) -> Result<(), String> {
        match stmt {
            TStmt::Let(_, expr) | TStmt::Var(_, expr) | TStmt::Expr(expr) => {
                self.find_suspensions_in_expr(&expr.0, analyzer)
            }
            TStmt::Match(_, arms) => {
                for arm in arms {
                    for s in &arm.body {
                        self.find_suspensions_in_stmt(&s.0, analyzer)?;
                    }
                }
                Ok(())
            }
            TStmt::Loop(body) | TStmt::For(_, _, body) => {
                for s in body {
                    self.find_suspensions_in_stmt(&s.0, analyzer)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Encontra pontos de suspensão em uma expressão
    fn find_suspensions_in_expr(&self, expr: &TExpr, analyzer: &mut ActionAnalyzer) -> Result<(), String> {
        match expr {
            TExpr::ChannelRecv(channel, _) => {
                let stage_before = analyzer.stage();
                let stage_after = analyzer.next_stage();
                analyzer.suspension_points.push(SuspensionPoint::ChannelRecv {
                    var_name: String::new(),
                    channel_expr: format!("{:?}", channel.0),
                    stage_before,
                    stage_after,
                });
                Ok(())
            }
            TExpr::ChannelSend(channel, value, _) => {
                let stage_before = analyzer.stage();
                let stage_after = analyzer.next_stage();
                analyzer.suspension_points.push(SuspensionPoint::ChannelSend {
                    channel_expr: format!("{:?}", channel.0),
                    value_expr: format!("{:?}", value.0),
                    stage_before,
                    stage_after,
                });
                Ok(())
            }
            TExpr::Call(callee, args, _) => {
                // Verifica se é sleep! ou outra Action
                if let TExpr::Ident(name, _) = &callee.0 {
                    if name == "sleep!" {
                        let stage_before = analyzer.stage();
                        let stage_after = analyzer.next_stage();
                        analyzer.suspension_points.push(SuspensionPoint::Sleep {
                            millis_expr: args.first().map(|a| format!("{:?}", a.0)).unwrap_or_default(),
                            stage_before,
                            stage_after,
                        });
                        return Ok(());
                    }
                }
                // Verifica argumentos recursivamente
                for arg in args {
                    self.find_suspensions_in_expr(&arg.0, analyzer)?;
                }
                Ok(())
            }
            TExpr::Sequence(exprs, _) => {
                for e in exprs {
                    self.find_suspensions_in_expr(&e.0, analyzer)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Cria o layout do estado para uma Action
    fn create_layout(
        &self,
        name: &str,
        params: &[(String, Spanned<TypeRef>)],
        analyzer: &ActionAnalyzer,
    ) -> Result<ActionStateLayout, String> {
        let mut layout = ActionStateLayout::new(name);

        // Adiciona parâmetros
        for (param_name, param_ty) in params {
            let size = self.type_size(&param_ty.0);
            let align = self.type_align(&param_ty.0);
            layout.add_var(param_name, size, align);
        }

        // Adiciona stages
        for _ in &analyzer.suspension_points {
            layout.add_stage();
        }

        layout.finalize();
        Ok(layout)
    }

    /// Retorna o tamanho em bytes de um tipo
    fn type_size(&self, ty: &TypeRef) -> usize {
        match ty {
            TypeRef::Simple(n) if n == "Int" => 8,
            TypeRef::Simple(n) if n == "Float" => 8,
            TypeRef::Simple(n) if n == "Bool" => 1,
            TypeRef::Simple(n) if n == "()" || n == "Unit" => 4,
            _ => 8, // Ponteiros
        }
    }

    /// Retorna o alinhamento de um tipo
    fn type_align(&self, ty: &TypeRef) -> usize {
        match ty {
            TypeRef::Simple(n) if n == "Bool" => 1,
            _ => 8,
        }
    }
}

impl SuspensionPoint {
    /// Retorna o stage após a retomada
    pub fn stage_after(&self) -> usize {
        match self {
            SuspensionPoint::ChannelRecv { stage_after, .. } => *stage_after,
            SuspensionPoint::ChannelSend { stage_after, .. } => *stage_after,
            SuspensionPoint::Sleep { stage_after, .. } => *stage_after,
        }
    }
}