use crate::type_checker::checker::TTopLevel;
use crate::parser::ast::{Spanned, TypeRef};
use crate::type_checker::directives::KataDirective;
use super::context::CodegenContext;
use cranelift_codegen::ir::{AbiParam, Type as IrType, types, InstBuilder};
use cranelift_module::{Linkage, Module};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_codegen::entity::EntityRef;

use crate::type_checker::environment::TypeEnv;

pub struct FunctionTranslator<'a> {
    pub ctx: &'a mut CodegenContext,
    pub env: &'a TypeEnv,
    builder_context: FunctionBuilderContext,
}

impl<'a> FunctionTranslator<'a> {
    pub fn new(ctx: &'a mut CodegenContext, env: &'a TypeEnv) -> Self {
        Self {
            ctx,
            env,
            builder_context: FunctionBuilderContext::new(),
        }
    }

    pub fn map_type(&self, ty: &TypeRef) -> IrType {
        match ty {
            TypeRef::Simple(n) if n == "Int" => types::I64,
            TypeRef::Simple(n) if n == "Float" => types::F64,
            TypeRef::Simple(n) if n == "Bool" => types::I8,
            TypeRef::Simple(n) if n == "()" || n == "Unit" => types::I32, // Cranelift doesn't really have a Unit type in return signatures naturally, we return 0.
            _ => types::I64, // Pointers or unknown sizes fall back to 64-bit words
        }
    }

    pub fn translate(&mut self, tast: Vec<Spanned<TTopLevel>>) -> Result<(), String> {
        self.declare_signatures(&tast)?;

        let mut current_sig_name: Option<String> = None;
        let mut current_mangled_name: Option<String> = None;
        let mut current_sig_params: Option<Vec<Spanned<TypeRef>>> = None;
        let mut current_sig_ret: Option<TypeRef> = None;
        let mut current_lambdas = Vec::new();

        let mut compile_lambdas = |mangled_name: &str, sig_params: &[Spanned<TypeRef>], sig_ret: &TypeRef, lambdas: &[(Vec<Spanned<crate::parser::ast::Pattern>>, crate::type_checker::tast::TExpr)], env: &TypeEnv, ctx: &mut CodegenContext, builder_context: &mut FunctionBuilderContext| -> Result<(), String> {
            if lambdas.is_empty() { return Ok(()); }
            
            let func_id = ctx.functions.get(mangled_name).ok_or_else(|| format!("ID não encontrado: {}", mangled_name))?;
            let mut cl_ctx = Context::new();
            let func_decl = ctx.module.declarations().get_function_decl(*func_id);
            cl_ctx.func.signature = func_decl.signature.clone();

            let mut param_ir_types = Vec::new();
            for (p_ty, _) in sig_params {
                param_ir_types.push(
                    match p_ty {
                        TypeRef::Simple(n) if n == "Int" => types::I64,
                        TypeRef::Simple(n) if n == "Float" => types::F64,
                        TypeRef::Simple(n) if n == "Bool" => types::I8,
                        TypeRef::Simple(n) if n == "()" || n == "Unit" => types::I32,
                        _ => types::I64,
                    }
                );
            }

            let is_unit_ret = matches!(sig_ret, TypeRef::Simple(ref n) if n == "()" || n == "Unit");
            let mut builder = FunctionBuilder::new(&mut cl_ctx.func, builder_context);
            
            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let mut next_block = builder.create_block();
            let mut var_index = 0;

            for (pat_idx, (lambda_params, body)) in lambdas.iter().enumerate() {
                let current_block = if pat_idx == 0 { entry_block } else { next_block };
                next_block = builder.create_block();

                if pat_idx > 0 {
                    builder.switch_to_block(current_block);
                    builder.seal_block(current_block);
                }

                let mut is_match = None;
                for (i, (pat, _)) in lambda_params.iter().enumerate() {
                    let arg_val = builder.block_params(entry_block)[i];
                    match pat {
                        crate::parser::ast::Pattern::Literal(crate::parser::ast::Expr::Int(val_str)) => {
                            let val_i64 = val_str.parse::<i64>().unwrap_or(0);
                            let const_val = builder.ins().iconst(types::I64, val_i64);
                            let cmp = builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, arg_val, const_val);
                            is_match = if let Some(prev) = is_match { Some(builder.ins().band(prev, cmp)) } else { Some(cmp) };
                        }
                        crate::parser::ast::Pattern::Ident(name) => {
                            if name != "otherwise" {
                                let type_name = match &sig_params[i].0 {
                                    TypeRef::Simple(n) => n.clone(),
                                    _ => "".to_string(),
                                };
                                if let Some(variants) = env.enums.get(&type_name) {
                                    if let Some(pos) = variants.iter().position(|v| v == name) {
                                        let const_val = builder.ins().iconst(types::I8, pos as i64);
                                        let cmp = builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, arg_val, const_val);
                                        is_match = if let Some(prev) = is_match { Some(builder.ins().band(prev, cmp)) } else { Some(cmp) };
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let match_body_block = builder.create_block();
                if let Some(cond) = is_match {
                    builder.ins().brif(cond, match_body_block, &[], next_block, &[]);
                } else {
                    builder.ins().jump(match_body_block, &[]);
                }

                builder.switch_to_block(match_body_block);
                builder.seal_block(match_body_block);

                let mut variables = std::collections::HashMap::new();
                for (i, (pat, _)) in lambda_params.iter().enumerate() {
                    if let crate::parser::ast::Pattern::Ident(name) = pat {
                        let type_name = match &sig_params[i].0 { TypeRef::Simple(n) => n.clone(), _ => "".to_string() };
                        let is_enum_variant = env.enums.get(&type_name).map_or(false, |vars| vars.contains(name));
                        if name != "otherwise" && !is_enum_variant {
                            let val = builder.block_params(entry_block)[i];
                            let var = cranelift_frontend::Variable::from_u32(var_index as u32);
                            builder.declare_var(var, param_ir_types[i]);
                            builder.def_var(var, val);
                            variables.insert(name.clone(), var);
                            var_index += 1;
                        }
                    }
                }

                let mut expr_translator = crate::codegen::expr::ExprTranslator {
                    builder: &mut builder,
                    module: &mut ctx.module,
                    functions: &ctx.functions,
                    variables: &mut variables,
                    var_index: &mut var_index,
                    env,
                };
                let result_val = expr_translator.translate_expr(&(body.clone(), 0..0))?;

                if is_unit_ret {
                    expr_translator.builder.ins().return_(&[]);
                } else {
                    expr_translator.builder.ins().return_(&[result_val]);
                }
            }

            builder.switch_to_block(next_block);
            builder.seal_block(next_block);
            builder.ins().trap(cranelift_codegen::ir::TrapCode::User(0));
            builder.finalize();

            ctx.module.define_function(*func_id, &mut cl_ctx).map_err(|e| format!("Falha: {}", e))?;
            ctx.module.clear_context(&mut cl_ctx);
            Ok(())
        };

        for (decl, _span) in tast {
            match decl {
                TTopLevel::Signature(name, params, ret, _) => {
                    if !current_lambdas.is_empty() {
                        if let (Some(mangled_name), Some(sig_params), Some(sig_ret)) = (&current_mangled_name, &current_sig_params, &current_sig_ret) {
                            compile_lambdas(mangled_name, sig_params, sig_ret, &current_lambdas, self.env, self.ctx, &mut self.builder_context)?;
                        }
                        current_lambdas.clear();
                    }

                    let mut mangled_name = name.clone();
                    for (p_ty, _) in &params {
                        if name != "main" {
                            mangled_name.push('_');
                            mangled_name.push_str(&Self::type_to_string_simple(p_ty));
                        }
                    }
                    mangled_name = Self::sanitize_name(&mangled_name);
                    current_sig_name = Some(name.clone());
                    current_mangled_name = Some(mangled_name);
                    current_sig_params = Some(params.clone());
                    current_sig_ret = Some(ret.0.clone());
                }
                TTopLevel::LambdaDef(params, body, _with, _dirs) => {
                    current_lambdas.push((params, body.0));
                }
                TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                    if !current_lambdas.is_empty() {
                        if let (Some(mangled_name), Some(sig_params), Some(sig_ret)) = (&current_mangled_name, &current_sig_params, &current_sig_ret) {
                            compile_lambdas(mangled_name, sig_params, sig_ret, &current_lambdas, self.env, self.ctx, &mut self.builder_context)?;
                        }
                        current_lambdas.clear();
                    }
                    
                    let mut mangled_name = name.clone();
                    for (_, p_ty) in &params {
                        if name != "main" {
                            mangled_name.push('_');
                            mangled_name.push_str(&Self::type_to_string_simple(&p_ty.0));
                        }
                    }
                    mangled_name = Self::sanitize_name(&mangled_name);

                    current_sig_name = None;
                    current_mangled_name = None;
                    current_sig_params = None;
                    current_sig_ret = None;
                    self.translate_action(&mangled_name, &params, &ret.0, &body, &dirs)?;
                }
                _ => {}
            }
        }
        
        if !current_lambdas.is_empty() {
            if let (Some(mangled_name), Some(sig_params), Some(sig_ret)) = (&current_mangled_name, &current_sig_params, &current_sig_ret) {
                compile_lambdas(mangled_name, sig_params, sig_ret, &current_lambdas, self.env, self.ctx, &mut self.builder_context)?;
            }
            current_lambdas.clear();
        }

        Ok(())
    }

    pub fn type_to_string_simple(ty: &TypeRef) -> String {
        match ty {
            TypeRef::Simple(n) => n.clone(),
            TypeRef::Generic(n, args) => {
                let args_str: Vec<String> = args.iter().map(|a| Self::type_to_string_simple(&a.0)).collect();
                format!("{}_{}", n, args_str.join("_"))
            }
            TypeRef::Function(_, _) => "Func".to_string(),
            TypeRef::Refined(base, _) => format!("Refined_{}", Self::type_to_string_simple(&base.0)),
            TypeRef::Variadic(inner) => format!("Var_{}", Self::type_to_string_simple(&inner.0)),
            TypeRef::Const(expr) => match expr {
                crate::parser::ast::Expr::Int(i) => format!("ConstInt_{}", i),
                crate::parser::ast::Expr::Float(f) => format!("ConstFloat_{}", f.replace(".", "_")),
                crate::parser::ast::Expr::String(s) => format!("ConstStr_{}", s),
                crate::parser::ast::Expr::Ident(id) => format!("ConstId_{}", id),
                _ => "ConstUnknown".to_string(),
            },
        }
    }

    pub fn sanitize_name(name: &str) -> String {
        name.replace("+", "add")
            .replace("-", "sub")
            .replace("**", "pow")
            .replace("*", "mul")
            .replace("//", "idiv")
            .replace("/", "div")
            .replace("!=", "neq")
            .replace("=", "eq")
            .replace(">=", "ge")
            .replace("<=", "le")
            .replace(">", "gt")
            .replace("<", "lt")
            .replace("!", "bang")
    }

    fn declare_signatures(&mut self, tast: &[Spanned<TTopLevel>]) -> Result<(), String> {
        let ptr_type = self.ctx.module.target_config().pointer_type();

        let mut alloc_sig = self.ctx.module.make_signature();
        alloc_sig.params.push(AbiParam::new(ptr_type));
        alloc_sig.params.push(AbiParam::new(ptr_type));
        alloc_sig.returns.push(AbiParam::new(ptr_type));
        
        if !self.ctx.functions.contains_key("kata_rt_alloc_local") {
            let local_id = self.ctx.module.declare_function("kata_rt_alloc_local", Linkage::Import, &alloc_sig)
                .map_err(|e| format!("Falha: {}", e))?;
            self.ctx.functions.insert("kata_rt_alloc_local".to_string(), local_id);
        }

        if !self.ctx.functions.contains_key("kata_rt_alloc_shared") {
            let shared_id = self.ctx.module.declare_function("kata_rt_alloc_shared", Linkage::Import, &alloc_sig)
                .map_err(|e| format!("Falha: {}", e))?;
            self.ctx.functions.insert("kata_rt_alloc_shared".to_string(), shared_id);
        }

        let mut current_sig_name = None;

        for (decl, _) in tast {
            match decl {
                TTopLevel::Signature(name, params, ret, dirs) => {
                    current_sig_name = Some(name.clone());
                    let mut sig = self.ctx.module.make_signature();

                    let mut mangled_name = name.clone();

                    for (p_ty, _) in params {
                        sig.params.push(AbiParam::new(self.map_type(p_ty)));
                        if name != "main" {
                            mangled_name.push('_');
                            mangled_name.push_str(&Self::type_to_string_simple(p_ty));
                        }
                    }
                    mangled_name = Self::sanitize_name(&mangled_name);

                    if !matches!(ret.0, TypeRef::Simple(ref n) if n == "()" || n == "Unit") {
                        sig.returns.push(AbiParam::new(self.map_type(&ret.0)));
                    }

                    // Check if it's an FFI
                    let is_ffi = dirs.iter().any(|(d, _)| matches!(d, KataDirective::Ffi(_)));
                    let linkage = if is_ffi { Linkage::Import } else { Linkage::Export };

                    let real_name = if let Some((KataDirective::Ffi(ext_name), _)) = dirs.iter().find(|(d, _)| matches!(d, KataDirective::Ffi(_))) {
                        ext_name.clone()
                    } else {
                        mangled_name.clone()
                    };

                    log::error!("DEBUG: Inserting function {} -> {}", name, mangled_name);

                    let func_id = self.ctx.module.declare_function(&real_name, linkage, &sig)
                        .map_err(|e| format!("Falha ao declarar a assinatura da funcao `{}`: {}", name, e))?;

                    self.ctx.functions.insert(mangled_name, func_id);
                }
                TTopLevel::ActionDef(name, params, ret, _, _) => {
                    current_sig_name = None;
                    let mut sig = self.ctx.module.make_signature();

                    let mut mangled_name = name.clone();

                    for (_, p_ty) in params {
                        sig.params.push(AbiParam::new(self.map_type(&p_ty.0)));
                        if name != "main" {
                            mangled_name.push('_');
                            mangled_name.push_str(&Self::type_to_string_simple(&p_ty.0));
                        }
                    }
                    mangled_name = Self::sanitize_name(&mangled_name);

                    if !matches!(ret.0, TypeRef::Simple(ref n) if n == "()" || n == "Unit") {
                        sig.returns.push(AbiParam::new(self.map_type(&ret.0)));
                    }

                    // Actions are almost always exported or local
                    let export_name = if name == "main" { "kata_main".to_string() } else { mangled_name.clone() };
                    let func_id = self.ctx.module.declare_function(&export_name, Linkage::Export, &sig)
                        .map_err(|e| format!("Falha ao declarar a assinatura da Action `{}`: {}", name, e))?;

                    self.ctx.functions.insert(mangled_name, func_id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn translate_action(
        &mut self,
        name: &str,
        params: &[(String, Spanned<TypeRef>)],
        ret_ty: &TypeRef,
        body: &[Spanned<crate::type_checker::tast::TStmt>],
        _dirs: &[Spanned<KataDirective>]
    ) -> Result<(), String> {
        let func_id = self.ctx.functions.get(name)
            .ok_or_else(|| format!("ID da funcao não encontrado para `{}`", name))?;

        let mut cl_ctx = Context::new();
        let func_decl = self.ctx.module.declarations().get_function_decl(*func_id);
        cl_ctx.func.signature = func_decl.signature.clone();

        // Pré-calcular os tipos das variáveis para não emprestar `self` iterativamente
        let mut param_ir_types = Vec::new();
        for (_, p_ty) in params {
            param_ir_types.push(self.map_type(&p_ty.0));
        }
        let ret_ir_type = self.map_type(ret_ty);

        let mut builder = FunctionBuilder::new(&mut cl_ctx.func, &mut self.builder_context);
        
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let mut var_index = 0;
        let mut variables = std::collections::HashMap::new();

        // Register parameters as Cranelift variables
        for (i, (p_name, _)) in params.iter().enumerate() {
            let val = builder.block_params(entry_block)[i];
            let var = cranelift_frontend::Variable::from_u32(var_index as u32);
            builder.declare_var(var, param_ir_types[i]);
            builder.def_var(var, val);
            variables.insert(p_name.clone(), var);
            var_index += 1;
        }

        // Translate statements
        let mut expr_translator = crate::codegen::expr::ExprTranslator {
            builder: &mut builder,
            module: &mut self.ctx.module,
            functions: &self.ctx.functions,
            variables: &mut variables,
            var_index: &mut var_index,
            env: self.env,
        };

        let mut last_val = None;
        for stmt in body {
            last_val = expr_translator.translate_stmt(stmt)?;
        }

        // Return logic
        if matches!(ret_ty, TypeRef::Simple(ref n) if n == "()" || n == "Unit") {
            expr_translator.builder.ins().return_(&[]);
        } else {
            if let Some(val) = last_val {
                expr_translator.builder.ins().return_(&[val]);
            } else {
                // Fallback (should have been caught by TypeChecker)
                let zero = expr_translator.builder.ins().iconst(ret_ir_type, 0);
                expr_translator.builder.ins().return_(&[zero]);
            }
        }

        builder.finalize();

        self.ctx.module.define_function(*func_id, &mut cl_ctx)
            .map_err(|e| format!("Falha ao compilar a funcao `{}`: {}", name, e))?;

        self.ctx.module.clear_context(&mut cl_ctx);

        Ok(())
    }

    }
