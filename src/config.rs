//! Configuration constants and options for detex.

/// Maximum number of separate paths in TEXINPUTS
pub const MAX_INPUT_PATHS: usize = 10;

/// Maximum file stack depth
pub const MAX_FILE_STACK: usize = 256;

/// Default TEXINPUTS paths
#[cfg(target_os = "windows")]
pub const DEFAULT_INPUTS: &str = ".;/emtex/texinput";
#[cfg(target_os = "windows")]
pub const PATH_SEP: char = ';';

#[cfg(not(target_os = "windows"))]
pub const DEFAULT_INPUTS: &str = ".:/usr/local/tex/inputs";
#[cfg(not(target_os = "windows"))]
pub const PATH_SEP: char = ':';

/// Default list of LaTeX environments to ignore
pub const DEFAULT_ENV: &str = "algorithm,align,array,bmatrix,displaymath,eqnarray,equation,\
    floatfig,floating,longtable,picture,pmatrix,psfrags,pspicture,smallmatrix,smallpmatrix,\
    tabular,tikzpicture,verbatim,vmatrix,wrapfigure";

/// Environment list separator
pub const ENV_SEP: char = ',';

/// Command-line options
#[derive(Debug, Clone)]
pub struct Options {
    /// Echo LaTeX \cite, \ref, and \pageref values
    pub cite: bool,
    /// Force LaTeX mode
    pub latex: bool,
    /// Do not follow \input and \include
    pub no_follow: bool,
    /// Replace control sequences with space
    pub space: bool,
    /// Force TeX mode (inhibit LaTeX mode)
    pub force_tex: bool,
    /// Word-only output (one word per line)
    pub word: bool,
    /// Output source location information
    pub src_loc: bool,
    /// Show picture names
    pub show_pictures: bool,
    /// Replace environments with "noun" for grammar checking
    pub replace: bool,
    /// List of environments to ignore
    pub env_ignore: Vec<String>,
    /// List of includeonly files
    pub include_list: Vec<String>,
    /// Input search paths
    pub input_paths: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cite: false,
            latex: false,
            no_follow: false,
            space: false,
            force_tex: false,
            word: false,
            src_loc: false,
            show_pictures: false,
            replace: false,
            env_ignore: DEFAULT_ENV.split(ENV_SEP).map(String::from).collect(),
            include_list: Vec::new(),
            input_paths: Vec::new(),
        }
    }
}

impl Options {
    /// Create options with custom environment ignore list
    pub fn with_env_ignore(mut self, env_list: &str) -> Self {
        self.env_ignore = env_list
            .split(ENV_SEP)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        self
    }

    /// Set up input paths from environment or defaults
    pub fn setup_input_paths(&mut self) {
        let texinputs = std::env::var("TEXINPUTS").unwrap_or_else(|_| DEFAULT_INPUTS.to_string());

        let mut paths = String::new();

        if texinputs.starts_with(PATH_SEP) {
            paths.push_str(DEFAULT_INPUTS);
        }

        paths.push_str(&texinputs);

        if texinputs.ends_with(PATH_SEP) {
            paths.push_str(DEFAULT_INPUTS);
        }

        self.input_paths = paths
            .split(PATH_SEP)
            .filter(|s| !s.is_empty())
            .take(MAX_INPUT_PATHS)
            .map(String::from)
            .collect();
    }

    /// Check if we're in LaTeX mode (latex flag set and not forced to tex)
    pub fn is_latex(&self) -> bool {
        self.latex && !self.force_tex
    }
}
