use crate::lexer::Token;
use crate::parser::ast::*;
use crate::parser::error::ParserError;
use crate::parser::core::tokens::ident_string;
use crate::parser::grammar::types::type_ref_parser;
use crate::parser::grammar::expr::{expr_parser, pattern_atom_parser, with_block_parser};
use crate::parser::grammar::stmt::stmt_parser;
use chumsky::prelude::*;

pub fn top_level_parser() -> impl Parser<Token, Spanned<TopLevel>, Error = ParserError> + Clone {
    recursive(|top_level| {
        let directive_parser = filter_map(|span, tok| match tok {
            Token::Directive(s) => Ok(s),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        })
        .then(
            expr_parser().separated_by(just(Token::Comma).or_not())
                .delimited_by(just(Token::LParen), just(Token::RParen))
                .or_not()
                .map(|opt| opt.unwrap_or_default())
        )
        .map_with_span(|(name, args), span| (Directive { name, args }, span));

        let directives_list = directive_parser
            .then_ignore(just(Token::Newline).repeated())
            .repeated();

        let data_decl = just(Token::Data)
            .ignore_then(ident_string())
            .then(
                choice((
                    ident_string()
                        .repeated()
                        .delimited_by(just(Token::LParen), just(Token::RParen))
                        .map(DataDef::Struct),
                    just(Token::As)
                        .ignore_then(
                            type_ref_parser()
                                .then_ignore(just(Token::Comma).or_not())
                                .then(expr_parser().separated_by(just(Token::Comma).or_not()))
                                .delimited_by(just(Token::LParen), just(Token::RParen))
                        )
                        .map(|(base, predicates)| DataDef::Refined(base, predicates))
                ))
            );

        let variant_parser = just(Token::Pipe)
            .ignore_then(ident_string())
            .then(
                choice((
                    type_ref_parser().delimited_by(just(Token::LParen), just(Token::RParen)).map(VariantData::Type),
                    expr_parser().delimited_by(just(Token::LParen), just(Token::RParen)).map(|(e, _)| {
                        fn has_hole(e: &Expr) -> bool {
                            match e {
                                Expr::Hole => true,
                                Expr::Tuple(es) | Expr::List(es) | Expr::Array(es) | Expr::Sequence(es) => {
                                    es.iter().any(|(ex, _)| has_hole(ex))
                                }
                                _ => false,
                            }
                        }
                        if has_hole(&e) { VariantData::Predicate(e) } else { VariantData::FixedValue(e) }
                    }),
                    empty().to(VariantData::Unit)
                ))
            )
            .map(|(name, data)| Variant { name, data });

        let enum_decl = just(Token::Enum)
            .ignore_then(ident_string())
            .then(
                just(Token::DoubleColon).ignore_then(
                    choice((
                        type_ref_parser().separated_by(just(Token::Comma).or_not()).delimited_by(just(Token::LParen), just(Token::RParen)),
                        type_ref_parser().map(|t| vec![t])
                    ))
                ).or_not()
            )
            .then_ignore(just(Token::Newline).repeated())
            .then(
                just(Token::Indent)
                    .ignore_then(
                        variant_parser.separated_by(just(Token::Newline).repeated()).allow_trailing()
                    )
                    .then_ignore(just(Token::Dedent))
            )
            .map(|((name, _generics), variants)| (name, variants));

        let export_decl = just(Token::Export)
            .ignore_then(ident_string().repeated())
            .map(|v| TopLevel::Export(v));

        let import_decl = just(Token::Import)
            .ignore_then(ident_string().separated_by(just(Token::Dot)))
            .then(
                just(Token::Dot).ignore_then(
                    ident_string()
                        .then(just(Token::As).ignore_then(ident_string()).or_not())
                        .separated_by(just(Token::Comma).or_not())
                        .delimited_by(just(Token::LParen), just(Token::RParen))
                ).or_not()
            )
            .map(|(parts, specific)| {
                let path = parts.join(".");
                TopLevel::Import(path, specific.unwrap_or_default())
            });

        let signature_decl = ident_string()
            .then_ignore(just(Token::DoubleColon))
            .then(type_ref_parser().repeated())
            .then_ignore(just(Token::FatArrow))
            .then(type_ref_parser())
            .map(|((name, args), ret)| (name, args, ret));

        let action_arg = choice((
            ident_string()
                .then(just(Token::DoubleColon).ignore_then(type_ref_parser()))
                .map(|(name, ty)| (name, ty)),
            type_ref_parser().map(|t| ("_".to_string(), t)),
            ident_string().map(|name| (name, (TypeRef::Simple("Unknown".to_string()), 0..0)))
        ));

        let action_def = just(Token::Action)
            .ignore_then(ident_string())
            .then(
                action_arg.separated_by(just(Token::Comma).or_not())
                    .delimited_by(just(Token::LParen), just(Token::RParen))
                    .or_not()
                    .map(|opt: Option<Vec<(String, Spanned<TypeRef>)>>| opt.unwrap_or_default())
            )
            .then(
                just(Token::FatArrow)
                    .ignore_then(type_ref_parser())
                    .or_not()
                    .map(|opt| opt.unwrap_or((TypeRef::Simple("()".to_string()), 0..0)))
            )
            .then(
                just(Token::Newline).repeated()
                    .ignore_then(just(Token::Indent))
                    .ignore_then(
                        stmt_parser().separated_by(just(Token::Newline).repeated()).allow_trailing()
                    )
                    .then_ignore(just(Token::Dedent))
                    .or_not()
                    .map(|opt| opt.unwrap_or_default())
            )
            .map(|(((name, args), ret), stmts)| (name, args, ret, stmts));

        let interface_decl = just(Token::Interface)
            .ignore_then(ident_string())
            .then(
                just(Token::Implements).ignore_then(ident_string().repeated()).or_not().map(|opt| opt.unwrap_or_default())
            )
            .then(
                just(Token::Newline).repeated()
                    .ignore_then(just(Token::Indent))
                    .ignore_then(top_level.clone().separated_by(just(Token::Newline).repeated()).allow_trailing())
                    .then_ignore(just(Token::Dedent))
                    .or_not()
                    .map(|opt| opt.unwrap_or_default())
            )
            .map(|((name, supertraits), methods)| (name, supertraits, methods));

        let implements_decl = ident_string()
            .then_ignore(just(Token::Implements))
            .then(ident_string())
            .then(
                just(Token::Newline).repeated()
                    .ignore_then(just(Token::Indent))
                    .ignore_then(top_level.clone().separated_by(just(Token::Newline).repeated()).allow_trailing())
                    .then_ignore(just(Token::Dedent))
                    .or_not()
                    .map(|opt| opt.unwrap_or_default())
            )
            .map(|((type_name, trait_name), methods)| TopLevel::Implements(type_name, trait_name, methods));

        let alias_decl = just(Token::Alias)
            .ignore_then(ident_string())
            .then(ident_string())
            .map(|(name, target)| (name, target));

        let lambda_body = {
            let sequence = expr_parser();
            
            let guard_branch = ident_string()
                .then_ignore(just(Token::Colon))
                .then(sequence.clone());

            let otherwise_branch = just(Token::Otherwise)
                .ignore_then(just(Token::Colon))
                .ignore_then(sequence.clone());

            let guards_block = just(Token::Newline).repeated()
                .ignore_then(just(Token::Indent))
                .ignore_then(
                    guard_branch.separated_by(just(Token::Newline).repeated().at_least(1))
                        .allow_trailing()
                        .at_least(1)
                )
                .then_ignore(just(Token::Newline).repeated())
                .then(otherwise_branch)
                .then(with_block_parser())
                .then_ignore(just(Token::Dedent))
                .map_with_span(|((branches, otherwise_body), w), span| {
                    ((Expr::Guard(branches, Box::new(otherwise_body)), span), w)
                });

            let lambda_block = just(Token::Newline).repeated()
                .ignore_then(just(Token::Indent))
                .ignore_then(
                    sequence.clone()
                        .separated_by(just(Token::Newline).repeated().at_least(1))
                        .allow_trailing()
                )
                .then(with_block_parser())
                .then_ignore(just(Token::Dedent))
                .map_with_span(|(mut exprs, w), span| {
                    let body = if exprs.len() == 1 {
                        exprs.remove(0)
                    } else {
                        (Expr::Sequence(exprs), span)
                    };
                    (body, w)
                });

            let inline_body = sequence.then(with_block_parser());

            choice((guards_block, lambda_block, inline_body))
        };

        let lambda_def = just(Token::Lambda)
            .ignore_then(pattern_atom_parser().repeated())
            .then_ignore(just(Token::Colon))
            .then(lambda_body.clone())
            .map(|(args, (body, with))| (args, body, with));

        let otherwise_lambda_def = just(Token::Otherwise)
            .then_ignore(just(Token::Colon))
            .ignore_then(lambda_body)
            .map(|(body, with)| TopLevel::LambdaDef(vec![(Pattern::Ident("otherwise".to_string()), 0..0)], body, with, Vec::new()));

        let execution_decl = expr_parser().map(TopLevel::Execution);

        directives_list.then(
            choice((
                data_decl.map(|(name, def)| TopLevel::Data(name, def, Vec::new())),
                enum_decl.map(|(name, variants)| TopLevel::Enum(name, variants, Vec::new())),
                signature_decl.map(|(name, args, ret)| TopLevel::Signature(name, args, ret, Vec::new())),
                lambda_def.map(|(args, body, with)| TopLevel::LambdaDef(args, body, with, Vec::new())),
                otherwise_lambda_def,
                action_def.map(|(name, args, ret, stmts)| TopLevel::ActionDef(name, args, ret, stmts, Vec::new())),
                interface_decl.map(|(name, supers, methods)| TopLevel::Interface(name, supers, methods, Vec::new())),
                alias_decl.map(|(name, target)| TopLevel::Alias(name, target, Vec::new())),
                export_decl,
                import_decl,
                implements_decl,
                execution_decl,
            ))
        ).map(|(dirs, mut tl)| {
            match &mut tl {
                TopLevel::Data(_, _, ref mut d) => *d = dirs,
                TopLevel::Enum(_, _, ref mut d) => *d = dirs,
                TopLevel::Signature(_, _, _, ref mut d) => *d = dirs,
                TopLevel::LambdaDef(_, _, _, ref mut d) => *d = dirs,
                TopLevel::ActionDef(_, _, _, _, ref mut d) => *d = dirs,
                TopLevel::Interface(_, _, _, ref mut d) => *d = dirs,
                TopLevel::Alias(_, _, ref mut d) => *d = dirs,
                TopLevel::Export(_) | TopLevel::Import(_, _) | TopLevel::Implements(..) | TopLevel::Execution(_) => {
                    // Essas ainda não suportam diretivas na AST ou não faz sentido
                }
            }
            tl
        })
        .map_with_span(|ast, span| (ast, span))
    })
}

pub fn module_parser() -> impl Parser<Token, Module, Error = ParserError> {
    top_level_parser()
        .then_ignore(just(Token::Newline).repeated())
        .repeated()
        .then_ignore(end())
        .map(|declarations| Module { declarations })
}
