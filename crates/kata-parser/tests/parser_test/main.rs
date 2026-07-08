//! Integration tests for kata-parser.
//!
//! These tests exercise the parser through the public `parse` API,
//! lexing source strings and verifying the resulting AST structure.

mod basics;
mod helpers;
mod lambdas;
mod match_tests;
mod signatures;
