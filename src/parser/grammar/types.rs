use crate::lexer::Token;
use crate::parser::ast::*;
use crate::parser::error::ParserError;
use chumsky::prelude::*;

pub fn type_ref_parser() -> impl Parser<Token, Spanned<TypeRef>, Error = ParserError> + Clone {
    recursive(|type_ref| {
        let simple_type = filter_map(|span, tok| match tok {
            Token::TypeID(s) => Ok(TypeRef::Simple(s)),
            Token::InterfaceID(s) => Ok(TypeRef::Simple(s)),
            Token::TypeVar(s) => Ok(TypeRef::Simple(s)),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let generic_args = choice((
            type_ref.clone()
                .separated_by(just(Token::Comma).or_not())
                .delimited_by(just(Token::LParen), just(Token::RParen)),
            type_ref.clone().map(|t| vec![t])
        ));

        let generic_type = filter_map(|span, tok| match tok {
            Token::TypeID(s) => Ok(s),
            Token::InterfaceID(s) => Ok(s),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        })
        .then(just(Token::DoubleColon).ignore_then(generic_args))
        .map(|(name, args)| TypeRef::Generic(name, args));

        let function_type = type_ref.clone()
            .repeated()
            .then_ignore(just(Token::Arrow))
            .then(type_ref.clone())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(|(args, ret)| TypeRef::Function(args, Box::new(ret)));

        choice((function_type, generic_type, simple_type))
            .map_with_span(|ast, span| (ast, span))
    })
}
