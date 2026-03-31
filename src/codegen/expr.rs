use crate::type_checker::tast::{TExpr, TLiteral, TStmt};
use crate::parser::ast::{Spanned, Pattern, TypeRef};
use cranelift_codegen::ir::{InstBuilder, Value, types};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{Module, FuncId};
use cranelift_object::ObjectModule;
use std::collections::HashMap;

use cranelift_codegen::entity::EntityRef;

pub struct ExprTranslator<'a, 'b> {
    pub builder: &'a mut FunctionBuilder<'b>,
    pub module: &'a mut ObjectModule,
    pub functions: &'a HashMap<String, FuncId>,
    pub variables: &'a mut HashMap<String, Variable>,
    pub var_index: &'a mut usize,
    pub env: &'a crate::type_checker::environment::TypeEnv,
}

impl<'a, 'b> ExprTranslator<'a, 'b> {
    pub fn translate_stmt(&mut self, stmt: &Spanned<TStmt>) -> Result<Option<Value>, String> {
        let (s, _span) = stmt;
        match s {
            TStmt::Let(pat, expr) => {
                let val = self.translate_expr(expr)?;
                let name = match &pat.0 {
                    Pattern::Ident(n) => n.clone(),
                    _ => return Err("Apenas binding simples (let nome = expr) e suportado no MVP AOT.".to_string()),
                };

                let ty = match &expr.0 {
                    TExpr::Literal(TLiteral::Int(_)) => types::I64,
                    TExpr::Literal(TLiteral::Float(_)) => types::F64,
                    TExpr::Literal(TLiteral::Bool(_)) => types::I8,
                    TExpr::Call(_, _, ty) | TExpr::Ident(_, ty) => self.map_type(ty),
                    _ => types::I64,
                };

                let var = Variable::from_u32(*self.var_index as u32);
                *self.var_index += 1;
                self.builder.declare_var(var, ty);
                self.builder.def_var(var, val);
                self.variables.insert(name, var);
                
                Ok(None)
            }
            TStmt::Var(name, expr) => {
                let val = self.translate_expr(expr)?;

                let ty = match &expr.0 {
                    TExpr::Literal(TLiteral::Int(_)) => types::I64,
                    TExpr::Literal(TLiteral::Float(_)) => types::F64,
                    TExpr::Literal(TLiteral::Bool(_)) => types::I8,
                    TExpr::Call(_, _, ty) | TExpr::Ident(_, ty) => self.map_type(ty),
                    _ => types::I64,
                };

                let var = Variable::from_u32(*self.var_index as u32);
                *self.var_index += 1;
                self.builder.declare_var(var, ty);
                self.builder.def_var(var, val);
                self.variables.insert(name.clone(), var);
                
                Ok(None)
            }
            TStmt::Expr(expr) => {
                let val = self.translate_expr(expr)?;
                Ok(Some(val))
            }
            TStmt::Match(target, arms) => {
                let target_val = self.translate_expr(target)?;
                
                let mut next_cond_block = self.builder.create_block();
                let end_block = self.builder.create_block();

                self.builder.ins().jump(next_cond_block, &[]);

                for arm in arms {
                    self.builder.switch_to_block(next_cond_block);
                    self.builder.seal_block(next_cond_block);
                    
                    let arm_body_block = self.builder.create_block();
                    next_cond_block = self.builder.create_block();

                    let mut is_match = None;
                    match &arm.pattern.0 {
                        crate::parser::ast::Pattern::Ident(name) => {
                            if name == "otherwise" {
                                let true_val = self.builder.ins().iconst(types::I8, 1);
                                is_match = Some(true_val);
                            } else if name == "True" {
                                let const_val = self.builder.ins().iconst(types::I8, 1);
                                is_match = Some(self.builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, target_val, const_val));
                            } else if name == "False" {
                                let const_val = self.builder.ins().iconst(types::I8, 0);
                                is_match = Some(self.builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, target_val, const_val));
                            } else {
                                // Default binding for MVP (catch-all)
                                let true_val = self.builder.ins().iconst(types::I8, 1);
                                is_match = Some(true_val);
                            }
                        }
                        crate::parser::ast::Pattern::Literal(crate::parser::ast::Expr::Int(val_str)) => {
                            let val_i64 = val_str.parse::<i64>().unwrap_or(0);
                            let const_val = self.builder.ins().iconst(types::I64, val_i64);
                            is_match = Some(self.builder.ins().icmp(cranelift_codegen::ir::condcodes::IntCC::Equal, target_val, const_val));
                        }
                        _ => {}
                    }
                    
                    if let Some(cond) = is_match {
                        self.builder.ins().brif(cond, arm_body_block, &[], next_cond_block, &[]);
                    } else {
                        self.builder.ins().jump(arm_body_block, &[]);
                    }

                    self.builder.switch_to_block(arm_body_block);
                    self.builder.seal_block(arm_body_block);

                    // Translate body
                    for stmt in &arm.body {
                        self.translate_stmt(stmt)?;
                    }

                    self.builder.ins().jump(end_block, &[]);
                }

                self.builder.switch_to_block(next_cond_block);
                self.builder.seal_block(next_cond_block);
                self.builder.ins().trap(cranelift_codegen::ir::TrapCode::User(0));

                self.builder.switch_to_block(end_block);
                self.builder.seal_block(end_block);

                Ok(None)
            }
            _ => Err(format!("Statement não suportado no MVP: {:?}", s)),
        }
    }

    pub fn translate_expr(&mut self, expr: &Spanned<TExpr>) -> Result<Value, String> {
        let (e, _span) = expr;
        match e {
            TExpr::Literal(TLiteral::Int(i)) => Ok(self.builder.ins().iconst(types::I64, *i as i64)),
            TExpr::Literal(TLiteral::Float(f)) => Ok(self.builder.ins().f64const(*f)),
            TExpr::Literal(TLiteral::Bool(b)) => {
                let i = if *b { 1 } else { 0 };
                Ok(self.builder.ins().iconst(types::I8, i))
            },
            TExpr::Literal(TLiteral::Unit) => Ok(self.builder.ins().iconst(types::I32, 0)),
            TExpr::Literal(TLiteral::String(s)) => {
                let mut data_ctx = cranelift_module::DataDescription::new();
                let mut bytes = s.clone().into_bytes();
                bytes.push(0); // Null terminator for C compatibility
                data_ctx.define(bytes.into_boxed_slice());

                let data_id = self.module.declare_anonymous_data(true, false)
                    .map_err(|e| format!("Falha ao declarar string literal: {}", e))?;

                self.module.define_data(data_id, &data_ctx)
                    .map_err(|e| format!("Falha ao definir string literal: {}", e))?;

                let local_id = self.module.declare_data_in_func(data_id, self.builder.func);
                let ptr_type = self.module.target_config().pointer_type();
                Ok(self.builder.ins().symbol_value(ptr_type, local_id))
            }
            TExpr::Ident(name, ty) => {
                if let Some(var) = self.variables.get(name) {
                    Ok(self.builder.use_var(*var))
                } else {
                    let type_name = match ty {
                        TypeRef::Simple(n) => n.clone(),
                        _ => "".to_string(),
                    };
                    if let Some(variants) = self.env.enums.get(&type_name) {
                        if let Some(pos) = variants.iter().position(|v| v == name) {
                            return Ok(self.builder.ins().iconst(types::I8, pos as i64));
                        }
                    }
                    Err(format!("Variável ou Variante não encontrada: {}", name))
                }
            }
            TExpr::Call(callee, args, _) => {
                let (name, callee_ty) = match &callee.0 {
                    TExpr::Ident(n, ty) => (n.clone(), ty.clone()),
                    _ => return Err("Chamadas anonimas ou high-order nao suportadas no MVP.".to_string()),
                };

                let mut mangled_name = name.clone();
                if name != "main" {
                    if let crate::parser::ast::TypeRef::Function(params, _) = &callee_ty {
                        for (p_ty, _) in params {
                            mangled_name.push('_');
                            mangled_name.push_str(&crate::codegen::translator::FunctionTranslator::type_to_string_simple(p_ty));
                        }
                    }
                }
                mangled_name = crate::codegen::translator::FunctionTranslator::sanitize_name(&mangled_name);

                let func_id = self.functions.get(&mangled_name)
                    .ok_or_else(|| format!("Função `{}` não encontrada no modulo.", mangled_name))?;
                
                let local_func = self.module.declare_func_in_func(*func_id, self.builder.func);
                
                let mut arg_vals = Vec::new();
                for arg in args {
                    arg_vals.push(self.translate_expr(arg)?);
                }

                let call = self.builder.ins().call(local_func, &arg_vals);
                let results = self.builder.inst_results(call);
                if results.is_empty() {
                    Ok(self.builder.ins().iconst(types::I32, 0)) // Unit
                } else {
                    Ok(results[0])
                }
            }
            TExpr::Sequence(exprs, _) => {
                let mut last_val = None;
                for ex in exprs {
                    last_val = Some(self.translate_expr(ex)?);
                }
                Ok(last_val.unwrap_or_else(|| self.builder.ins().iconst(types::I32, 0))) // Unit fallback
            }
            TExpr::Tuple(exprs, _, _) => {
                if exprs.is_empty() {
                    Ok(self.builder.ins().iconst(types::I32, 0)) // Unit
                } else {
                    Err(format!("Tuplas com elementos não suportadas no MVP: {:?}", e))
                }
            }
            _ => Err(format!("Expressão não suportada no MVP: {:?}", e)),
        }
    }

    fn map_type(&self, ty: &TypeRef) -> cranelift_codegen::ir::Type {
        match ty {
            TypeRef::Simple(n) if n == "Int" => types::I64,
            TypeRef::Simple(n) if n == "Float" => types::F64,
            TypeRef::Simple(n) if n == "Bool" => types::I8,
            TypeRef::Simple(n) if n == "()" || n == "Unit" => types::I32,
            _ => types::I64,
        }
    }
}
