use crate::type_checker::checker::TTopLevel;
use crate::type_checker::directives::KataDirective;
use crate::type_checker::tast::{TExpr, TLiteral};
use crate::parser::ast::{Spanned, TypeRef};
use crate::optimizer::error::OptimizerError;
use std::collections::HashMap;

pub struct MemoizePass {
    pub memoized_functions: HashMap<String, KataDirective>,
}

impl MemoizePass {
    pub fn new() -> Self {
        Self {
            memoized_functions: HashMap::new(),
        }
    }

    pub fn run(&mut self, tast: Vec<Spanned<TTopLevel>>, _errors: &mut Vec<OptimizerError>) -> Vec<Spanned<TTopLevel>> {
        log::debug!("Executando Memoization Pass (@cache_strategy)...");

        // Passo 1: Coletar funcoes que pediram cache
        for (decl, _) in &tast {
            if let TTopLevel::Signature(name, _, _, dirs) = decl {
                for (dir, _) in dirs {
                    if let KataDirective::CacheStrategy { strategy, size, ttl } = dir {
                        self.memoized_functions.insert(
                            name.clone(), 
                            KataDirective::CacheStrategy { strategy: strategy.clone(), size: *size, ttl: *ttl }
                        );
                    }
                }
            }
        }

        if self.memoized_functions.is_empty() {
            return tast;
        }

        let mut final_tast = Vec::new();
        let mut current_sig_name = None;

        let get_sig = TTopLevel::Signature(
            "kata_rt_cache_get".to_string(),
            vec![(TypeRef::Simple("Int".to_string()), 0..0)], 
            (TypeRef::Simple("Unknown".to_string()), 0..0),    
            vec![(KataDirective::Ffi("kata_rt_cache_get".to_string()), 0..0)]
        );
        let set_sig = TTopLevel::Signature(
            "kata_rt_cache_set".to_string(),
            vec![(TypeRef::Simple("Int".to_string()), 0..0), (TypeRef::Simple("Unknown".to_string()), 0..0)],
            (TypeRef::Simple("()".to_string()), 0..0),
            vec![(KataDirective::Ffi("kata_rt_cache_set".to_string()), 0..0)]
        );
        final_tast.push((get_sig, 0..0));
        final_tast.push((set_sig, 0..0));

        for (decl, span) in tast {
            match decl {
                TTopLevel::Signature(ref name, _, _, _) => {
                    current_sig_name = Some(name.clone());
                    final_tast.push((decl, span));
                }
                TTopLevel::LambdaDef(params, body, with, dirs) => {
                    if let Some(name) = &current_sig_name {
                        if self.memoized_functions.contains_key(name) {
                            let hash_val = TExpr::Literal(TLiteral::Int(name.len() as i64));
                            let hash_expr = (hash_val, span.clone());
                            
                            let get_callee = Box::new((TExpr::Ident("kata_rt_cache_get".to_string(), TypeRef::Simple("Unknown".to_string())), span.clone()));
                            let get_call = (TExpr::Call(get_callee, vec![hash_expr.clone()], TypeRef::Simple("Unknown".to_string())), span.clone());

                            let original_body = body.clone();

                            let set_callee = Box::new((TExpr::Ident("kata_rt_cache_set".to_string(), TypeRef::Simple("Unknown".to_string())), span.clone()));
                            let set_call = (TExpr::Call(set_callee, vec![hash_expr, original_body.clone()], TypeRef::Simple("()".to_string())), span.clone());

                            let ty = crate::type_checker::arity_resolver::ArityResolver::get_expr_type(&body.0);
                            let seq = TExpr::Sequence(vec![get_call, set_call, original_body], ty);

                            final_tast.push((TTopLevel::LambdaDef(params, (seq, span.clone()), with, dirs), span));
                        } else {
                            final_tast.push((TTopLevel::LambdaDef(params, body, with, dirs), span));
                        }
                    } else {
                        final_tast.push((TTopLevel::LambdaDef(params, body, with, dirs), span));
                    }
                }
                TTopLevel::ActionDef(..) => {
                    current_sig_name = None;
                    final_tast.push((decl, span));
                }
                _ => {
                    current_sig_name = None;
                    final_tast.push((decl, span));
                }
            }
        }

        final_tast
    }
}
