    fn collect_top_level(&mut self, decl: &TopLevel) {
        let mut local_errs = Vec::new();

        let validate_directives = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> Vec<String> {
            let mut errs = Vec::new();
            for (dir, _) in dirs {
                if dir.name == "log" {
                    if let Some((arg, _)) = dir.args.first() {
                        if let Expr::Ident(level) = arg {
                            let valid_levels = ["error", "warn", "info", "debug", "trace"];
                            if !valid_levels.contains(&level.as_str()) {
                                errs.push(format!("Diretiva @log invalida: Nivel de log '{}' desconhecido. Use um de {:?}", level, valid_levels));
                            }
                        }
                    }
                }
            }
            errs
        };

        let extract_test_desc = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> Option<String> {
            for (dir, _) in dirs {
                if dir.name == "test" {
                    if let Some((Expr::String(desc), _)) = dir.args.first() {
                        return Some(desc.clone());
                    }
                    return Some("Sem descricao".to_string());
                }
            }
            None
        };

        let is_commutative = |dirs: &[Spanned<crate::parser::ast::Directive>]| -> bool {
            dirs.iter().any(|(d, _)| d.name == "commutative")
        };

        match decl {
            TopLevel::Data(name, fields, dirs) => {
                local_errs.extend(validate_directives(dirs));
                self.env.define(name.clone(), fields.len(), TypeRef::Simple(name.clone()), false, false);
            }
            TopLevel::Enum(name, variants, dirs) => {
                local_errs.extend(validate_directives(dirs));
                let mut has_smart_constructor = false;
                let mut has_predicate = false;
                let mut var_names = Vec::new();

                for variant in variants {
                    var_names.push(variant.name.clone());
                    let arity = match &variant.data {
                        crate::parser::ast::VariantData::Unit => 0,
                        crate::parser::ast::VariantData::Type(_) => 1,
                        crate::parser::ast::VariantData::FixedValue(_) => { has_smart_constructor = true; 0 },
                        crate::parser::ast::VariantData::Predicate(_) => { has_smart_constructor = true; has_predicate = true; 1 },
                    };
                    self.env.define(variant.name.clone(), arity, TypeRef::Simple(name.clone()), false, false);
                }

                if has_predicate {
                    let len = variants.len();
                    for (i, variant) in variants.iter().enumerate() {
                        let is_pred = matches!(variant.data, crate::parser::ast::VariantData::Predicate(_));
                        if i < len - 1 && !is_pred {
                            local_errs.push(format!("Enum Predicativo Invalido ({}): Variante '{}' no meio da declaracao nao possui predicado.", name, variant.name));
                        } else if i == len - 1 && is_pred {
                            local_errs.push(format!("Enum Predicativo Invalido ({}): A ultima variante ('{}') deve ser um catch-all sem predicado.", name, variant.name));
                        }
                    }
                }

                self.env.define_enum(name.clone(), var_names);

                if has_smart_constructor {
                    self.env.define(
                        name.clone(),
                        1,
                        TypeRef::Function(
                            vec![(TypeRef::Simple("Any".to_string()), 0..0)],
                            Box::new((TypeRef::Simple(name.clone()), 0..0))
                        ),
                        false,
                        false
                    );
                }
            }
            TopLevel::Signature(name, params, ret, dirs) => {
                local_errs.extend(validate_directives(dirs));
                if let Some(desc) = extract_test_desc(dirs) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: false });
                }
                self.env.define(name.clone(), params.len(), TypeRef::Function(params.clone(), Box::new(ret.clone())), false, is_commutative(dirs));
            }
            TopLevel::ActionDef(name, params, ret, _, dirs) => {
                local_errs.extend(validate_directives(dirs));
                if let Some(desc) = extract_test_desc(dirs) {
                    self.tests.push(TestInfo { name: name.clone(), description: desc, is_action: true });
                }
                self.env.define(
                    name.clone(), 
                    params.len(), 
                    TypeRef::Function(params.iter().map(|(_, t)| t.clone()).collect(), Box::new(ret.clone())), 
                    true,
                    is_commutative(dirs)
                );
            }
            TopLevel::Interface(_, _, methods, _) => {
                for (m, _span) in methods {
                    self.collect_top_level(m);
                }
            }
            TopLevel::Implements(_, _, methods) => {
                for (m, _span) in methods {
                    self.collect_top_level(m);
                }
            }
            _ => {}
        }

        self.errors.extend(local_errs);
    }
