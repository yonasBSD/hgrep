use crate::chunk::Chunks;
use crate::grep::Match;
use crate::printer::Printer;
use anyhow::{Error, Result};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::overrides::OverrideBuilder;
use ignore::{WalkBuilder, WalkParallel, WalkState};
use rayon::prelude::*;
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Mutex;

// Note: 'main is a lifetime of scope of main() function

#[derive(Default)]
pub struct Config<'main> {
    context_lines: u64,
    no_ignore: bool,
    hidden: bool,
    case_insensitive: bool,
    smart_case: bool,
    globs: Box<[&'main str]>,
    glob_case_insensitive: bool,
    fixed_strings: bool,
    word_regexp: bool,
    follow_symlink: bool,
    multi_line: bool,
    crlf: bool,
}

impl<'main> Config<'main> {
    pub fn new(context_lines: u64) -> Self {
        Self {
            context_lines,
            ..Default::default()
        }
    }

    pub fn no_ignore(&mut self, yes: bool) -> &mut Self {
        self.no_ignore = yes;
        self
    }

    pub fn hidden(&mut self, yes: bool) -> &mut Self {
        self.hidden = yes;
        self
    }

    pub fn case_insensitive(&mut self, yes: bool) -> &mut Self {
        self.case_insensitive = yes;
        if yes {
            self.smart_case = false;
        }
        self
    }

    pub fn smart_case(&mut self, yes: bool) -> &mut Self {
        self.smart_case = yes;
        if yes {
            self.case_insensitive = false;
        }
        self
    }

    pub fn globs(&mut self, globs: impl Iterator<Item = &'main str>) -> &mut Self {
        self.globs = globs.collect();
        self
    }

    pub fn glob_case_insensitive(&mut self, yes: bool) -> &mut Self {
        self.glob_case_insensitive = yes;
        self
    }

    pub fn fixed_strings(&mut self, yes: bool) -> &mut Self {
        self.fixed_strings = yes;
        self
    }

    pub fn word_regexp(&mut self, yes: bool) -> &mut Self {
        self.word_regexp = yes;
        self
    }

    pub fn follow_symlink(&mut self, yes: bool) -> &mut Self {
        self.follow_symlink = yes;
        self
    }

    pub fn multi_line(&mut self, yes: bool) -> &mut Self {
        self.multi_line = yes;
        self
    }

    pub fn crlf(&mut self, yes: bool) -> &mut Self {
        self.crlf = yes;
        self
    }

    fn build_walker(&self, mut paths: impl Iterator<Item = &'main OsStr>) -> Result<WalkParallel> {
        let target = paths.next().unwrap();

        let mut builder = OverrideBuilder::new(target);
        if self.glob_case_insensitive {
            builder.case_insensitive(true)?;
        }
        for glob in self.globs.iter() {
            builder.add(glob)?;
        }
        let overrides = builder.build()?;

        let mut builder = WalkBuilder::new(target);
        for path in paths {
            builder.add(path);
        }
        builder
            .hidden(!self.hidden)
            .parents(!self.no_ignore)
            .ignore(!self.no_ignore)
            .git_global(!self.no_ignore)
            .git_ignore(!self.no_ignore)
            .git_exclude(!self.no_ignore)
            .require_git(false)
            .follow_links(self.follow_symlink)
            .overrides(overrides);

        if !self.no_ignore {
            builder.add_custom_ignore_filename(".rgignore");
        }

        Ok(builder.build_parallel())
    }

    fn build_regex_matcher(&self, pat: &str) -> Result<RegexMatcher> {
        let mut builder = RegexMatcherBuilder::new();
        builder
            .case_insensitive(self.case_insensitive)
            .case_smart(self.smart_case)
            .word(self.word_regexp)
            .multi_line(true);

        if self.multi_line {
            if self.crlf {
                builder.crlf(true).line_terminator(None);
            }
        } else {
            builder
                .line_terminator(Some(b'\n'))
                .dot_matches_new_line(false)
                .crlf(self.crlf);
        }

        Ok(if self.fixed_strings {
            builder.build(&regex::escape(pat))?
        } else {
            builder.build(pat)?
        })
    }

    fn build_searcher(&self) -> Searcher {
        let mut builder = SearcherBuilder::new();
        builder
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .multi_line(self.multi_line);
        builder.build()
    }
}

pub fn grep<'main, P: Printer + Send>(
    printer: P,
    pat: &str,
    paths: impl Iterator<Item = &'main OsStr>,
    config: Config<'main>,
) -> Result<()> {
    let paths = walk(paths, &config)?;
    if paths.is_empty() {
        return Ok(());
    }

    let printer = Mutex::new(printer);
    paths.into_par_iter().try_for_each(|path| {
        let matches = search(pat, path, &config)?;
        let printer = printer.lock().unwrap();
        for chunk in Chunks::new(matches.into_iter().map(Ok), config.context_lines) {
            printer.print(chunk?)?;
        }
        Ok(())
    })
}

fn walk<'main>(
    paths: impl Iterator<Item = &'main OsStr>,
    config: &Config<'main>,
) -> Result<Vec<PathBuf>> {
    let walker = config.build_walker(paths)?;
    let (tx, rx) = channel();
    walker.run(|| {
        // This function is called per threads for initialization.
        let tx = tx.clone();
        Box::new(move |entry| {
            let quit = entry.is_err();
            let path = entry.map_err(Error::new);
            tx.send(path).unwrap();
            if quit {
                WalkState::Quit
            } else {
                WalkState::Continue
            }
        })
    });
    drop(tx); // Notify sender finishes

    let mut files = vec![];
    for entry in rx.into_iter() {
        let entry = entry?;
        if entry.file_type().map(|f| f.is_file()).unwrap_or(false) {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

struct Matches {
    multi_line: bool,
    path: PathBuf,
    buf: Vec<Match>,
}

impl Sink for Matches {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        let line_number = mat.line_number().unwrap();
        let path = self.path.clone();
        self.buf.push(Match { path, line_number });
        if self.multi_line {
            for i in 1..mat.lines().count() {
                let line_number = line_number + i as u64;
                let path = self.path.clone();
                self.buf.push(Match { path, line_number });
            }
        }
        Ok(true)
    }
}

fn search(pat: &str, path: PathBuf, config: &Config) -> Result<Vec<Match>> {
    let file = File::open(&path)?;
    let matcher = config.build_regex_matcher(pat)?;
    let mut searcher = config.build_searcher();
    let mut matches = Matches {
        multi_line: config.multi_line,
        path,
        buf: vec![],
    };
    searcher.search_file(&matcher, &file, &mut matches)?;
    Ok(matches.buf)
}