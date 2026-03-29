use crate::lexer::Token;
use crate::parser::ast::*;
use crate::parser::error::ParserError;
use crate::parser::core::tokens::ident_string;
use chumsky::prelude::*;

pub fn pattern_atom_parser() -> impl Parser<Token, Spanned<Pattern>, Error = ParserError> + Clone {
    recursive(|pattern| {
        let lit = choice((
            filter_map(|span, tok| match tok {
                Token::Int(s) => Ok(Expr::Int(s)),
                _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
            }),
            filter_map(|span, tok| match tok {
                Token::Float(s) => Ok(Expr::Float(s)),
                _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
            }),
            filter_map(|span, tok| match tok {
                Token::String(s) => Ok(Expr::String(s)),
                _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
            }),
        )).map(Pattern::Literal);

        let hole = just(Token::Hole).to(Pattern::Hole);
        let ident = filter_map(|span, tok| match tok {
            Token::Ident(s) => Ok(Pattern::Ident(s)),
            Token::TypeID(s) => Ok(Pattern::Ident(s)),
            Token::InterfaceID(s) => Ok(Pattern::Ident(s)),
            Token::Otherwise => Ok(Pattern::Ident("otherwise".to_string())),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let tuple = pattern.clone()
            .separated_by(just(Token::Comma).or_not())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(Pattern::Tuple);

        let list = pattern.clone()
            .separated_by(just(Token::Comma).or_not())
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map(Pattern::List);

        choice((lit, hole, ident, tuple, list))
            .map_with_span(|ast, span| (ast, span))
    })
}

pub fn pattern_sequence_parser() -> impl Parser<Token, Spanned<Pattern>, Error = ParserError> + Clone {
    pattern_atom_parser().repeated().at_least(1).map_with_span(|mut atoms, span| {
        if atoms.len() == 1 {
            atoms.remove(0)
        } else {
            (Pattern::Sequence(atoms), span)
        }
    })
}

pub fn with_block_parser() -> impl Parser<Token, Vec<Spanned<Expr>>, Error = ParserError> + Clone {
    let sequence = atom_expr_parser().repeated().at_least(1).map_with_span(|mut atoms, span| {
        if atoms.len() == 1 {
            atoms.remove(0)
        } else {
            (Expr::Sequence(atoms), span)
        }
    });

    let with_decl = ident_string()
        .then_ignore(just(Token::As))
        .then(sequence)
        .map_with_span(|(name, val), span| (Expr::WithDecl(name, Box::new(val)), span));

    just(Token::Newline).repeated()
        .ignore_then(just(Token::With))
        .ignore_then(just(Token::Newline).repeated())
        .ignore_then(just(Token::Indent))
        .ignore_then(
            with_decl.separated_by(just(Token::Newline).repeated()).allow_trailing()
        )
        .then_ignore(just(Token::Dedent))
        .or_not()
        .map(|opt| opt.unwrap_or_default())
}

fn atom_expr_parser() -> impl Parser<Token, Spanned<Expr>, Error = ParserError> + Clone {
    recursive(|expr| {
        let int = filter_map(|span, tok| match tok {
            Token::Int(s) => Ok(Expr::Int(s)),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let float = filter_map(|span, tok| match tok {
            Token::Float(s) => Ok(Expr::Float(s)),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let string = filter_map(|span, tok| match tok {
            Token::String(s) => Ok(Expr::String(s)),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let ident = filter_map(|span, tok| match tok {
            Token::Ident(s) => Ok(Expr::Ident(s)),
            Token::TypeID(s) => Ok(Expr::Ident(s)),
            Token::InterfaceID(s) => Ok(Expr::Ident(s)),
            Token::Otherwise => Ok(Expr::Ident("otherwise".to_string())),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        });

        let action_call = filter_map(|span, tok| match tok {
            Token::ActionIdent(s) => Ok(s),
            _ => Err(Simple::expected_input_found(span, Vec::new(), Some(tok))),
        })
        .then(
            expr.clone()
                .separated_by(just(Token::Comma).or_not())
                .delimited_by(just(Token::LParen), just(Token::RParen))
        )
        .map(|(name, args)| Expr::ActionCall(name, args));

        let channel_send = just(Token::ChannelSend).to(Expr::ChannelSend);
        let channel_recv = just(Token::ChannelRecv).to(Expr::ChannelRecv);
        let channel_recv_non_block = just(Token::ChannelRecvNonBlock).to(Expr::ChannelRecvNonBlock);

        let hole = just(Token::Hole).to(Expr::Hole);

        let tuple = expr.clone()
            .separated_by(just(Token::Comma).or_not())
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(Expr::Tuple);

        let list = expr.clone()
            .separated_by(just(Token::Comma).or_not())
            .delimited_by(just(Token::LBracket), just(Token::RBracket))
            .map(Expr::List);

        let array = expr.clone()
            .separated_by(just(Token::Comma).or_not())
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(Expr::Array);

        let dollar_app = just(Token::Dollar)
            .ignore_then(expr.clone())
            .map(|e| Expr::ExplicitApp(Box::new(e)));

        let atom = choice((
            int, float, string, ident, action_call, channel_send, channel_recv, channel_recv_non_block, hole, tuple, list, array, dollar_app
        )).map_with_span(|ast, span| (ast, span));

        let try_atom = atom.then(just(Token::Question).repeated())
            .map(|(mut e, questions): ((Expr, std::ops::Range<usize>), Vec<Token>)| {
                for _ in questions {
                    let span = e.1.clone();
                    e = (Expr::Try(Box::new(e)), span);
                }
                e
            });

        try_atom
    })
}

pub fn expr_parser() -> impl Parser<Token, Spanned<Expr>, Error = ParserError> + Clone {
    let sequence = atom_expr_parser().repeated().at_least(1).map_with_span(|mut atoms, span| {
        if atoms.len() == 1 {
            atoms.remove(0)
        } else {
            (Expr::Sequence(atoms), span)
        }
    });

    let lambda_body = {
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
            .map_with_span(|(mut exprs, w): (Vec<Spanned<Expr>>, Vec<Spanned<Expr>>), span| {
                let body = if exprs.len() == 1 {
                    exprs.remove(0)
                } else {
                    (Expr::Sequence(exprs), span)
                };
                (body, w)
            });

        let inline_body = sequence.clone().then(with_block_parser());

        choice((guards_block, lambda_block, inline_body))
    };

    let lambda = just(Token::Lambda)
        .ignore_then(pattern_atom_parser().repeated())
        .then_ignore(just(Token::Colon))
        .then(lambda_body)
        .map_with_span(|(args, (b, w)), span| {
            (Expr::Lambda(args, Box::new(b), w), span)
        });

    choice((lambda, sequence))
}
