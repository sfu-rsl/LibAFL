use core::ffi::c_char;

use thiserror::Error;

use crate::{GuestAddr, symbols::Symbols};

#[derive(Debug)]
pub struct NopSymbols;

impl Symbols for NopSymbols {
    type Error = NopSymbolsError;

    fn lookup(name: *const c_char) -> Result<GuestAddr, Self::Error> {
        Err(NopSymbolsError::SymbolNotFound(name))
    }
}

#[derive(Error, Debug, PartialEq, Clone)]
pub enum NopSymbolsError {
    #[error("Symbol not found: {0:p}")]
    SymbolNotFound(*const c_char),
}
