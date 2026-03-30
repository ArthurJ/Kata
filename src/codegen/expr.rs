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

                let func_id = self.functions.get(&name)
                    .ok_or_else(|| format!("Função `{}` não encontrada no modulo.", name))?;
                
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
