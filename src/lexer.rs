//! Lexer state machine for stripping TeX/LaTeX commands.
//!
//! This is a port of opendetex's detex.l flex lexer to Rust.
//! Comments throughout reference line numbers in opendetex-2.8.11/detex.l.
//!
//! # Structure
//!
//! The original detex.l is a flex (lex) scanner with:
//! - Pattern definitions (lines 192-204)
//! - State declarations (lines 206-209)
//! - Pattern-action rules (lines 211-514)
//! - Support functions (lines 517-1132)
//!
//! This Rust port uses a state machine approach where:
//! - Each flex state becomes a State enum variant
//! - Each state has a process_* method that handles its patterns
//! - Pattern matching is done manually using character-by-character processing
//!
//! # Correspondence to detex.l
//!
//! - detex.l:96-111: Macro definitions → Helper methods like noun(), verb_noun(), etc.
//! - detex.l:192-204: Pattern definitions → Handled inline in the processing methods
//! - detex.l:206-209: State declarations → State enum (lines 23-42)
//! - detex.l:212-485: Normal state rules → process_normal() and process_backslash()
//! - detex.l:487-514: Other state rules → process_la_macro(), process_math(), etc.

#![allow(clippy::single_match)]

use std::io::{Read, Write};

use crate::config::{MAX_FILE_STACK, Options};
use crate::file_handler::{CharSource, in_include_list, tex_open};

/// Lexer states matching the original flex states.
/// See detex.l lines 206-209:
/// ```text
/// %Start Define Display IncludeOnly Input Math Normal Control
/// %Start LaBegin LaDisplay LaEnd LaEnv LaFormula LaInclude LaSubfile
/// %Start LaMacro LaOptArg LaMacro2 LaOptArg2 LaVerbatim
/// %start LaBreak LaPicture
/// ```
/// Note: LaBegin is handled inline in process_backslash for \begin.
/// LaSubfile uses the same logic as LaInclude.
/// LaBreak is unused (commented out in detex.l).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Define,      // detex.l:373-376 - inside \def, waiting for '{'
    Display,     // detex.l:390-394 - inside $$...$$ display math
    IncludeOnly, // detex.l:410-417 - parsing \includeonly{...}
    Input,       // detex.l:426-431 - parsing \input filename
    Math,        // detex.l:396-401 - inside $...$ inline math
    Control,     // detex.l:451-456 - after unknown \command
    LaDisplay,   // detex.l:384-388 - inside \[...\] display math
    LaEnd,       // detex.l:266-272 - inside \end{...} parsing env name
    LaEnv,       // detex.l:262-264 - absorbing ignored environment content
    LaFormula,   // detex.l:378-382 - inside \(...\) inline math
    LaInclude,   // detex.l:403-408 - parsing \include{...} filename
    LaMacro,     // detex.l:487-498 - consuming N brace-delimited arguments (KILLARGS)
    LaOptArg,    // detex.l:497-498 - inside optional [...] arg for LaMacro
    LaMacro2,    // detex.l:500-514 - consuming args but keeping last one (STRIPARGS)
    LaOptArg2,   // detex.l:513-514 - inside optional [...] arg for LaMacro2
    LaVerbatim,  // detex.l:225-227 - inside verbatim environment, echoing content
    LaPicture,   // detex.l:300-303 - parsing \includegraphics{...}
}

/// File context for stack
struct FileContext {
    source: CharSource,
    name: String,
}

/// The main detex processor
pub struct Detex<W: Write> {
    opts: Options,
    state: State,
    output: W,
    file_stack: Vec<FileContext>,
    current_ignored_env: String,
    open_braces: usize,
    args_count: usize,
    current_braces_level: usize,
    footnote_level: i32,
    at_column_zero: bool,
}

impl<W: Write> Detex<W> {
    pub fn new(opts: Options, output: W) -> Self {
        Self {
            opts,
            state: State::Normal,
            output,
            file_stack: Vec::with_capacity(MAX_FILE_STACK),
            current_ignored_env: String::new(),
            open_braces: 0,
            args_count: 0,
            current_braces_level: 0,
            footnote_level: -100,
            at_column_zero: true,
        }
    }

    /// Process a file
    pub fn process_file(&mut self, filename: &str) -> Result<(), String> {
        let (mut file, _path) = tex_open(filename, &self.opts)
            .ok_or_else(|| format!("can't open file {}", filename))?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| format!("error reading {}: {}", filename, e))?;

        let source = CharSource::new(content);
        let name = filename.to_string();

        self.file_stack.push(FileContext { source, name });
        self.state = State::Normal;
        self.process()
    }

    /// Process stdin
    pub fn process_stdin(&mut self) -> Result<(), String> {
        let mut content = String::new();
        std::io::stdin()
            .read_to_string(&mut content)
            .map_err(|e| format!("error reading stdin: {}", e))?;

        let source = CharSource::new(content);
        self.file_stack.push(FileContext {
            source,
            name: "<stdin>".to_string(),
        });
        self.state = State::Normal;
        self.process()
    }

    fn current_source(&self) -> Option<&CharSource> {
        self.file_stack.last().map(|ctx| &ctx.source)
    }

    fn current_source_mut(&mut self) -> Option<&mut CharSource> {
        self.file_stack.last_mut().map(|ctx| &mut ctx.source)
    }

    fn current_filename(&self) -> &str {
        self.file_stack
            .last()
            .map(|ctx| ctx.name.as_str())
            .unwrap_or("<unknown>")
    }

    fn current_line(&self) -> usize {
        self.current_source().map(|s| s.line).unwrap_or(1)
    }

    /// Main processing loop
    fn process(&mut self) -> Result<(), String> {
        while !self.file_stack.is_empty() {
            if self.current_source().map(|s| s.is_eof()).unwrap_or(true) {
                self.file_stack.pop();
                continue;
            }
            self.process_next()?;
        }
        Ok(())
    }

    /// Process next token based on current state
    fn process_next(&mut self) -> Result<(), String> {
        match self.state {
            State::Normal => self.process_normal(),
            State::Define => self.process_define(),
            State::Display => self.process_display(),
            State::IncludeOnly => self.process_include_only(),
            State::Input => self.process_input(),
            State::Math => self.process_math(),
            State::Control => self.process_control(),
            State::LaDisplay => self.process_la_display(),
            State::LaEnd => self.process_la_end(),
            State::LaEnv => self.process_la_env(),
            State::LaFormula => self.process_la_formula(),
            State::LaInclude => self.process_la_include(),
            State::LaMacro => self.process_la_macro(),
            State::LaOptArg => self.process_la_opt_arg(),
            State::LaMacro2 => self.process_la_macro2(),
            State::LaOptArg2 => self.process_la_opt_arg2(),
            State::LaVerbatim => self.process_la_verbatim(),
            State::LaPicture => self.process_la_picture(),
        }
    }

    // ========== Output helpers ==========
    // These correspond to macros defined in detex.l lines 100-111:
    //   #define IGNORE      Ignore()
    //   #define INCRLINENO  IncrLineNo()
    //   #define ECHO        Echo()
    //   #define NOUN        if (fSpace && !fWord && !fReplace) putchar(' '); else {if (fReplace) printf("noun");}
    //   #define VERBNOUN    if (fReplace) printf(" verbs noun");
    //   #define SPACE       if (!fWord) putchar(' ')
    //   #define NEWLINE     LineBreak()
    //   #define LATEX       fLatex=!fForcetex
    //   #define KILLARGS(x) cArgs=x; LaBEGIN LaMacro
    //   #define STRIPARGS(x) cArgs=x; LaBEGIN LaMacro2
    //   #define CITE(x)     if (fLatex && !fCite) KILLARGS(x)
    //   #define LaBEGIN     if (fLatex) BEGIN

    /// detex.l:702-709 PrintPrefix() - outputs source location if -1 flag
    fn print_prefix(&mut self) {
        if self.opts.src_loc && self.at_column_zero {
            let filename = self.current_filename().to_string();
            let line = self.current_line();
            let _ = write!(self.output, "{}:{}: ", filename, line);
            self.at_column_zero = false;
        }
    }

    /// detex.l:730-735 Echo() - outputs text with optional prefix
    /// Corresponds to: #define ECHO Echo()
    fn echo(&mut self, c: char) {
        self.print_prefix();
        let _ = write!(self.output, "{}", c);
    }

    fn echo_str(&mut self, s: &str) {
        self.print_prefix();
        let _ = write!(self.output, "{}", s);
    }

    /// detex.l:716-723 LineBreak() - outputs newline unless -w flag
    /// Corresponds to: #define NEWLINE LineBreak()
    fn newline(&mut self) {
        if self.opts.word {
            return;
        }
        self.print_prefix();
        let _ = writeln!(self.output);
        // detex.l:722 - fFileLines[csb]++; fIsColumn0=1;
        if let Some(source) = self.current_source_mut() {
            source.incr_line();
        }
        self.at_column_zero = true;
    }

    /// detex.l:106 - outputs space unless -w flag
    /// Corresponds to: #define SPACE if (!fWord) putchar(' ')
    fn space(&mut self) {
        if !self.opts.word {
            let _ = write!(self.output, " ");
        }
    }

    /// detex.l:104 - outputs space (if -s) or "noun" (if -r) for math
    /// Corresponds to: #define NOUN if (fSpace && !fWord && !fReplace) putchar(' '); else {if (fReplace) printf("noun");}
    fn noun(&mut self) {
        if self.opts.space && !self.opts.word && !self.opts.replace {
            let _ = write!(self.output, " ");
        } else if self.opts.replace {
            let _ = write!(self.output, "noun");
        }
    }

    /// detex.l:105 - outputs " verbs noun" for verb symbols in math (if -r)
    /// Corresponds to: #define VERBNOUN if (fReplace) printf(" verbs noun");
    fn verb_noun(&mut self) {
        if self.opts.replace {
            let _ = write!(self.output, " verbs noun");
        }
    }

    /// detex.l:757-763 Ignore() - outputs space if -s flag, otherwise nothing
    /// Corresponds to: #define IGNORE Ignore()
    fn ignore(&mut self) {
        if self.opts.space && !self.opts.word {
            let _ = write!(self.output, " ");
        }
    }

    // ========== Helper methods ==========

    /// detex.l:100 - conditionally change state if in latex mode
    /// Corresponds to: #define LaBEGIN if (fLatex) BEGIN
    fn la_begin(&mut self, new_state: State) {
        if self.opts.is_latex() {
            self.state = new_state;
        }
    }

    /// detex.l:108 - set latex mode unless -t flag
    /// Corresponds to: #define LATEX fLatex=!fForcetex
    fn set_latex(&mut self) {
        if !self.opts.force_tex {
            self.opts.latex = true;
        }
    }

    /// detex.l:109 - consume N brace-delimited arguments
    /// Corresponds to: #define KILLARGS(x) cArgs=x; LaBEGIN LaMacro
    fn kill_args(&mut self, n: usize) {
        self.args_count = n;
        self.open_braces = 0;
        self.la_begin(State::LaMacro);
    }

    /// detex.l:110 - consume N-1 arguments, keep the Nth
    /// Corresponds to: #define STRIPARGS(x) cArgs=x; LaBEGIN LaMacro2
    fn strip_args(&mut self, n: usize) {
        self.args_count = n;
        self.open_braces = 0;
        self.la_begin(State::LaMacro2);
    }

    /// detex.l:788-800 BeginEnv() - check if env should be ignored
    /// Returns true if the environment is in the ignore list.
    fn begin_env(&mut self, env: &str) -> bool {
        if !self.opts.is_latex() {
            return false;
        }
        if self.opts.env_ignore.iter().any(|e| e == env) {
            self.current_ignored_env = env.to_string();
            true
        } else {
            false
        }
    }

    /// detex.l:806-813 EndEnv() - check if env matches current ignored env
    fn end_env(&mut self, env: &str) -> bool {
        if !self.opts.is_latex() {
            return false;
        }
        env == self.current_ignored_env
    }

    /// detex.l:847-868 IncludeFile() - include file if in includeonly list
    fn include_file(&mut self, filename: &str) -> Result<(), String> {
        if self.opts.no_follow {
            return Ok(());
        }
        if !in_include_list(filename, &self.opts) {
            return Ok(());
        }
        self.input_file(filename)
    }

    /// detex.l:821-840 InputFile() - push current file and open new one
    fn input_file(&mut self, filename: &str) -> Result<(), String> {
        if self.opts.no_follow {
            return Ok(());
        }

        if self.file_stack.len() >= MAX_FILE_STACK {
            eprintln!("detex: warning: file stack overflow, ignoring {}", filename);
            return Ok(());
        }

        match tex_open(filename, &self.opts) {
            Some((mut file, _path)) => {
                let mut content = String::new();
                if let Err(e) = file.read_to_string(&mut content) {
                    eprintln!("detex: warning: can't read file {}: {}", filename, e);
                    return Ok(());
                }

                let source = CharSource::new(content);
                let name = filename.to_string();
                self.file_stack.push(FileContext { source, name });
                Ok(())
            }
            None => {
                eprintln!("detex: warning: can't open file {}", filename);
                Ok(())
            }
        }
    }

    /// detex.l:875-884 AddInclude() - add file to includeonly list
    fn add_include(&mut self, filename: &str) {
        if self.opts.no_follow {
            return;
        }
        let base = if let Some(pos) = filename.rfind('.') {
            &filename[..pos]
        } else {
            filename
        };
        self.opts.include_list.push(base.to_string());
    }

    fn next_char(&mut self) -> Option<char> {
        self.current_source_mut().and_then(|s| s.next())
    }

    fn peek_char(&self) -> Option<char> {
        self.current_source().and_then(|s| s.peek())
    }

    fn unget_char(&mut self, c: char) {
        if let Some(s) = self.current_source_mut() {
            s.unget(c);
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                let ch = self.next_char().unwrap();
                // Track newlines in skipped whitespace
                if ch == '\n' {
                    if let Some(source) = self.current_source_mut() {
                        source.incr_line();
                    }
                    self.at_column_zero = true;
                }
            } else {
                break;
            }
        }
    }

    fn read_command_name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_ascii_alphabetic() || c == '@' {
                name.push(self.next_char().unwrap());
            } else {
                break;
            }
        }
        name
    }

    fn try_match(&mut self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        let mut matched = Vec::new();

        for expected in &chars {
            match self.next_char() {
                Some(c) if c == *expected => matched.push(c),
                Some(c) => {
                    self.unget_char(c);
                    for c in matched.into_iter().rev() {
                        self.unget_char(c);
                    }
                    return false;
                }
                None => {
                    for c in matched.into_iter().rev() {
                        self.unget_char(c);
                    }
                    return false;
                }
            }
        }
        true
    }

    fn match_optional_star(&mut self) {
        if self.peek_char() == Some('*') {
            self.next_char();
        }
    }

    fn skip_glue(&mut self) {
        self.skip_whitespace();
        if let Some(c) = self.peek_char()
            && (c == '+' || c == '-')
        {
            self.next_char();
        }
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '.' {
                self.next_char();
            } else {
                break;
            }
        }
        let _ = self.read_command_name();
        self.skip_whitespace();
        while self.try_match("plus") || self.try_match("minus") {
            self.skip_whitespace();
            if let Some(c) = self.peek_char()
                && (c == '+' || c == '-')
            {
                self.next_char();
            }
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() || c == '.' {
                    self.next_char();
                } else {
                    break;
                }
            }
            let _ = self.read_command_name();
            self.skip_whitespace();
        }
    }

    fn skip_brace_arg(&mut self) {
        self.skip_whitespace();
        if self.try_match("{") {
            let mut depth = 1;
            while depth > 0 {
                match self.next_char() {
                    Some('{') => depth += 1,
                    Some('}') => depth -= 1,
                    Some('\\') => {
                        // Skip the next character after backslash
                        if let Some(ch) = self.next_char()
                            && ch == '\n' {
                                if let Some(source) = self.current_source_mut() {
                                    source.incr_line();
                                }
                                self.at_column_zero = true;
                            }
                    }
                    Some('\n') => {
                        // Track newlines in skipped content
                        if let Some(source) = self.current_source_mut() {
                            source.incr_line();
                        }
                        self.at_column_zero = true;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    fn skip_optional_bracket_arg(&mut self) {
        if self.peek_char() == Some('[') {
            self.next_char();
            let mut depth = 1;
            while depth > 0 {
                match self.next_char() {
                    Some('[') => depth += 1,
                    Some(']') => depth -= 1,
                    Some('\\') => {
                        // Skip the next character after backslash
                        if let Some(ch) = self.next_char()
                            && ch == '\n' {
                                if let Some(source) = self.current_source_mut() {
                                    source.incr_line();
                                }
                                self.at_column_zero = true;
                            }
                    }
                    Some('\n') => {
                        // Track newlines in skipped content
                        if let Some(source) = self.current_source_mut() {
                            source.incr_line();
                        }
                        self.at_column_zero = true;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    // ========== State processors ==========

    /// Process Normal state - the main text processing state.
    /// This handles most LaTeX constructs in the document body.
    /// See detex.l lines 212-485 for the Normal state rules.
    fn process_normal(&mut self) -> Result<(), String> {
        let c = match self.next_char() {
            Some(c) => c,
            None => return Ok(()),
        };

        match c {
            // detex.l:212 - <Normal>"%".*  - ignore comments
            // The pattern includes "\n" which is consumed but not output
            // detex.l:212 calls INCRLINENO which tracks newlines in matched text
            '%' => {
                while let Some(c) = self.next_char() {
                    if c == '\n' {
                        // Increment line for the consumed newline
                        if let Some(source) = self.current_source_mut() {
                            source.incr_line();
                        }
                        self.at_column_zero = true;
                        break;
                    }
                }
                self.ignore();
            }

            // Backslash commands handled in process_backslash
            '\\' => self.process_backslash()?,

            // detex.l:390 - <Normal>"$$" - display math mode
            // detex.l:396 - <Normal>"$" - inline math mode
            '$' => {
                if self.peek_char() == Some('$') {
                    self.next_char();
                    self.state = State::Display;
                    self.noun();
                } else {
                    self.state = State::Math;
                    self.noun();
                }
            }

            // detex.l:467-469 - <Normal>"{" - increment brace level
            // Also check for detex.l:280 - <Normal>"{"{N}"pt}" - dimension hack for minipage
            '{' => {
                // detex.l:280 - hack to fix \begin{minipage}{300pt}
                // Try to match {NUMBER pt} pattern and ignore it
                if let Some(next) = self.peek_char()
                    && (next.is_ascii_digit() || next == '+' || next == '-' || next == '.')
                {
                    let mut consumed = Vec::new();
                    let mut has_digit = false;

                    // Try to consume number
                    while let Some(ch) = self.peek_char() {
                        if ch.is_ascii_digit() || ch == '.' || ch == '+' || ch == '-' {
                            has_digit = true;
                            consumed.push(self.next_char().unwrap());
                        } else {
                            break;
                        }
                    }

                    // Check for 'pt' followed by '}'
                    let pt_match = self.try_match("pt");
                    let close_brace = self.peek_char() == Some('}');

                    if has_digit && pt_match && close_brace {
                        self.next_char(); // consume '}'
                        // Pattern matched, ignore it completely (detex.l:280)
                        return Ok(());
                    } else {
                        // Not a match, put everything back
                        if pt_match {
                            self.unget_char('t');
                            self.unget_char('p');
                        }
                        for ch in consumed.into_iter().rev() {
                            self.unget_char(ch);
                        }
                    }
                }
                self.current_braces_level += 1;
            }

            // detex.l:470-476 - <Normal>"}" - decrement brace level, check footnote
            '}' => {
                self.current_braces_level = self.current_braces_level.saturating_sub(1);
                if self.current_braces_level as i32 == self.footnote_level {
                    let _ = write!(self.output, ")");
                    self.footnote_level = -100;
                }
            }

            // detex.l:460 - <Normal>~ - non-breaking space -> space
            '~' => self.space(),

            // detex.l:458 - <Normal>[\\|] - ignore pipe, output space if -s
            '|' => self.ignore(),

            // detex.l:459 - <Normal>[!?]"`" - Spanish punctuation
            '!' | '?' => {
                if self.peek_char() == Some('`') {
                    self.next_char();
                } else if !self.opts.word {
                    self.echo(c);
                }
            }

            // detex.l:461 - <Normal>-{2,3} - em/en dash -> single dash
            '-' => {
                let mut dashes = 1;
                while self.peek_char() == Some('-') && dashes < 3 {
                    self.next_char();
                    dashes += 1;
                }
                if !self.opts.word {
                    let _ = write!(self.output, "-");
                }
            }

            // detex.l:462-463 - <Normal>`` -> " and <Normal>` -> '
            '`' => {
                if self.peek_char() == Some('`') {
                    self.next_char();
                    if !self.opts.word {
                        let _ = write!(self.output, "\"");
                    }
                } else if !self.opts.word {
                    let _ = write!(self.output, "'");
                }
            }

            // detex.l:464 - <Normal>'' -> "
            '\'' => {
                if self.peek_char() == Some('\'') {
                    self.next_char();
                    if !self.opts.word {
                        let _ = write!(self.output, "\"");
                    }
                } else if !self.opts.word {
                    self.echo(c);
                }
            }

            // detex.l:465 - <Normal>,, -> " (German quotes)
            ',' => {
                if self.peek_char() == Some(',') {
                    self.next_char();
                    if !self.opts.word {
                        let _ = write!(self.output, "\"");
                    }
                } else if !self.opts.word {
                    self.echo(c);
                }
            }

            // detex.l:484 - <Normal>"\n" - newline
            '\n' => {
                if !self.opts.word {
                    self.newline();
                }
            }

            // detex.l:485 - <Normal>("\t")+ - tabs
            '\t' => {
                if !self.opts.word {
                    let _ = write!(self.output, "\t");
                }
            }

            // detex.l:477-481 - <Normal>{W}[']*{W} - words with apostrophes
            _ if c.is_ascii_alphabetic() => {
                let mut word = String::new();
                word.push(c);

                while let Some(ch) = self.peek_char() {
                    if ch.is_ascii_alphabetic() {
                        word.push(self.next_char().unwrap());
                    } else if ch == '\'' {
                        self.next_char();
                        if let Some(next) = self.peek_char() {
                            if next.is_ascii_alphabetic() {
                                word.push('\'');
                            } else {
                                self.unget_char('\'');
                                break;
                            }
                        } else {
                            self.unget_char('\'');
                            break;
                        }
                    } else {
                        break;
                    }
                }

                if self.opts.word {
                    let _ = writeln!(self.output, "{}", word);
                } else {
                    self.echo_str(&word);
                }
            }

            // detex.l:482 - <Normal>[0-9]+ - numbers
            _ if c.is_ascii_digit() => {
                if !self.opts.word {
                    let mut num = String::new();
                    num.push(c);
                    while let Some(ch) = self.peek_char() {
                        if ch.is_ascii_digit() {
                            num.push(self.next_char().unwrap());
                        } else {
                            break;
                        }
                    }
                    self.echo_str(&num);
                }
            }

            // detex.l:483 - <Normal>. - any other character
            _ => {
                if !self.opts.word {
                    // detex.l:327 - <Normal>" "?"\\cite" - kill space before \cite
                    // Check for space before \cite and don't output it
                    if c == ' ' && self.peek_char() == Some('\\') {
                        // Peek ahead to see if this is \cite
                        if let Some(src) = self.current_source() {
                            let lookahead = src.peek_ahead(6); // Look at next 6 chars: \cite + potential next char
                            if lookahead.len() >= 5 && lookahead[0..5] == *"\\cite" {
                                // Check that "cite" is not followed by more letters (i.e., it's not \citation)
                                if lookahead.len() == 5
                                    || !lookahead.chars().nth(5).unwrap().is_ascii_alphabetic()
                                {
                                    // This is " \cite" - don't output the space
                                    return Ok(());
                                }
                            }
                        }
                    }
                    self.echo(c);
                }
            }
        }

        Ok(())
    }

    /// Process backslash commands in Normal state.
    /// See detex.l lines 214-456 for backslash-initiated patterns.
    ///
    /// This handles all patterns starting with '\' in Normal state:
    /// - detex.l:214: \begin{document}
    /// - detex.l:216: \begin{...} (general)
    /// - detex.l:274-280: Spacing commands (\kern, \vskip, \hskip, \vspace, \hspace, \addvspace)
    /// - detex.l:282-315: Box and layout commands (\newlength, \setlength, \raisebox, etc.)
    /// - detex.l:316-322: Sectioning commands (\part, \chapter, \section, etc.)
    /// - detex.l:324-345: Bibliography and reference commands (\bibitem, \cite, \ref, etc.)
    /// - detex.l:347-367: \footnote and \verb
    /// - detex.l:369-371: Command definitions (\newcommand, \renewcommand, \newenvironment)
    /// - detex.l:373: \def
    /// - detex.l:378: \( inline math
    /// - detex.l:384: \[ display math
    /// - detex.l:403-431: File inclusion (\include, \includeonly, \subfile, \input)
    /// - detex.l:434-439: Special characters and ligatures (\slash, \aa, \O, \linebreak, etc.)
    /// - detex.l:441-444: Generic escape sequences (\\, \ , \%, \., etc.)
    fn process_backslash(&mut self) -> Result<(), String> {
        let cmd = self.read_command_name();

        if cmd.is_empty() {
            // Non-alphabetic command: \(, \[, \\, \ , \%, \$, etc.
            match self.next_char() {
                // detex.l:378 - <Normal>"\\(" - inline formula mode
                Some('(') => {
                    self.la_begin(State::LaFormula);
                    self.noun();
                }
                // detex.l:384 - <Normal>"\\[" - display formula mode
                Some('[') => {
                    self.la_begin(State::LaDisplay);
                    self.noun();
                }
                // detex.l:443 - <Normal>"\\\\"{Z}(\[[^\]]*\])? - line break
                Some('\\') => {
                    self.match_optional_star();
                    self.skip_optional_bracket_arg();
                    self.newline();
                }
                // detex.l:442 - <Normal>"\\ " - explicit space
                Some(' ') => self.space(),
                // detex.l:435 - <Normal>"\\%" - literal percent
                Some('%') => {
                    if !self.opts.word {
                        let _ = write!(self.output, "%");
                    }
                }
                // Escaped dollar sign (not explicit in detex.l but handled similarly)
                Some('$') => {
                    if !self.opts.word {
                        let _ = write!(self.output, "$");
                    }
                }
                // detex.l:444 - <Normal>"\\." - other escaped chars -> IGNORE
                Some(_) | None => {
                    self.ignore();
                }
            }
            return Ok(());
        }

        match cmd.as_str() {
            // detex.l:214-258 - \begin{...} handling
            // Line 216: <Normal>"\\begin" {LaBEGIN LaBegin; IGNORE;}
            // Note: LaBEGIN is "if (fLatex) BEGIN", so in TeX mode we don't
            // enter LaBegin state - we stay in Normal and just call IGNORE.
            "begin" => {
                self.ignore(); // detex.l:216 IGNORE

                // In TeX mode (not LaTeX), we don't process the {env} part.
                // The {env} will be processed as normal text in Normal state.
                if !self.opts.is_latex() {
                    return Ok(());
                }

                self.skip_whitespace();
                if self.try_match("{") {
                    self.skip_whitespace();
                    let env = self.read_command_name();
                    self.skip_whitespace();
                    self.try_match("}");

                    // detex.l:214 - \begin{document}
                    if env == "document" {
                        self.set_latex();
                        // detex.l:214 - pattern is: "\\begin"{S}"{"{S}"document"{S}"}""\n"*
                        // The "\n"* part consumes optional newlines
                        while self.peek_char() == Some('\n') {
                            self.next_char();
                            // Track the consumed newline
                            if let Some(source) = self.current_source_mut() {
                                source.incr_line();
                            }
                            self.at_column_zero = true;
                        }
                        // detex.l:214 has IGNORE but document is special (LATEX; IGNORE)
                        // detex.l:218-223 - \begin{verbatim}
                    } else if env == "verbatim" {
                        if self.begin_env("verbatim") {
                            self.state = State::LaEnv;
                        } else {
                            self.state = State::LaVerbatim;
                        }
                        self.ignore(); // detex.l:222 IGNORE
                    // detex.l:229-235 - \begin{minipage}
                    } else if env == "minipage" {
                        self.kill_args(1); // detex.l:229 KILLARGS(1)
                        if self.begin_env("minipage") {
                            self.state = State::LaEnv;
                        }
                        // State is either LaEnv or LaMacro (from kill_args)
                        self.ignore(); // detex.l:234 IGNORE
                    // detex.l:237-251 - \begin{table}[pos] or \begin{figure}[pos]
                    } else if env == "table" || env == "figure" {
                        self.skip_whitespace();
                        self.skip_optional_bracket_arg();
                        if self.begin_env(&env) {
                            self.state = State::LaEnv;
                        }
                        self.ignore(); // detex.l:242,250 IGNORE
                    // detex.l:253-258 - \begin{other_env}
                    } else {
                        if self.begin_env(&env) {
                            self.state = State::LaEnv;
                        }
                        self.ignore(); // detex.l:257 IGNORE (outside the if/else)
                    }
                }
            }

            // detex.l:331 - <Normal>"\\end" {KILLARGS(1); IGNORE;}
            "end" => {
                self.kill_args(1);
                self.ignore();
            }

            // detex.l:274-279 - spacing commands (no IGNORE)
            // <Normal>"\\kern"{HD}            ;
            // <Normal>"\\vskip"{VG}           ;
            // <Normal>"\\hskip"{HG}           ;
            "kern" | "vskip" | "hskip" => {
                self.skip_glue();
            }
            // <Normal>"\\vspace"{Z}{S}"{"{VG}"}"  ;
            // <Normal>"\\hspace"{Z}{S}"{"{HG}"}"  ;
            "vspace" | "hspace" => {
                self.match_optional_star();
                self.skip_brace_arg();
            }
            // <Normal>"\\addvspace"{S}"{"{VG}"}" ;
            "addvspace" => {
                self.skip_brace_arg();
            }

            // detex.l:282-298 - KILLARGS commands WITHOUT IGNORE
            // Note: These do NOT call IGNORE in detex.l
            "newlength" | "newsavebox" | "usebox" | "parbox" | "rotatebox" | "sbox" => {
                self.kill_args(1);
                // No IGNORE - detex.l:282,288,291,293,297,289
            }

            // detex.l:283-287,290 - KILLARGS(2) commands WITHOUT IGNORE
            "setlength" | "addtolength" | "settowidth" | "settoheight" | "settodepth"
            | "savebox" => {
                self.kill_args(2);
                // No IGNORE - detex.l:283-287,290
            }

            // detex.l:292,294,311 - STRIPARGS commands
            "raisebox" | "scalebox" | "foilhead" => {
                self.strip_args(2);
                // No IGNORE
            }

            // detex.l:295 - <Normal>"\\resizebox"{Z} {KILLARGS(2);}
            "resizebox" => {
                self.match_optional_star();
                self.kill_args(2);
            }

            // detex.l:296 - <Normal>"\\reflectbox" ;
            "reflectbox" => {
                // Do nothing - just skip the command
            }

            // detex.l:305-310 - color commands (no IGNORE)
            "definecolor" | "fcolorbox" | "addcontentsline" => {
                self.kill_args(3);
            }
            "textcolor" | "colorbox" => {
                self.kill_args(2);
            }
            "color" | "pagecolor" => {
                self.kill_args(1);
                // No IGNORE - detex.l:306,310
            }

            // detex.l:312-314 - more commands without IGNORE
            "addfontfeature" | "thispagestyle" => {
                self.kill_args(1);
                // No IGNORE - detex.l:312,313
            }

            // detex.l:298 - <Normal>"\\includegraphics"[^{]* {LaBEGIN LaPicture;}
            "includegraphics" => {
                // Skip any [...] options before the {filename}
                while self.peek_char() == Some('[') {
                    self.skip_optional_bracket_arg();
                }
                self.la_begin(State::LaPicture);
            }

            // detex.l:316-322 - sectioning commands (just skip optional *)
            "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
            | "subparagraph" => {
                self.match_optional_star();
                // No IGNORE or KILLARGS - these are printed as-is
            }

            // detex.l:324-326 - bibliography commands WITH IGNORE
            "bibitem" | "bibliography" | "bibstyle" => {
                self.kill_args(1);
                self.ignore();
            }

            // detex.l:327 - <Normal>" "?"\\cite" {KILLARGS(1);}
            // Note: NO IGNORE! The space before is handled in process_normal
            "cite" => {
                self.kill_args(1);
            }

            // detex.l:332-333 - hypersetup, index (no IGNORE)
            "hypersetup" | "index" => {
                self.kill_args(1);
                // No IGNORE - detex.l:332,333
            }

            // detex.l:335 - <Normal>"\\label" {KILLARGS(1); IGNORE;}
            "label" => {
                self.kill_args(1);
                self.ignore();
            }

            // detex.l:336-337,339 - CITE macro (conditional KILLARGS) + IGNORE
            // #define CITE(x) if (fLatex && !fCite) KILLARGS(x)
            "nameref" | "pageref" | "ref" => {
                if self.opts.is_latex() && !self.opts.cite {
                    self.kill_args(1);
                }
                self.ignore();
            }

            // detex.l:338 - <Normal>"\\pagestyle" {KILLARGS(1); IGNORE;}
            "pagestyle" => {
                self.kill_args(1);
                self.ignore();
            }

            // detex.l:340-341 - setcounter, addtocounter WITH IGNORE
            "setcounter" | "addtocounter" => {
                self.kill_args(2);
                self.ignore();
            }

            // detex.l:342-343 - newcounter, stepcounter (no IGNORE)
            "newcounter" => {
                self.kill_args(1);
            }
            "stepcounter" => {
                self.kill_args(2); // Note: detex.l has KILLARGS(2) though stepcounter takes 1 arg
            }

            // detex.l:345 - <Normal>"\\fontspec" {KILLARGS(1);}
            "fontspec" => {
                self.kill_args(1);
                // No IGNORE
            }

            // detex.l:328-330 - document class/style and usepackage
            "documentstyle" | "documentclass" => {
                self.set_latex();
                self.kill_args(1);
                self.ignore();
            }
            "usepackage" => {
                self.kill_args(1);
                self.ignore();
            }

            // detex.l:403-408 - <Normal>"\\include" {LaBEGIN LaInclude; IGNORE;}
            "include" => {
                self.la_begin(State::LaInclude);
                self.ignore();
            }

            // detex.l:410 - <Normal>"\\includeonly" {BEGIN IncludeOnly; IGNORE;}
            "includeonly" => {
                self.state = State::IncludeOnly;
                self.ignore();
            }

            // detex.l:419-424 - <Normal>"\\subfile" {LaBEGIN LaSubfile; IGNORE;}
            // LaSubfile has same rules as LaInclude
            "subfile" => {
                self.la_begin(State::LaInclude);
                self.ignore();
            }

            // detex.l:426 - <Normal>"\\input" {BEGIN Input; IGNORE;}
            "input" => {
                self.state = State::Input;
                self.ignore();
            }

            // detex.l:347-351 - <Normal>"\\footnote"(\[([^\]])+\])?"{"
            "footnote" => {
                self.skip_optional_bracket_arg();
                if self.try_match("{") {
                    let _ = write!(self.output, "(");
                    self.footnote_level = self.current_braces_level as i32;
                    self.current_braces_level += 1;
                }
            }

            // detex.l:352-367 - <Normal>"\\verb"
            "verb" => {
                if self.opts.is_latex()
                    && let Some(delim) = self.next_char()
                {
                    if delim < ' ' {
                        return Err("\\verb not complete before eof".to_string());
                    }
                    while let Some(c) = self.next_char() {
                        if c == delim {
                            break;
                        }
                        if c == '\n' || c == '\0' {
                            return Err("\\verb not complete before eof".to_string());
                        }
                        let _ = write!(self.output, "{}", c);
                    }
                }
            }

            // detex.l:369-371 - newcommand, renewcommand, newenvironment
            "newcommand" | "renewcommand" => {
                self.set_latex();
                self.kill_args(2);
            }
            "newenvironment" => {
                self.set_latex();
                self.kill_args(3);
            }

            // detex.l:373 - <Normal>"\\def" {BEGIN Define; IGNORE;}
            "def" => {
                self.state = State::Define;
                self.ignore();
            }

            // detex.l:434 - <Normal>"\\slash" putchar('/');
            "slash" => {
                if !self.opts.word {
                    let _ = write!(self.output, "/");
                }
            }

            // detex.l:437 - \\(aa|AA|ae|AE|oe|OE|ss)[ \t]*[ \t\n}] - ligatures (2 char)
            "aa" | "AA" | "ae" | "AE" | "oe" | "OE" | "ss" => {
                if !self.opts.word {
                    let _ = write!(self.output, "{}", cmd);
                }
                // Consume trailing whitespace or }
                if let Some(c) = self.peek_char()
                    && (c.is_whitespace() || c == '}')
                {
                    self.next_char();
                }
            }

            // detex.l:438 - \\[OoijLl][ \t]*[ \t\n}] - ligatures (1 char)
            "O" | "o" | "i" | "j" | "L" | "l" => {
                if !self.opts.word {
                    let _ = write!(self.output, "{}", cmd);
                }
                // Consume trailing whitespace or }
                if let Some(c) = self.peek_char()
                    && (c.is_whitespace() || c == '}')
                {
                    self.next_char();
                }
            }

            // detex.l:439 - <Normal>"\\linebreak"(\[[0-4]\])? {NEWLINE;}
            "linebreak" => {
                self.skip_optional_bracket_arg();
                self.newline();
            }

            // detex.l:441 - <Normal>\\[a-zA-Z@]+ - unknown commands -> Control state
            _ => {
                self.state = State::Control;
                self.ignore();
            }
        }

        Ok(())
    }

    /// detex.l:373-376 - Define state (inside \def, waiting for '{')
    /// <Define>"{"   BEGIN Normal;
    /// <Define>"\n"  NEWLINE;
    /// <Define>.     ;
    fn process_define(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('{') => self.state = State::Normal,
            Some('\n') => self.newline(),
            Some(_) | None => {}
        }
        Ok(())
    }

    /// detex.l:390-394 - Display state (inside $$...$$)
    /// <Display>"$$"           BEGIN Normal;
    /// <Display>"\n"           NEWLINE;
    /// <Display>{VERBSYMBOL}   VERBNOUN;
    /// <Display>.              ;
    fn process_display(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('$') => {
                if self.peek_char() == Some('$') {
                    self.next_char();
                    self.state = State::Normal;
                } else {
                    self.check_verb_symbol('$');
                }
            }
            Some('\n') => self.newline(),
            Some(c) => self.check_verb_symbol(c),
            None => {}
        }
        Ok(())
    }

    /// detex.l:396-401 - Math state (inside $...$)
    /// <Math>"$"            BEGIN Normal;
    /// <Math>"\n"           ;
    /// <Math>"\\$"          ;
    /// <Math>{VERBSYMBOL}   VERBNOUN;
    /// <Math>.              ;
    fn process_math(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('$') => self.state = State::Normal,
            Some('\\') => {
                if self.peek_char() == Some('$') {
                    self.next_char(); // escaped $ in math mode
                } else {
                    let cmd = self.read_command_name();
                    if is_verb_symbol(&cmd) {
                        self.verb_noun();
                    }
                }
            }
            Some('\n') => {} // No NEWLINE in Math state
            Some(c) => self.check_verb_symbol(c),
            None => {}
        }
        Ok(())
    }

    /// Check if character/command is a VERBSYMBOL and call verb_noun() if so.
    /// detex.l:204 defines VERBSYMBOL:
    /// VERBSYMBOL = |\\leq|\\geq|\\in|>|<|\\subseteq|\subseteq|\\subset|\\supset|\\sim|\\neq|\\mapsto
    /// This includes the character symbols: = > <
    /// And the LaTeX command symbols: \leq \geq \in \subseteq \subset \supset \sim \neq \mapsto
    fn check_verb_symbol(&mut self, c: char) {
        match c {
            // detex.l:204 - character symbols: = > <
            '=' | '>' | '<' => self.verb_noun(),
            '\\' => {
                let cmd = self.read_command_name();
                // detex.l:204 - command symbols like \leq, \geq, etc.
                if is_verb_symbol(&cmd) {
                    self.verb_noun();
                }
            }
            _ => {}
        }
    }

    /// detex.l:451-456 - Control state (after unknown \command)
    /// <Control>\\[a-zA-Z@]+              IGNORE;
    /// <Control>[a-zA-Z@0-9]*[-'=`][^ \t\n{]*  IGNORE;
    /// <Control>"\n"                      {BEGIN Normal;}
    /// <Control>[ \t]*[{]+                {++currBracesLevel;BEGIN Normal; IGNORE;}
    /// <Control>[ \t]*                    {BEGIN Normal; IGNORE;}
    /// <Control>.                         {yyless(0);BEGIN Normal;}
    fn process_control(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('\n') => self.state = State::Normal,
            Some(c) if c.is_whitespace() => {
                self.skip_whitespace();
                if self.peek_char() == Some('{') {
                    self.next_char();
                    self.current_braces_level += 1;
                }
                self.state = State::Normal;
                self.ignore();
            }
            Some('{') => {
                self.current_braces_level += 1;
                self.state = State::Normal;
                self.ignore();
            }
            Some('\\') => {
                let _ = self.read_command_name();
                self.ignore();
            }
            Some(c)
                if c.is_ascii_alphanumeric() || c == '-' || c == '\'' || c == '=' || c == '`' => {}
            Some(c) => {
                self.unget_char(c);
                self.state = State::Normal;
            }
            None => self.state = State::Normal,
        }
        Ok(())
    }

    /// detex.l:410-417 - IncludeOnly state (parsing \includeonly{file1,file2,...})
    /// <IncludeOnly>[^{ \t,\n}]+   AddInclude(yytext);
    /// <IncludeOnly>"}"            {if (csbIncList==0) rgsbIncList[csbIncList++]='\0'; BEGIN Normal;}
    /// <IncludeOnly>"\n"+          NEWLINE;
    /// <IncludeOnly>.              ;
    fn process_include_only(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => {
                self.next_char();
            }
            Some('}') => {
                self.next_char();
                if self.opts.include_list.is_empty() {
                    self.opts.include_list.push(String::new());
                }
                self.state = State::Normal;
            }
            Some(',') => {
                self.next_char();
            }
            Some('\n') => {
                self.newline();
            }
            Some(_) => {
                let mut filename = String::new();
                while let Some(c) = self.peek_char() {
                    if c == ',' || c == '}' || c.is_whitespace() {
                        break;
                    }
                    filename.push(self.next_char().unwrap());
                }
                if !filename.is_empty() {
                    self.add_include(&filename);
                }
            }
            None => {}
        }
        Ok(())
    }

    /// detex.l:426-431 - Input state (parsing \input filename)
    /// <Input>[^{ \t\n}]+   {InputFile(yytext); BEGIN Normal;}
    /// <Input>"\n"+         NEWLINE;
    /// <Input>.             ;
    fn process_input(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => {
                self.next_char();
            }
            Some('\n') => {
                self.newline();
            }
            Some(c) if !c.is_whitespace() && c != '}' => {
                let mut filename = String::new();
                while let Some(c) = self.peek_char() {
                    if c.is_whitespace() || c == '}' {
                        break;
                    }
                    filename.push(self.next_char().unwrap());
                }
                if !filename.is_empty() {
                    self.input_file(&filename)?;
                }
                self.state = State::Normal;
            }
            Some(_) | None => self.state = State::Normal,
        }
        Ok(())
    }

    /// detex.l:262-264 - LaEnv state (absorbing ignored environment content)
    /// <LaEnv>"\\end"  {LaBEGIN LaEnd; IGNORE;}
    /// <LaEnv>"\n"+    ;  (newlines are consumed but not processed)
    /// <LaEnv>.        {INCRLINENO;}
    fn process_la_env(&mut self) -> Result<(), String> {
        match self.peek_char() {
            Some('\\') => {
                self.next_char();
                if self.try_match("end") {
                    self.la_begin(State::LaEnd);
                    self.ignore();
                } else {
                    // Consumed '\' - track it (no newline)
                }
            }
            Some('\n') => {
                // detex.l:263 - newlines in ignored environments
                self.next_char();
                // Track the newline
                if let Some(source) = self.current_source_mut() {
                    source.incr_line();
                }
                self.at_column_zero = true;
            }
            Some(_) => {
                // detex.l:264 - any other character calls INCRLINENO
                // (though since we're consuming one char at a time, we only increment if it's '\n')
                self.next_char();
            }
            None => {}
        }
        Ok(())
    }

    /// detex.l:266-272 - LaEnd state (parsing \end{envname})
    /// <LaEnd>{W}   {if (EndEnv(yytext)) BEGIN Normal; IGNORE;}
    /// <LaEnd>"}"   {BEGIN LaEnv; IGNORE;}
    /// <LaEnd>.     {INCRLINENO;}
    fn process_la_end(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => {
                self.next_char();
                self.skip_whitespace();
                let env = self.read_command_name();
                self.skip_whitespace();
                // Don't consume the '}' here - let it be matched separately
                // to match opendetex behavior where '}' in LaEnd calls IGNORE

                if self.end_env(&env) {
                    self.state = State::Normal;
                } else {
                    self.state = State::LaEnv;
                }
                self.ignore();
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let env = self.read_command_name();
                if self.end_env(&env) {
                    self.state = State::Normal;
                }
                self.ignore();
            }
            Some('}') => {
                self.next_char();
                self.state = State::LaEnv;
                self.ignore();
            }
            Some('\n') => {
                self.next_char();
            }
            Some(_) => {
                self.next_char();
            }
            None => self.state = State::Normal,
        }
        Ok(())
    }

    /// detex.l:384-388 - LaDisplay state (inside \[...\])
    /// <LaDisplay>"\\]"         BEGIN Normal;
    /// <LaDisplay>"\n"          NEWLINE;
    /// <LaDisplay>{VERBSYMBOL}  VERBNOUN;
    /// <LaDisplay>.             ;
    fn process_la_display(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('\\') => {
                if self.try_match("]") {
                    self.state = State::Normal;
                } else {
                    let cmd = self.read_command_name();
                    if is_verb_symbol(&cmd) {
                        self.verb_noun();
                    }
                }
            }
            Some('\n') => self.newline(),
            Some(c) => self.check_verb_symbol(c),
            None => {}
        }
        Ok(())
    }

    /// detex.l:378-382 - LaFormula state (inside \(...\))
    /// <LaFormula>"\\)"         BEGIN Normal;
    /// <LaFormula>"\n"          NEWLINE;
    /// <LaFormula>{VERBSYMBOL}  VERBNOUN;
    /// <LaFormula>.             ;
    fn process_la_formula(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('\\') => {
                if self.try_match(")") {
                    self.state = State::Normal;
                } else {
                    let cmd = self.read_command_name();
                    if is_verb_symbol(&cmd) {
                        self.verb_noun();
                    }
                }
            }
            Some('\n') => self.newline(),
            Some(c) => self.check_verb_symbol(c),
            None => {}
        }
        Ok(())
    }

    /// detex.l:403-408 - LaInclude state (parsing \include{filename})
    /// Also handles LaSubfile (detex.l:419-424) which has identical rules.
    /// <LaInclude>[^{ \t\n}]+   {IncludeFile(yytext); BEGIN Normal;}
    /// <LaInclude>"\n"+         NEWLINE;
    /// <LaInclude>.             ;
    fn process_la_include(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => {
                self.next_char();
            }
            Some('\n') => {
                self.newline();
            }
            Some(c) if !c.is_whitespace() && c != '}' => {
                let mut filename = String::new();
                while let Some(c) = self.peek_char() {
                    if c.is_whitespace() || c == '}' {
                        break;
                    }
                    filename.push(self.next_char().unwrap());
                }
                if !filename.is_empty() {
                    self.include_file(&filename)?;
                }
                self.state = State::Normal;
            }
            Some(_) | None => self.state = State::Normal,
        }
        Ok(())
    }

    /// detex.l:487-498 - LaMacro state (consuming N brace-delimited arguments)
    /// <LaMacro>"\["              { BEGIN LaOptArg; }
    /// <LaMacro>"{"               { cOpenBrace++; }
    /// <LaMacro>"}""\n"{0,1}      { cOpenBrace--; INCRLINENO;
    ///                              if (cOpenBrace == 0) {
    ///                                if (--cArgs==0)
    ///                                  BEGIN Normal;
    ///                              }
    ///                            }
    /// <LaMacro>.                 ;
    fn process_la_macro(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:487
            Some('[') => self.state = State::LaOptArg,
            // detex.l:488
            Some('{') => self.open_braces += 1,
            // detex.l:489-495
            Some('}') => {
                self.open_braces = self.open_braces.saturating_sub(1);
                if self.open_braces == 0 {
                    self.args_count = self.args_count.saturating_sub(1);
                    if self.args_count == 0 {
                        // detex.l:489 - "}""\n"{0,1} - consume optional trailing newline
                        // detex.l:489 calls INCRLINENO to track newlines in matched text
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                            if let Some(source) = self.current_source_mut() {
                                source.incr_line();
                            }
                            self.at_column_zero = true;
                        }
                        self.state = State::Normal;
                    }
                }
            }
            // detex.l:496 - <LaMacro>. - ignore all other characters
            Some('\n') => {
                // Track newlines in consumed content
                if let Some(source) = self.current_source_mut() {
                    source.incr_line();
                }
                self.at_column_zero = true;
            }
            Some(_) | None => {}
        }
        Ok(())
    }

    /// detex.l:497-498 - LaOptArg state (inside optional [...] for LaMacro)
    /// <LaOptArg>"\]"    BEGIN LaMacro;
    /// <LaOptArg>[^\]]*  ;
    fn process_la_opt_arg(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:497
            Some(']') => self.state = State::LaMacro,
            // detex.l:498 - ignore everything else
            Some('\n') => {
                // Track newlines in consumed content
                if let Some(source) = self.current_source_mut() {
                    source.incr_line();
                }
                self.at_column_zero = true;
            }
            Some(_) | None => {}
        }
        Ok(())
    }

    /// detex.l:500-514 - LaMacro2 state (consuming N-1 args, keeping last)
    /// <LaMacro2>"\["    { BEGIN LaOptArg2; }
    /// <LaMacro2>"{"     { if (cOpenBrace == 0) {
    ///                       if (--cArgs==0) {
    ///                         BEGIN Normal;
    ///                         cOpenBrace--;
    ///                       }
    ///                     }
    ///                     cOpenBrace++;
    ///                   }
    /// <LaMacro2>"}"     { cOpenBrace--; }
    /// <LaMacro2>.       ;
    fn process_la_macro2(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:500
            Some('[') => self.state = State::LaOptArg2,
            // detex.l:501-510
            Some('{') => {
                if self.open_braces == 0 {
                    self.args_count = self.args_count.saturating_sub(1);
                    if self.args_count == 0 {
                        self.state = State::Normal;
                        self.open_braces = self.open_braces.wrapping_sub(1);
                    }
                }
                self.open_braces += 1;
            }
            // detex.l:511
            Some('}') => self.open_braces = self.open_braces.saturating_sub(1),
            // detex.l:512 - ignore all other characters
            Some('\n') => {
                // Track newlines in consumed content
                if let Some(source) = self.current_source_mut() {
                    source.incr_line();
                }
                self.at_column_zero = true;
            }
            Some(_) | None => {}
        }
        Ok(())
    }

    /// detex.l:513-514 - LaOptArg2 state (inside optional [...] for LaMacro2)
    /// <LaOptArg2>"\]"  BEGIN LaMacro2;
    /// <LaOptArg2>.     ;
    fn process_la_opt_arg2(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:513
            Some(']') => self.state = State::LaMacro2,
            // detex.l:514 - ignore everything else
            Some('\n') => {
                // Track newlines in consumed content
                if let Some(source) = self.current_source_mut() {
                    source.incr_line();
                }
                self.at_column_zero = true;
            }
            Some(_) | None => {}
        }
        Ok(())
    }

    /// detex.l:225-227 - LaVerbatim state (inside verbatim environment)
    /// <LaVerbatim>"\\end"{S}"{"{S}"verbatim"{S}"}"  BEGIN Normal; IGNORE;
    /// <LaVerbatim>[^\\]+                            ECHO;
    /// <LaVerbatim>.                                 ECHO;
    fn process_la_verbatim(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:225 - check for \end{verbatim}
            Some('\\') => {
                if self.try_match("end") {
                    self.skip_whitespace();
                    if self.try_match("{") {
                        self.skip_whitespace();
                        if self.try_match("verbatim") {
                            self.skip_whitespace();
                            self.try_match("}");
                            self.state = State::Normal;
                            self.ignore();
                            return Ok(());
                        }
                    }
                }
                // detex.l:227 - if not \end{verbatim}, echo the backslash
                let _ = write!(self.output, "\\");
            }
            // detex.l:226-227 - echo all other characters
            Some(c) => self.echo(c),
            None => {}
        }
        Ok(())
    }

    /// detex.l:300-303 - LaPicture state (parsing \includegraphics{filename})
    /// <LaPicture>"{"         ;
    /// <LaPicture>[^{}]+      { if(fShowPictures) { printf("<Picture %s>", yytext); } }
    /// <LaPicture>"\}"{S}"\n"+  { BEGIN Normal; INCRLINENO; }
    /// <LaPicture>"\}"        BEGIN Normal;
    fn process_la_picture(&mut self) -> Result<(), String> {
        match self.next_char() {
            // detex.l:300 - skip opening brace
            Some('{') => {}
            // detex.l:302-303 - closing brace ends picture mode
            Some('}') => {
                self.state = State::Normal;
                // detex.l:302 - consume trailing whitespace/newlines
                while let Some(c) = self.peek_char() {
                    if c.is_whitespace() {
                        self.next_char();
                        if c == '\n' {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            // detex.l:301 - picture filename
            Some(c) => {
                if self.opts.show_pictures {
                    let mut name = String::new();
                    name.push(c);
                    while let Some(c) = self.peek_char() {
                        if c == '{' || c == '}' {
                            break;
                        }
                        name.push(self.next_char().unwrap());
                    }
                    let _ = write!(self.output, "<Picture {}>", name);
                }
            }
            None => self.state = State::Normal,
        }
        Ok(())
    }
}

/// Check if a LaTeX command name is a verb symbol
/// detex.l:204 - VERBSYMBOL pattern includes these commands
fn is_verb_symbol(cmd: &str) -> bool {
    matches!(
        cmd,
        "leq" | "geq" | "in" | "subseteq" | "subset" | "supset" | "sim" | "neq" | "mapsto"
    )
}
