use crate::lexer::Token;
use crate::parser::ast::*;
use crate::parser::error::ParserError;
use crate::parser::core::tokens::ident_string;
use crate::parser::grammar::types::type_ref_parser;
use crate::parser::grammar::expr::{expr_parser, pattern_atom_parser};
use crate::parser::grammar::stmt::stmt_parser;
use chumsky::prelude::*;

pub fn top_level_parser() -> impl Parser<Token, Spanned<TopLevel>, Error = ParserError> + Clone {
    recursive(|top_level| {
        let data_decl = just(Token::Data)
            .ignore_then(ident_string())
            .then(
                ident_string()
                    .repeated()
                    .delimited_by(just(Token::LParen), just(Token::RParen))
            )
            .map(|(name, fields)| TopLevel::Data(name, fields));

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
            .then_ignore(just(Token::Newline).repeated())
            .then(
                just(Token::Indent)
                    .ignore_then(
                        variant_parser.separated_by(just(Token::Newline).repeated()).allow_trailing()
                    )
                    .then_ignore(just(Token::Dedent))
            )
            .map(|(name, variants)| TopLevel::Enum(name, variants));

        let export_decl = just(Token::Export)
            .ignore_then(ident_string().repeated())
            .map(TopLevel::Export);

        let import_decl = just(Token::Import)
            .ignore_then(ident_string().separated_by(just(Token::Dot)))
            .map(|parts| {
                let path = parts.join(".");
                TopLevel::Import(path, None)
            });

        let signature_decl = ident_string()
            .then_ignore(just(Token::DoubleColon))
            .then(type_ref_parser().repeated())
            .then_ignore(just(Token::FatArrow))
            .then(type_ref_parser())
            .map(|((name, args), ret)| TopLevel::Signature(name, args, ret));

        let action_arg = ident_string()
            .then_ignore(just(Token::DoubleColon))
            .then(type_ref_parser());

        let action_def = just(Token::Action)
            .ignore_then(ident_string())
            .then(
                action_arg.separated_by(just(Token::Comma).or_not())
                    .delimited_by(just(Token::LParen), just(Token::RParen))
            )
            .then_ignore(just(Token::FatArrow))
            .then(type_ref_parser())
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
            .map(|(((name, args), ret), stmts)| TopLevel::ActionDef(name, args, ret, stmts));

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
            .map(|((name, supertraits), methods)| TopLevel::Interface(name, supertraits, methods));

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
            .map(|(name, target)| TopLevel::Alias(name, target));

        let lambda_def = just(Token::Lambda)
            .ignore_then(pattern_atom_parser().repeated())
            .then_ignore(just(Token::Colon))
            .then(expr_parser()) 
            .map_with_span(|(args, (body, span_body)), _span| {
                TopLevel::LambdaDef(args, (body, span_body), Vec::new())
            });

        data_decl
            .or(enum_decl)
            .or(export_decl)
            .or(import_decl)
            .or(signature_decl)
            .or(lambda_def)
            .or(action_def)
            .or(interface_decl)
            .or(implements_decl)
            .or(alias_decl)
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
