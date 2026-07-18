//! Integration tests for kata-parser.
//!
//! These tests exercise the parser through the public `parse` API,
//! lexing source strings and verifying the resulting AST structure.

mod actions;
mod basics;
mod collections;
mod csp;
mod directive_values;
mod guards;
mod helpers;
mod lambdas;
mod match_tests;
mod pipe;
mod refined_decls;
mod signatures;
