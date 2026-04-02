use crate::lexer::Token;
use crate::parser::ast::*;
use crate::parser::error::ParserError;
use crate::parser::core::tokens::ident_string;
use crate::parser::grammar::expr::{expr_parser, pattern_atom_parser, pattern_sequence_parser};
use chumsky::prelude::*;

pub fn stmt_parser() -> impl Parser<Token, Spanned<Stmt>, Error = ParserError> + Clone {
    recursive(|stmt| {
        let let_stmt = just(Token::Let)
            .ignore_then(pattern_atom_parser())
            .then(expr_parser())
            .map(|(p, e)| Stmt::Let(p, e));

        let var_stmt = just(Token::Var)
            .ignore_then(ident_string())
            .then(expr_parser())
            .map(|(name, e)| Stmt::Var(name, e));

        let block = just(Token::Newline).repeated()
            .ignore_then(just(Token::Indent))
            .ignore_then(
                stmt.clone()
                    .separated_by(just(Token::Newline).repeated().at_least(1))
                    .allow_trailing()
            )
            .then_ignore(just(Token::Dedent));

        let loop_stmt = just(Token::Loop)
            .ignore_then(block.clone())
            .map(Stmt::Loop);

        let for_stmt = just(Token::For)
            .ignore_then(ident_string())
            .then_ignore(just(Token::In))
            .then(expr_parser())
            .then(block.clone())
            .map(|((name, iter), body)| Stmt::For(name, iter, body));

        let match_arm = pattern_sequence_parser()
            .then_ignore(just(Token::Colon))
            .then(
                choice((
                    block.clone(),
                    stmt.clone().map(|s| vec![s])
                ))
            )
            .map(|(pat, stmts)| MatchArm::Pattern(pat, stmts));

        let match_block = just(Token::Newline).repeated()
            .ignore_then(just(Token::Indent))
            .ignore_then(
                match_arm.separated_by(just(Token::Newline).repeated()).allow_trailing()
            )
            .then_ignore(just(Token::Dedent));

        let match_stmt = just(Token::Match)
            .ignore_then(expr_parser())
            .then(match_block)
            .map(|(target, arms)| Stmt::Match(target, arms));

        let break_stmt = just(Token::Break).to(Stmt::Break);
        let continue_stmt = just(Token::Continue).to(Stmt::Continue);

        let select_case_arm = just(Token::Case)
            .ignore_then(expr_parser())
            .then(just(Token::Arrow).ignore_then(pattern_atom_parser()).or_not())
            .then_ignore(just(Token::Colon))
            .then(choice((block.clone(), stmt.clone().map(|s| vec![s]))))
            .map(|((operation, binding), body)| SelectArm { operation, binding, body });

        let select_timeout_arm = just(Token::Timeout)
            .ignore_then(expr_parser())
            .then_ignore(just(Token::Colon))
            .then(choice((block.clone(), stmt.clone().map(|s| vec![s]))));

        let select_block = just(Token::Newline).repeated()
            .ignore_then(just(Token::Indent))
            .ignore_then(
                select_case_arm.separated_by(just(Token::Newline).repeated()).allow_trailing()
                    .then(select_timeout_arm.or_not().then_ignore(just(Token::Newline).repeated()))
            )
            .then_ignore(just(Token::Dedent));

        let select_stmt = just(Token::Select)
            .ignore_then(select_block)
            .map(|(cases, timeout)| Stmt::Select(cases, timeout));

        let expr_stmt = expr_parser().map(Stmt::Expr);

        choice((
            let_stmt,
            var_stmt,
            loop_stmt,
            for_stmt,
            match_stmt,
            select_stmt,
            break_stmt,
            continue_stmt,
            expr_stmt
        )).map_with_span(|ast, span| (ast, span))
    })
}
