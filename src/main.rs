//! detex - Strip TeX/LaTeX commands from input files
//!
//! This is a Rust port of the opendetex program originally written
//! by Daniel Trinkle at Purdue University.
//!
//! Copyright (c) 1986-2007 Purdue University
//! All rights reserved.
//!
//! Permission is hereby granted, free of charge, to any person obtaining
//! a copy of this software and associated documentation files (the
//! "Software"), to deal with the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to
//! permit persons to whom the Software is furnished to do so, subject to
//! the following conditions:
//!
//! Redistributions of source code must retain the above copyright notice,
//! this list of conditions and the following disclaimers.

mod config;
mod file_handler;
mod lexer;

use std::io::{self, BufWriter};
use std::process;

use config::{Options, VERSION};
use lexer::Detex;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let program_name = std::path::Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("detex");

    // Check if invoked as 'delatex'
    let mut opts = Options::default();
    if program_name == "delatex" {
        opts.latex = true;
    }

    // Set up input paths
    opts.setup_input_paths();

    // Parse command line arguments
    let mut files: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];

        if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            while j < chars.len() {
                match chars[j] {
                    'c' => opts.cite = true,
                    'e' => {
                        i += 1;
                        if i >= args.len() {
                            error_exit(program_name, "-e option requires an argument");
                        }
                        opts = opts.with_env_ignore(&args[i]);
                    }
                    'l' => opts.latex = true,
                    'n' => opts.no_follow = true,
                    'r' => opts.replace = true,
                    's' => opts.space = true,
                    't' => opts.force_tex = true,
                    'w' => opts.word = true,
                    '1' => opts.src_loc = true,
                    'v' => version_exit(),
                    'h' | '?' => usage_exit(program_name),
                    c => {
                        eprintln!("{}: warning: unknown option ignored -{}", program_name, c);
                        usage_exit(program_name);
                    }
                }
                j += 1;
            }
        } else {
            files.push(arg.clone());
        }

        i += 1;
    }

    // Create buffered stdout for better performance
    let stdout = io::stdout();
    let output = BufWriter::new(stdout.lock());
    let mut detex = Detex::new(opts, output);

    if files.is_empty() {
        if let Err(e) = detex.process_stdin() {
            eprintln!("{}: error: {}", program_name, e);
            process::exit(1);
        }
    } else {
        for file in files {
            if let Err(e) = detex.process_file(&file) {
                eprintln!("{}: warning: {}", program_name, e);
            }
        }
    }
}

fn usage_exit(program_name: &str) -> ! {
    println!(
        "\n{} [ -clnrstw1v ] [ -e environment-list ] [ filename[.tex] ... ]",
        program_name
    );
    println!("Strip (La)TeX commands from the input.\n");
    println!("  -c  echo LaTeX \\cite, \\ref, and \\pageref values");
    println!("  -e  <env-list> list of LaTeX environments to ignore");
    println!("  -l  force latex mode");
    println!("  -n  do not follow \\input, \\include and \\subfile");
    println!("  -r  replace math with \"noun\" and \"noun verbs noun\" to preserve grammar");
    println!("  -s  replace control sequences with space");
    println!("  -t  force tex mode");
    println!("  -w  word only output");
    println!("  -1  outputs the original file name and line number in the beginning of each line");
    println!("  -v  show program version and exit");
    println!("\nRust port of opendetex: https://github.com/pkubowicz/opendetex");
    process::exit(0);
}

fn version_exit() -> ! {
    println!("\nDetex (Rust) version {}", VERSION);
    println!("Rust port of opendetex: https://github.com/pkubowicz/opendetex");
    process::exit(0);
}

fn error_exit(program_name: &str, message: &str) -> ! {
    eprintln!("{}: error: {}", program_name, message);
    process::exit(1);
}
