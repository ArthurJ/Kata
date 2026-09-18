//! Utilitários para testes do driver — diagnósticos de teste.

use std::fmt;

/// Diagnóstico de teste com código e mensagem controlados.
#[derive(Debug)]
pub struct TestDiagnostic {
    code: String,
    message: String,
}

impl TestDiagnostic {
    pub fn new(code: &str, message: &str) -> Self {
        TestDiagnostic {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl fmt::Display for TestDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TestDiagnostic {}

impl miette::Diagnostic for TestDiagnostic {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        Some(Box::new(self.code.clone()))
    }
}
