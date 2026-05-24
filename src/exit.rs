use std::{error::Error, fmt};

use anyhow::Error as AnyhowError;

#[derive(Debug)]
pub struct ExitError {
    code: u8,
    source: AnyhowError,
}

impl ExitError {
    pub fn pull(source: AnyhowError) -> Self {
        Self { code: 2, source }
    }

    pub fn push(source: AnyhowError) -> Self {
        Self { code: 3, source }
    }

    pub fn check_drift(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            source: anyhow::anyhow!(message.into()),
        }
    }

    pub fn parity_lint(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            source: anyhow::anyhow!(message.into()),
        }
    }

    pub fn code(&self) -> u8 {
        self.code
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl Error for ExitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.source()
    }
}
