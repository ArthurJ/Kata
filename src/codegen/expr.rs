use crate::type_checker::tast::{TExpr, TLiteral, TStmt};
use crate::parser::ast::{Spanned, Pattern, TypeRef};
use cranelift_codegen::ir::{InstBuilder, Value, types, MemFlags};
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
}

impl<'a, 'b> ExprTranslator<'a, 'b> {
    /// Get the type of an expression
    fn get_expr_type(expr: &TExpr) -> TypeRef {
        match expr {
            TExpr::Literal(TLiteral::Int(_)) => TypeRef::Simple("Int".to_string()),
            TExpr::Literal(TLiteral::Float(_)) => TypeRef::Simple("Float".to_string()),
            TExpr::Literal(TLiteral::String(_)) => TypeRef::Simple("Text".to_string()),
            TExpr::Literal(TLiteral::Bool(_)) => TypeRef::Simple("Bool".to_string()),
            TExpr::Literal(TLiteral::Unit) => TypeRef::Simple("()".to_string()),
            TExpr::Ident(_, ty) | TExpr::Call(_, _, ty) | TExpr::Tuple(_, ty, _) | TExpr::List(_, ty, _) | TExpr::Lambda(_, _, ty) | TExpr::Sequence(_, ty) | TExpr::Guard(_, _, ty) | TExpr::Try(_, ty) | TExpr::ChannelSend(_, _, ty) | TExpr::ChannelRecv(_, ty) | TExpr::ChannelRecvNonBlock(_, ty) => ty.clone(),
            TExpr::Hole => TypeRef::Simple("Unknown".to_string()),
        }
    }

    pub fn translate_stmt(&mut self, stmt: &Spanned<TStmt>) -> Result<Option<Value>, String> {
        let (s, _span) = stmt;
        match s {
            TStmt::Let(pat, expr) => {
                let val = self.translate_expr(expr)?;
                self.bind_pattern(pat, val, &expr.0)?;
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
            TExpr::Literal(TLiteral::String(_)) => {
                // Para strings, criamos os dados na secao data do modulo e retornamos um ponteiro.
                // Isso requer mais manipulacao de contexto (DataDescription). 
                // Para o MVP super basico, ignoramos alocacao de strings ou retornamos 0
                Err("Strings literais ainda não são suportadas totalmente no Cranelift MVP.".to_string())
            }
            TExpr::Ident(name, _) => {
                if let Some(var) = self.variables.get(name) {
                    Ok(self.builder.use_var(*var))
                } else {
                    Err(format!("Variável não encontrada: {}", name))
                }
            }
            TExpr::Call(callee, args, _) => {
                let name = match &callee.0 {
                    TExpr::Ident(n, _) => n.clone(),
                    _ => return Err("Chamadas anonimas ou high-order nao suportadas no MVP.".to_string()),
                };

                // Extract actual arguments - the arity resolver wraps them in a Tuple
                let actual_args: Vec<&Spanned<TExpr>> = if args.len() == 1 {
                    if let TExpr::Tuple(elements, _, _) = &args[0].0 {
                        elements.iter().collect()
                    } else {
                        args.iter().collect()
                    }
                } else {
                    args.iter().collect()
                };

                // Build mangled name using argument types
                let mut mangled_name = name.clone();
                if name != "main" {
                    for arg in &actual_args {
                        let arg_ty = Self::get_expr_type(&arg.0);
                        mangled_name.push('_');
                        mangled_name.push_str(&crate::codegen::translator::FunctionTranslator::type_to_string_simple(&arg_ty));
                    }
                }
                mangled_name = crate::codegen::translator::FunctionTranslator::sanitize_name(&mangled_name);

                // Try exact match first, then try with Unknown as wildcard
                let func_id = if let Some(id) = self.functions.get(&mangled_name) {
                    *id
                } else {
                    // Try to find a function with Unknown types that match
                    let base_name = name.clone();
                    let candidate = self.functions.iter().find(|(k, _)| {
                        // Check if this function has the same base name and matches with Unknown as wildcard
                        let expected_parts: Vec<&str> = mangled_name.split('_').collect();
                        let candidate_parts: Vec<&str> = k.split('_').collect();
                        if expected_parts.len() != candidate_parts.len() {
                            return false;
                        }
                        // First part should be the base name
                        if expected_parts.is_empty() || candidate_parts.is_empty() || expected_parts[0] != base_name {
                            return false;
                        }
                        // Check if all parts match, treating "Unknown" as wildcard
                        for (exp, cand) in expected_parts.iter().zip(candidate_parts.iter()) {
                            if exp != cand && *cand != "Unknown" && *exp != "Unknown" {
                                return false;
                            }
                        }
                        true
                    });
                    match candidate {
                        Some((k, id)) => {
                            
                            *id
                        },
                        None => return Err(format!("Função `{}` não encontrada no modulo.", mangled_name)),
                    }
                };

                let local_func = self.module.declare_func_in_func(func_id, self.builder.func);

                let mut arg_vals = Vec::new();
                for arg in &actual_args {
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
            // Channel operations
            TExpr::ChannelSend(channel, value, _) => {
                self.translate_channel_send(channel, value)
            }
            TExpr::ChannelRecv(channel, _) => {
                self.translate_channel_recv(channel, true) // blocking
            }
            TExpr::ChannelRecvNonBlock(channel, _) => {
                self.translate_channel_recv(channel, false) // non-blocking
            }
            _ => Err(format!("Expressão não suportada no MVP: {:?}", e)),
        }
    }

    /// Translate channel send operation
    fn translate_channel_send(&mut self, channel: &Spanned<TExpr>, value: &Spanned<TExpr>) -> Result<Value, String> {
        let chan_val = self.translate_expr(channel)?;
        let val = self.translate_expr(value)?;

        // Get the FFI function for channel send
        let func_id = self.functions.get("kata_rt_channel_send_async")
            .ok_or_else(|| "Função kata_rt_channel_send_async não encontrada".to_string())?;
        let local_func = self.module.declare_func_in_func(*func_id, self.builder.func);

        // For now, use null waker (blocking send)
        let waker_null = self.builder.ins().iconst(types::I64, 0);

        // Call: kata_rt_channel_send_async(sender, value, waker) -> bool
        let call = self.builder.ins().call(local_func, &[chan_val, val, waker_null]);
        let results = self.builder.inst_results(call);

        Ok(results.get(0).copied().unwrap_or_else(|| self.builder.ins().iconst(types::I32, 0)))
    }

    /// Translate channel receive operation
    fn translate_channel_recv(&mut self, channel: &Spanned<TExpr>, blocking: bool) -> Result<Value, String> {
        let chan_val = self.translate_expr(channel)?;

        let func_name = if blocking {
            "kata_rt_channel_recv_async"
        } else {
            "kata_rt_channel_recv_try"
        };

        let func_id = self.functions.get(func_name)
            .ok_or_else(|| format!("Função {} não encontrada", func_name))?;
        let local_func = self.module.declare_func_in_func(*func_id, self.builder.func);

        let call = if blocking {
            // For blocking, use null waker
            let waker_null = self.builder.ins().iconst(types::I64, 0);
            self.builder.ins().call(local_func, &[chan_val, waker_null])
        } else {
            // For non-blocking, just pass the channel
            self.builder.ins().call(local_func, &[chan_val])
        };

        let results = self.builder.inst_results(call);
        Ok(results.get(0).copied().unwrap_or_else(|| self.builder.ins().iconst(types::I64, -1)))
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

    /// Bind a pattern to a value, supporting tuple destructuring
    fn bind_pattern(&mut self, pat: &Spanned<Pattern>, val: Value, expr_ty: &TExpr) -> Result<(), String> {
        match &pat.0 {
            Pattern::Ident(name) => {
                // Simple binding: let x = expr
                let ty = match expr_ty {
                    TExpr::Literal(TLiteral::Int(_)) => types::I64,
                    TExpr::Literal(TLiteral::Float(_)) => types::F64,
                    TExpr::Literal(TLiteral::Bool(_)) => types::I8,
                    TExpr::Call(_, _, ty) | TExpr::Ident(_, ty) | TExpr::Tuple(_, ty, _) => self.map_type(ty),
                    _ => types::I64,
                };
                let var = Variable::from_u32(*self.var_index as u32);
                *self.var_index += 1;
                self.builder.declare_var(var, ty);
                self.builder.def_var(var, val);
                self.variables.insert(name.clone(), var);
                Ok(())
            }
            Pattern::Tuple(patterns) => {
                // Tuple destructuring: let (a, b, c) = expr
                let ptr_type = self.module.target_config().pointer_type();
                for (i, inner_pat) in patterns.iter().enumerate() {
                    let offset = (i * 8) as i32; // Assume 64-bit pointers
                    let elem_ptr = if offset == 0 {
                        val
                    } else {
                        self.builder.ins().iadd_imm(val, offset as i64)
                    };
                    let elem_val = self.builder.ins().load(types::I64, MemFlags::new(), elem_ptr, 0);
                    self.bind_pattern(inner_pat, elem_val, &TExpr::Literal(TLiteral::Int(0)))?;
                }
                Ok(())
            }
            Pattern::Hole => Ok(()), // _ pattern, discard the value
            _ => Err(format!("Pattern não suportado no MVP: {:?}", pat.0)),
        }
    }
}
