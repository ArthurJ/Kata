use crate::type_checker::checker::TTopLevel;
use crate::parser::ast::{Spanned, TypeRef};
use crate::type_checker::directives::KataDirective;
use super::context::CodegenContext;
use cranelift_codegen::ir::{AbiParam, Type as IrType, types, InstBuilder};
use cranelift_module::{Linkage, Module};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_codegen::entity::EntityRef;

pub struct FunctionTranslator<'a> {
    pub ctx: &'a mut CodegenContext,
    builder_context: FunctionBuilderContext,
}

impl<'a> FunctionTranslator<'a> {
    pub fn new(ctx: &'a mut CodegenContext) -> Self {
        Self {
            ctx,
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

    pub fn translate(&mut self, tast: Vec<Spanned<TTopLevel>>) -> Result<(), String> {
        // 1. Declare all function signatures first (functions and actions) to allow mutual recursion / out-of-order calls
        self.declare_signatures(&tast)?;

        // 2. Translate the bodies
        for (decl, _span) in tast {
            match decl {
                TTopLevel::ActionDef(name, params, ret, body, dirs) => {
                    // Compute mangled name to match what was declared
                    let mut mangled_name = name.clone();
                    for (_, p_ty) in &params {
                        if name != "main" {
                            mangled_name.push('_');
                            mangled_name.push_str(&Self::type_to_string_simple(&p_ty.0));
                        }
                    }
                    mangled_name = Self::sanitize_name(&mangled_name);
                    self.translate_action(&mangled_name, &params, &ret.0, &body, &dirs)?;
                }
                TTopLevel::LambdaDef(_params, _body, _, _) => {
                    // Requires matching the lambda back to its signature.
                    // For the MVP, we assume Monomorphization left functions with explicit names if they are generic,
                    // but the LambdaDef itself doesn't hold its name directly in our current AST structure unless it's immediately after a Signature.
                    // Since TreeShaker/Monomorphizer preserves the Signature -> LambdaDef order, we would need to track `current_sig_name`.
                    // For now, let's keep it simple and focus on translating Action `main!` as the MVP Entrypoint.
                }
                _ => {} // Other constructs (Data, Enum) are handled differently or erased by now
            }
        }

        Ok(())
    }

    fn declare_signatures(&mut self, tast: &[Spanned<TTopLevel>]) -> Result<(), String> {
        let mut current_sig_name = None;

        for (decl, _) in tast {
            match decl {
                TTopLevel::Signature(name, params, ret, dirs) => {
                    current_sig_name = Some(name.clone());
                    let mut sig = self.ctx.module.make_signature();

                    for (p_ty, _) in params {
                        sig.params.push(AbiParam::new(self.map_type(p_ty)));
                    }
                    if !matches!(ret.0, TypeRef::Simple(ref n) if n == "()" || n == "Unit") {
                        sig.returns.push(AbiParam::new(self.map_type(&ret.0)));
                    }

                    // Check if it's an FFI
                    let is_ffi = dirs.iter().any(|(d, _)| matches!(d, KataDirective::Ffi(_)));
                    let linkage = if is_ffi { Linkage::Import } else { Linkage::Export };
                    
                    let real_name = if let Some((KataDirective::Ffi(ext_name), _)) = dirs.iter().find(|(d, _)| matches!(d, KataDirective::Ffi(_))) {
                        ext_name.clone()
                    } else {
                        name.clone()
                    };

                    let func_id = self.ctx.module.declare_function(&real_name, linkage, &sig)
                        .map_err(|e| format!("Falha ao declarar a assinatura da funcao `{}`: {}", name, e))?;

                    
                    self.ctx.functions.insert(name.clone(), func_id);
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
