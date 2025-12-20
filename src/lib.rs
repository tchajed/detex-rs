//! detex - Strip TeX/LaTeX commands from input files
//!
//! This is a Rust port of the opendetex program originally written
//! by Daniel Trinkle at Purdue University.
//!
//! # Example
//!
//! ```
//! use detex_rs::{Detex, Options};
//!
//! let opts = Options::default();
//! let mut output = Vec::new();
//! let mut detex = Detex::new(opts, &mut output);
//! // Process files or stdin...
//! ```

mod config;
mod file_handler;
mod lexer;

pub use config::{Options, VERSION};
pub use lexer::Detex;
