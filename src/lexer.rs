//! Lexer state machine for stripping TeX/LaTeX commands.

use std::io::{Read, Write};

use crate::config::{Options, MAX_FILE_STACK};
use crate::file_handler::{in_include_list, tex_open, CharSource};

/// Lexer states matching the original flex states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Define,
    Display,
    IncludeOnly,
    Input,
    Math,
    Control,
    LaDisplay,
    LaEnd,
    LaEnv,
    LaFormula,
    LaInclude,
    LaMacro,
    LaOptArg,
    LaMacro2,
    LaOptArg2,
    LaVerbatim,
    LaPicture,
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
        let (mut file, path) = tex_open(filename, &self.opts)
            .ok_or_else(|| format!("can't open file {}", filename))?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| format!("error reading {}: {}", filename, e))?;

        let source = CharSource::new(content);
        let name = path.to_string_lossy().to_string();

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

    fn print_prefix(&mut self) {
        if self.opts.src_loc && self.at_column_zero {
            let filename = self.current_filename().to_string();
            let line = self.current_line();
            let _ = write!(self.output, "{}:{}: ", filename, line);
            self.at_column_zero = false;
        }
    }

    fn echo(&mut self, c: char) {
        self.print_prefix();
        let _ = write!(self.output, "{}", c);
    }

    fn echo_str(&mut self, s: &str) {
        self.print_prefix();
        let _ = write!(self.output, "{}", s);
    }

    fn newline(&mut self) {
        if self.opts.word {
            return;
        }
        self.print_prefix();
        let _ = writeln!(self.output);
        self.at_column_zero = true;
    }

    fn space(&mut self) {
        if !self.opts.word {
            let _ = write!(self.output, " ");
        }
    }

    fn noun(&mut self) {
        if self.opts.space && !self.opts.word && !self.opts.replace {
            let _ = write!(self.output, " ");
        } else if self.opts.replace {
            let _ = write!(self.output, "noun");
        }
    }

    fn verb_noun(&mut self) {
        if self.opts.replace {
            let _ = write!(self.output, " verbs noun");
        }
    }

    fn ignore(&mut self) {
        if self.opts.space && !self.opts.word {
            let _ = write!(self.output, " ");
        }
    }

    // ========== Helper methods ==========

    fn la_begin(&mut self, new_state: State) {
        if self.opts.is_latex() {
            self.state = new_state;
        }
    }

    fn set_latex(&mut self) {
        if !self.opts.force_tex {
            self.opts.latex = true;
        }
    }

    fn kill_args(&mut self, n: usize) {
        self.args_count = n;
        self.open_braces = 0;
        self.la_begin(State::LaMacro);
    }

    fn strip_args(&mut self, n: usize) {
        self.args_count = n;
        self.open_braces = 0;
        self.la_begin(State::LaMacro2);
    }

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

    fn end_env(&mut self, env: &str) -> bool {
        if !self.opts.is_latex() {
            return false;
        }
        env == self.current_ignored_env
    }

    fn include_file(&mut self, filename: &str) -> Result<(), String> {
        if self.opts.no_follow {
            return Ok(());
        }
        if !in_include_list(filename, &self.opts) {
            return Ok(());
        }
        self.input_file(filename)
    }

    fn input_file(&mut self, filename: &str) -> Result<(), String> {
        if self.opts.no_follow {
            return Ok(());
        }

        if self.file_stack.len() >= MAX_FILE_STACK {
            eprintln!("detex: warning: file stack overflow, ignoring {}", filename);
            return Ok(());
        }

        match tex_open(filename, &self.opts) {
            Some((mut file, path)) => {
                let mut content = String::new();
                if let Err(e) = file.read_to_string(&mut content) {
                    eprintln!("detex: warning: can't read file {}: {}", filename, e);
                    return Ok(());
                }

                let source = CharSource::new(content);
                let name = path.to_string_lossy().to_string();
                self.file_stack.push(FileContext { source, name });
                Ok(())
            }
            None => {
                eprintln!("detex: warning: can't open file {}", filename);
                Ok(())
            }
        }
    }

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
                self.next_char();
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
        if let Some(c) = self.peek_char() {
            if c == '+' || c == '-' {
                self.next_char();
            }
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
            if let Some(c) = self.peek_char() {
                if c == '+' || c == '-' {
                    self.next_char();
                }
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
                        self.next_char();
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    fn skip_optional_bracket_arg(&mut self) {
        self.skip_whitespace();
        if self.peek_char() == Some('[') {
            self.next_char();
            let mut depth = 1;
            while depth > 0 {
                match self.next_char() {
                    Some('[') => depth += 1,
                    Some(']') => depth -= 1,
                    Some('\\') => {
                        self.next_char();
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    // ========== State processors ==========

    fn process_normal(&mut self) -> Result<(), String> {
        let c = match self.next_char() {
            Some(c) => c,
            None => return Ok(()),
        };

        match c {
            '%' => {
                while let Some(c) = self.next_char() {
                    if c == '\n' {
                        break;
                    }
                }
            }

            '\\' => self.process_backslash()?,

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

            '{' => {
                self.current_braces_level += 1;
            }

            '}' => {
                self.current_braces_level = self.current_braces_level.saturating_sub(1);
                if self.current_braces_level as i32 == self.footnote_level {
                    let _ = write!(self.output, ")");
                    self.footnote_level = -100;
                }
            }

            '~' => self.space(),
            '|' => {}

            '!' | '?' => {
                if self.peek_char() == Some('`') {
                    self.next_char();
                } else if !self.opts.word {
                    self.echo(c);
                }
            }

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

            '\n' => {
                if !self.opts.word {
                    self.newline();
                }
            }

            '\t' => {
                if !self.opts.word {
                    let _ = write!(self.output, "\t");
                }
            }

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

            _ => {
                if !self.opts.word {
                    self.echo(c);
                }
            }
        }

        Ok(())
    }

    fn process_backslash(&mut self) -> Result<(), String> {
        let cmd = self.read_command_name();

        if cmd.is_empty() {
            match self.next_char() {
                Some('(') => {
                    self.la_begin(State::LaFormula);
                    self.noun();
                }
                Some('[') => {
                    self.la_begin(State::LaDisplay);
                    self.noun();
                }
                Some('\\') => {
                    self.match_optional_star();
                    self.skip_optional_bracket_arg();
                    self.newline();
                }
                Some(' ') => self.space(),
                Some('%') => {
                    if !self.opts.word {
                        let _ = write!(self.output, "%");
                    }
                }
                Some('$') => {
                    if !self.opts.word {
                        let _ = write!(self.output, "$");
                    }
                }
                Some(_) | None => {
                    self.ignore();
                }
            }
            return Ok(());
        }

        match cmd.as_str() {
            "begin" => {
                self.skip_whitespace();
                if self.try_match("{") {
                    self.skip_whitespace();
                    let env = self.read_command_name();
                    self.skip_whitespace();
                    self.try_match("}");

                    if env == "document" {
                        self.set_latex();
                        while self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                    } else if env == "verbatim" {
                        if self.begin_env("verbatim") {
                            self.state = State::LaEnv;
                        } else {
                            self.state = State::LaVerbatim;
                        }
                    } else if env == "minipage" {
                        if self.begin_env("minipage") {
                            self.state = State::LaEnv;
                        } else {
                            self.kill_args(1);
                        }
                    } else if env == "table" || env == "figure" {
                        self.skip_whitespace();
                        self.skip_optional_bracket_arg();
                        if self.begin_env(&env) {
                            self.state = State::LaEnv;
                        }
                    } else if self.begin_env(&env) {
                        self.state = State::LaEnv;
                    }
                }
                self.ignore();
            }

            "end" => {
                self.skip_whitespace();
                if self.try_match("{") {
                    self.skip_whitespace();
                    let _env = self.read_command_name();
                    self.skip_whitespace();
                    self.try_match("}");
                }
                self.ignore();
            }

            "kern" | "vskip" | "hskip" => {
                self.skip_glue();
            }
            "vspace" | "hspace" => {
                self.match_optional_star();
                self.skip_brace_arg();
            }
            "addvspace" => {
                self.skip_brace_arg();
            }

            "newlength" | "newsavebox" | "usebox" | "parbox" | "rotatebox" | "color"
            | "pagecolor" | "bibitem" | "bibliography" | "bibstyle" | "index" | "label"
            | "pagestyle" | "thispagestyle" | "addfontfeature" | "fontspec" | "hypersetup"
            | "sbox" => {
                self.kill_args(1);
                self.ignore();
            }

            "setlength" | "addtolength" | "settowidth" | "settoheight" | "settodepth"
            | "savebox" | "setcounter" | "addtocounter" | "stepcounter" => {
                self.kill_args(2);
                self.ignore();
            }

            "raisebox" | "scalebox" | "foilhead" => {
                self.strip_args(2);
            }
            "resizebox" => {
                self.match_optional_star();
                self.kill_args(2);
            }

            "definecolor" | "fcolorbox" | "addcontentsline" => {
                self.kill_args(3);
            }
            "textcolor" | "colorbox" => {
                self.kill_args(2);
            }

            "includegraphics" => {
                while self.peek_char() == Some('[') {
                    self.skip_optional_bracket_arg();
                }
                self.state = State::LaPicture;
            }

            "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
            | "subparagraph" => {
                self.match_optional_star();
            }

            "cite" => {
                self.kill_args(1);
            }
            "nameref" | "pageref" | "ref" => {
                if self.opts.is_latex() && !self.opts.cite {
                    self.kill_args(1);
                }
                self.ignore();
            }

            "documentstyle" | "documentclass" => {
                self.set_latex();
                self.kill_args(1);
                self.ignore();
            }
            "usepackage" => {
                self.kill_args(1);
                self.ignore();
            }

            "include" => {
                self.la_begin(State::LaInclude);
                self.ignore();
            }
            "includeonly" => {
                self.state = State::IncludeOnly;
                self.ignore();
            }
            "subfile" => {
                self.la_begin(State::LaInclude);
                self.ignore();
            }
            "input" => {
                self.state = State::Input;
                self.ignore();
            }

            "footnote" => {
                self.skip_optional_bracket_arg();
                if self.try_match("{") {
                    let _ = write!(self.output, "(");
                    self.footnote_level = self.current_braces_level as i32;
                    self.current_braces_level += 1;
                }
            }

            "verb" => {
                if self.opts.is_latex() {
                    if let Some(delim) = self.next_char() {
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
            }

            "newcommand" | "renewcommand" => {
                self.set_latex();
                self.kill_args(2);
            }
            "newenvironment" => {
                self.set_latex();
                self.kill_args(3);
            }
            "newcounter" => {
                self.kill_args(1);
            }

            "def" => {
                self.state = State::Define;
                self.ignore();
            }

            "slash" => {
                if !self.opts.word {
                    let _ = write!(self.output, "/");
                }
            }

            "aa" | "AA" | "ae" | "AE" | "oe" | "OE" | "ss" => {
                if !self.opts.word {
                    let _ = write!(self.output, "{}", cmd);
                }
                if let Some(c) = self.peek_char() {
                    if c.is_whitespace() || c == '}' {
                        self.next_char();
                    }
                }
            }

            "O" | "o" | "i" | "j" | "L" | "l" => {
                if !self.opts.word {
                    let _ = write!(self.output, "{}", cmd);
                }
                if let Some(c) = self.peek_char() {
                    if c.is_whitespace() || c == '}' {
                        self.next_char();
                    }
                }
            }

            "linebreak" => {
                self.skip_optional_bracket_arg();
                self.newline();
            }

            "reflectbox" => {}

            _ => {
                self.state = State::Control;
                self.ignore();
            }
        }

        Ok(())
    }

    fn process_define(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('{') => self.state = State::Normal,
            Some('\n') => self.newline(),
            Some(_) | None => {}
        }
        Ok(())
    }

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

    fn process_math(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('$') => self.state = State::Normal,
            Some('\\') => {
                if self.peek_char() == Some('$') {
                    self.next_char();
                } else {
                    let cmd = self.read_command_name();
                    if is_verb_symbol(&cmd) {
                        self.verb_noun();
                    }
                }
            }
            Some('\n') => {}
            Some(c) => self.check_verb_symbol(c),
            None => {}
        }
        Ok(())
    }

    fn check_verb_symbol(&mut self, c: char) {
        match c {
            '=' | '>' | '<' => self.verb_noun(),
            '\\' => {
                let cmd = self.read_command_name();
                if is_verb_symbol(&cmd) {
                    self.verb_noun();
                }
            }
            _ => {}
        }
    }

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
            }
            Some('{') => {
                self.current_braces_level += 1;
                self.state = State::Normal;
            }
            Some('\\') => {
                let _ = self.read_command_name();
                self.ignore();
            }
            Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '\'' || c == '=' || c == '`' => {
            }
            Some(c) => {
                self.unget_char(c);
                self.state = State::Normal;
            }
            None => self.state = State::Normal,
        }
        Ok(())
    }

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

    fn process_la_env(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('\\') => {
                if self.try_match("end") {
                    self.la_begin(State::LaEnd);
                }
            }
            Some('\n') | Some(_) | None => {}
        }
        Ok(())
    }

    fn process_la_end(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        match self.peek_char() {
            Some('{') => {
                self.next_char();
                self.skip_whitespace();
                let env = self.read_command_name();
                self.skip_whitespace();
                self.try_match("}");

                if self.end_env(&env) {
                    self.state = State::Normal;
                } else {
                    self.state = State::LaEnv;
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let env = self.read_command_name();
                if self.end_env(&env) {
                    self.state = State::Normal;
                }
            }
            Some('}') => {
                self.next_char();
                self.state = State::LaEnv;
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

    fn process_la_macro(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('[') => self.state = State::LaOptArg,
            Some('{') => self.open_braces += 1,
            Some('}') => {
                self.open_braces = self.open_braces.saturating_sub(1);
                if self.open_braces == 0 {
                    self.args_count = self.args_count.saturating_sub(1);
                    if self.args_count == 0 {
                        // Consume optional trailing newline after closing brace
                        if self.peek_char() == Some('\n') {
                            self.next_char();
                        }
                        self.state = State::Normal;
                    }
                }
            }
            Some('\n') | Some(_) | None => {}
        }
        Ok(())
    }

    fn process_la_opt_arg(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some(']') => self.state = State::LaMacro,
            Some(_) | None => {}
        }
        Ok(())
    }

    fn process_la_macro2(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('[') => self.state = State::LaOptArg2,
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
            Some('}') => self.open_braces = self.open_braces.saturating_sub(1),
            Some(_) | None => {}
        }
        Ok(())
    }

    fn process_la_opt_arg2(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some(']') => self.state = State::LaMacro2,
            Some(_) | None => {}
        }
        Ok(())
    }

    fn process_la_verbatim(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('\\') => {
                if self.try_match("end") {
                    self.skip_whitespace();
                    if self.try_match("{") {
                        self.skip_whitespace();
                        if self.try_match("verbatim") {
                            self.skip_whitespace();
                            self.try_match("}");
                            self.state = State::Normal;
                        }
                    }
                } else {
                    let _ = write!(self.output, "\\");
                }
            }
            Some(c) => self.echo(c),
            None => {}
        }
        Ok(())
    }

    fn process_la_picture(&mut self) -> Result<(), String> {
        match self.next_char() {
            Some('{') => {}
            Some('}') => {
                self.state = State::Normal;
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

fn is_verb_symbol(cmd: &str) -> bool {
    matches!(
        cmd,
        "leq" | "geq" | "in" | "subseteq" | "subset" | "supset" | "sim" | "neq" | "mapsto"
    )
}
