use anyhow::{Context, Result};
use clap::{App, Arg};
use hgrep::grep::BufReadExt;
use hgrep::printer::{PrinterOptions, TextWrapMode};
use std::cmp;
use std::env;
use std::io;
use std::process;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "ripgrep")]
use hgrep::ripgrep;

#[cfg(feature = "bat-printer")]
use hgrep::bat::BatPrinter;

#[cfg(feature = "syntect-printer")]
use hgrep::syntect::SyntectPrinter;

fn cli<'a>() -> App<'a> {
    #[cfg(feature = "syntect-printer")]
    const DEFAULT_PRINTER: &str = "syntect";

    #[cfg(all(not(feature = "syntect-printer"), feature = "bat-printer"))]
    const DEFAULT_PRINTER: &str = "bat";

    let app = App::new("hgrep")
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "hgrep is grep with human-friendly search output. It eats an output of `grep -nH` and prints the matches \
            with syntax-highlighted code snippets.\n\n\
            $ grep -nH pattern -R . | hgrep\n\n\
            For more details, visit https://github.com/rhysd/hgrep"
        )
        .arg(
            Arg::new("min-context")
                .short('c')
                .long("min-context")
                .takes_value(true)
                .value_name("NUM")
                .default_value("3")
                .about("Minimum lines of leading and trailing context surrounding each match"),
        )
        .arg(
            Arg::new("max-context")
                .short('C')
                .long("max-context")
                .takes_value(true)
                .value_name("NUM")
                .default_value("6")
                .about("Maximum lines of leading and trailing context surrounding each match"),
        )
        .arg(
            Arg::new("no-grid")
                .short('G')
                .long("no-grid")
                .about("Remove borderlines for more compact output"),
        )
        .arg(
            Arg::new("grid")
                .long("grid")
                .about("Add borderlines to output. This flag is an opposite of --no-grid"),
        )
        .arg(
            Arg::new("tab")
                .long("tab")
                .takes_value(true)
                .value_name("NUM")
                .default_value("4")
                .about("Number of spaces for tab character. Set 0 to pass tabs through directly"),
        )
        .arg(
            Arg::new("theme")
                .long("theme")
                .takes_value(true)
                .value_name("THEME")
                .about("Theme for syntax highlighting. Use --list-themes flag to print the theme list"),
        )
        .arg(
            Arg::new("list-themes")
                .long("list-themes")
                .about("List all available theme names and their samples. Samples show the output where 'let' is searched. The names can be used at --theme option"),
        )
        .arg(
            Arg::new("printer")
                .short('p')
                .long("printer")
                .value_name("PRINTER")
                .default_value(DEFAULT_PRINTER)
                .about("Printer to print the match results. 'bat' or 'syntect' is available"),
        )
        .arg(
            Arg::new("term-width")
                .long("term-width")
                .takes_value(true)
                .value_name("NUM")
                .about("Width (number of characters) of terminal window"),
        ).arg(
            Arg::new("wrap")
                .long("wrap")
                .takes_value(true)
                .value_name("MODE")
                .default_value("char")
                .possible_values(["char", "never"])
                .case_insensitive(true)
                .about("Text-wrapping mode. 'char' enables character-wise text-wrapping. 'never' disables text-wrapping")
        ).arg(
            Arg::new("first-only")
                .short('f')
                .long("first-only")
                .about("Show only the first code snippet per file")
        )
        .arg(
            Arg::new("generate-completion-script")
                .long("generate-completion-script")
                .takes_value(true)
                .value_name("SHELL")
                .possible_values(["bash", "zsh", "powershell", "fish", "elvish"])
                .case_insensitive(true)
                .about("Print completion script for SHELL to stdout"),
        );

    #[cfg(feature = "bat-printer")]
    let app = app.arg(
        Arg::new("custom-assets")
            .long("custom-assets")
            .about("Load bat's custom assets. Note that this flag may not work with some version of `bat` command. This flag is only for bat printer"),
    );

    #[cfg(feature = "syntect-printer")]
    let app = app
        .arg(
            Arg::new("background")
                .long("background")
                .about("Paint background colors. This flag is only for syntect printer"),
        )
        .arg(
            Arg::new("ascii-lines").long("ascii-lines").about(
                "Use ASCII characters for drawing border lines instead of Unicode characters",
            ),
        );

    #[cfg(feature = "ripgrep")]
    let app = app
            .about(
                "hgrep is grep with human-friendly search output. It eats an output of `grep -nH` and prints the \
                matches with syntax-highlighted code snippets.\n\n\
                $ grep -nH pattern -R . | hgrep\n\n\
                hgrep has its builtin grep implementation. It's subset of ripgrep and faster when many matches are found.\n\n\
                $ hgrep pattern\n\n\
                For more details, visit https://github.com/rhysd/hgrep"
            )
            .override_usage("hgrep [FLAGS] [OPTIONS] [PATTERN [PATH...]]")
            .arg(
                Arg::new("no-ignore")
                    .long("no-ignore")
                    .about("Don't respect ignore files (.gitignore, .ignore, etc.)"),
            )
            .arg(
                Arg::new("ignore-case")
                    .short('i')
                    .long("ignore-case")
                    .about("When this flag is provided, the given patterns will be searched case insensitively"),
            )
            .arg(
                Arg::new("smart-case")
                    .short('S')
                    .long("smart-case")
                    .about("Search case insensitively if the pattern is all lowercase. Search case sensitively otherwise"),
            )
            .arg(Arg::new("hidden").long("hidden").about("Search hidden files and directories. By default, hidden files and directories are skipped"))
            .arg(
                Arg::new("glob")
                    .short('g')
                    .long("glob")
                    .takes_value(true)
                    .value_name("GLOB")
                    .multiple_values(true)
                    .allow_hyphen_values(true)
                    .about("Include or exclude files and directories for searching that match the given glob"),
            )
            .arg(
                Arg::new("glob-case-insensitive")
                    .long("glob-case-insensitive")
                    .about("Process glob patterns given with the -g/--glob flag case insensitively"),
            )
            .arg(
                Arg::new("fixed-strings")
                    .short('F')
                    .long("fixed-strings")
                    .about("Treat the pattern as a literal string instead of a regular expression"),
            )
            .arg(
                Arg::new("word-regexp")
                    .short('w')
                    .long("word-regexp")
                    .about("Only show matches surrounded by word boundaries"),
            )
            .arg(
                Arg::new("follow-symlink")
                    .short('L')
                    .long("follow")
                    .about("When this flag is enabled, hgrep will follow symbolic links while traversing directories"),
            )
            .arg(
                Arg::new("multiline")
                    .short('U')
                    .long("multiline")
                    .about("Enable matching across multiple lines"),
            )
            .arg(
                Arg::new("multiline-dotall")
                    .long("multiline-dotall")
                    .about("Enable \"dot all\" in your regex pattern, which causes '.' to match newlines when multiline searching is enabled"),
            )
            .arg(
                Arg::new("crlf")
                    .long("crlf")
                    .about(r"When enabled, hgrep will treat CRLF ('\r\n') as a line terminator instead of just '\n'. This flag is useful on Windows"),
            )
            .arg(
                Arg::new("mmap")
                    .long("mmap")
                    .about("Search using memory maps when possible. mmap is disabled by default unlike ripgrep"),
            )
            .arg(
                Arg::new("max-count")
                    .short('m')
                    .long("max-count")
                    .takes_value(true)
                    .value_name("NUM")
                    .about("Limit the number of matching lines per file searched to NUM"),
            )
            .arg(
                Arg::new("max-depth")
                    .long("max-depth")
                    .takes_value(true)
                    .value_name("NUM")
                    .about("Limit the depth of directory traversal to NUM levels beyond the paths given"),
            )
            .arg(
                Arg::new("line-regexp")
                    .short('x')
                    .long("line-regexp")
                    .about("Only show matches surrounded by line boundaries. This is equivalent to putting ^...$ around the search pattern"),
            )
            .arg(
                Arg::new("pcre2")
                    .short('P')
                    .long("pcre2")
                    .about("When this flag is present, hgrep will use the PCRE2 regex engine instead of its default regex engine"),
            )
            .arg(
                Arg::new("type")
                    .short('t')
                    .long("type")
                    .takes_value(true)
                    .value_name("TYPE")
                    .multiple_occurrences(true)
                    .about("Only search files matching TYPE. This option is repeatable. --type-list can print the list of types"),
            )
            .arg(
                Arg::new("type-not")
                    .short('T')
                    .long("type-not")
                    .takes_value(true)
                    .value_name("TYPE")
                    .multiple_occurrences(true)
                    .about("Do not search files matching TYPE. Inverse of --type. This option is repeatable. --type-list can print the list of types"),
            )
            .arg(
                Arg::new("type-list")
                    .long("type-list")
                    .about("Show all supported file types and their corresponding globs"),
            )
            .arg(
                Arg::new("max-filesize")
                    .long("max-filesize")
                    .takes_value(true)
                    .value_name("NUM+SUFFIX?")
                    .about("Ignore files larger than NUM in size. This does not apply to directories.The input format accepts suffixes of K, M or G which correspond to kilobytes, megabytes and gigabytes, respectively. If no suffix is provided the input is treated as bytes"),
            )
            .arg(
                Arg::new("invert-match")
                    .short('v')
                    .long("invert-match")
                    .about("Invert matching. Show lines that do not match the given patterns"),
            )
            .arg(
                Arg::new("PATTERN")
                    .about("Pattern to search. Regular expression is available"),
            )
            .arg(
                Arg::new("PATH")
                    .about("Paths to search")
                    .multiple_values(true)
                    .value_hint(clap::ValueHint::AnyPath),
            );

    app
}

fn generate_completion_script(shell: &str) {
    use clap_generate::generate;
    use clap_generate::generators::*;

    let mut app = cli();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if shell.eq_ignore_ascii_case("bash") {
        generate(Bash, &mut app, "hgrep", &mut stdout)
    } else if shell.eq_ignore_ascii_case("zsh") {
        generate(Zsh, &mut app, "hgrep", &mut stdout)
    } else if shell.eq_ignore_ascii_case("powershell") {
        generate(PowerShell, &mut app, "hgrep", &mut stdout)
    } else if shell.eq_ignore_ascii_case("fish") {
        generate(Fish, &mut app, "hgrep", &mut stdout)
    } else if shell.eq_ignore_ascii_case("elvish") {
        generate(Elvish, &mut app, "hgrep", &mut stdout)
    } else {
        unreachable!() // SHELL argument was validated by clap
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrinterKind {
    #[cfg(feature = "bat-printer")]
    Bat,
    #[cfg(feature = "syntect-printer")]
    Syntect,
}

fn app() -> Result<bool> {
    let matches = cli().get_matches();
    if let Some(shell) = matches.value_of("generate-completion-script") {
        generate_completion_script(shell);
        return Ok(true);
    }

    #[allow(unused_variables)] // printer_kind is unused when syntect-printer is disabled for now
    let printer_kind = match matches.value_of("printer").unwrap() {
        #[cfg(feature = "bat-printer")]
        "bat" => PrinterKind::Bat,
        #[cfg(not(feature = "bat-printer"))]
        "bat" => anyhow::bail!("--printer bat is not available because 'bat-printer' feature was disabled at compilation"),
        #[cfg(feature = "syntect-printer")]
        "syntect" => PrinterKind::Syntect,
        #[cfg(not(feature = "syntect-printer"))]
        "syntect" => anyhow::bail!("--printer syntect is not available because 'syntect-printer' feature was disabled at compilation"),
        p => anyhow::bail!(
            "Unknown printer '{}', at --printer option. It must be one of 'bat' or 'syntect'",
            p
        ),
    };

    let min_context = matches
        .value_of("min-context")
        .unwrap()
        .parse()
        .context("could not parse \"min-context\" option value as unsigned integer")?;
    let max_context = matches
        .value_of("max-context")
        .unwrap()
        .parse()
        .context("could not parse \"max-context\" option value as unsigned integer")?;
    let max_context = cmp::max(min_context, max_context);

    let mut printer_opts = PrinterOptions::default();
    if let Some(width) = matches.value_of("tab") {
        printer_opts.tab_width = width
            .parse()
            .context("could not parse \"tab\" option value as unsigned integer")?;
    }

    #[cfg(feature = "bat-printer")]
    let theme_env = env::var("BAT_THEME").ok();
    #[cfg(feature = "bat-printer")]
    if printer_kind == PrinterKind::Bat {
        if let Some(var) = &theme_env {
            printer_opts.theme = Some(var);
        }
    }
    if let Some(theme) = matches.value_of("theme") {
        printer_opts.theme = Some(theme);
    }

    let is_grid = matches.is_present("grid");
    #[cfg(feature = "bat-printer")]
    if printer_kind == PrinterKind::Bat {
        if let Ok("plain" | "header" | "numbers") =
            env::var("BAT_STYLE").as_ref().map(String::as_str)
        {
            if !is_grid {
                printer_opts.grid = false;
            }
        }
    }
    if matches.is_present("no-grid") && !is_grid {
        printer_opts.grid = false;
    }

    if let Some(width) = matches.value_of("term-width") {
        let width = width
            .parse()
            .context("could not parse \"term-width\" option value as unsigned integer")?;
        printer_opts.term_width = width;
        if width < 10 {
            anyhow::bail!("Too small value at --term-width option ({} < 10)", width);
        }
    }

    if let Some(mode) = matches.value_of("wrap") {
        if mode.eq_ignore_ascii_case("never") {
            printer_opts.text_wrap = TextWrapMode::Never;
        } else if mode.eq_ignore_ascii_case("char") {
            printer_opts.text_wrap = TextWrapMode::Char;
        } else {
            unreachable!(); // Option value was validated by clap
        }
    }

    if matches.is_present("first-only") {
        printer_opts.first_only = true;
    }

    #[cfg(feature = "syntect-printer")]
    {
        if matches.is_present("background") {
            printer_opts.background_color = true;
            #[cfg(feature = "bat-printer")]
            if printer_kind == PrinterKind::Bat {
                anyhow::bail!("--background flag is only available for syntect printer since bat does not support painting background colors");
            }
        }

        if matches.is_present("ascii-lines") {
            printer_opts.ascii_lines = true;
            #[cfg(feature = "bat-printer")]
            if printer_kind == PrinterKind::Bat {
                anyhow::bail!("--ascii-lines flag is only available for syntect printer since bat does not support this feature");
            }
        }
    }

    #[cfg(feature = "bat-printer")]
    if matches.is_present("custom-assets") {
        printer_opts.custom_assets = true;
        #[cfg(feature = "syntect-printer")]
        if printer_kind == PrinterKind::Syntect {
            anyhow::bail!("--custom-assets flag is only available for bat printer");
        }
    }

    if matches.is_present("list-themes") {
        #[cfg(feature = "syntect-printer")]
        if printer_kind == PrinterKind::Syntect {
            hgrep::syntect::list_themes(io::stdout().lock(), &printer_opts)?;
            return Ok(true);
        }

        #[cfg(feature = "bat-printer")]
        if printer_kind == PrinterKind::Bat {
            BatPrinter::new(printer_opts).list_themes()?;
            return Ok(true);
        }

        unreachable!();
    }

    #[cfg(feature = "ripgrep")]
    if let Some(pattern) = matches.value_of("PATTERN") {
        let paths = matches.values_of_os("PATH");
        let mut config = ripgrep::Config::default();
        config
            .min_context(min_context)
            .max_context(max_context)
            .no_ignore(matches.is_present("no-ignore"))
            .hidden(matches.is_present("hidden"))
            .case_insensitive(matches.is_present("ignore-case"))
            .smart_case(matches.is_present("smart-case"))
            .glob_case_insensitive(matches.is_present("glob-case-insensitive"))
            .pcre2(matches.is_present("pcre2")) // must be before fixed_string
            .fixed_strings(matches.is_present("fixed-strings"))
            .word_regexp(matches.is_present("word-regexp"))
            .follow_symlink(matches.is_present("follow-symlink"))
            .multiline(matches.is_present("multiline"))
            .crlf(matches.is_present("crlf"))
            .multiline_dotall(matches.is_present("multiline-dotall"))
            .mmap(matches.is_present("mmap"))
            .line_regexp(matches.is_present("line-regexp"))
            .invert_match(matches.is_present("invert-match"));

        if matches.is_present("type-list") {
            config.print_types(io::stdout().lock())?;
            return Ok(true);
        }

        let globs = matches.values_of("glob");
        if let Some(globs) = globs {
            config.globs(globs);
        }

        if let Some(num) = matches.value_of("max-count") {
            let num = num
                .parse()
                .context("could not parse --max-count option value as unsigned integer")?;
            config.max_count(num);
        }

        if let Some(num) = matches.value_of("max-depth") {
            let num = num
                .parse()
                .context("could not parse --max-depth option value as unsigned integer")?;
            config.max_depth(num);
        }

        if let Some(size) = matches.value_of("max-filesize") {
            config
                .max_filesize(size)
                .context("coult not parse --max-filesize option value as file size string")?;
        }

        let types = matches.values_of("type");
        if let Some(types) = types {
            config.types(types);
        }

        let types_not = matches.values_of("type-not");
        if let Some(types_not) = types_not {
            config.types_not(types_not);
        }

        #[cfg(feature = "syntect-printer")]
        if printer_kind == PrinterKind::Syntect {
            let printer = SyntectPrinter::with_stdout(printer_opts)?;
            return ripgrep::grep(printer, pattern, paths, config);
        }

        #[cfg(feature = "bat-printer")]
        if printer_kind == PrinterKind::Bat {
            let printer = std::sync::Mutex::new(BatPrinter::new(printer_opts));
            return ripgrep::grep(printer, pattern, paths, config);
        }

        unreachable!();
    }

    #[cfg(feature = "syntect-printer")]
    if printer_kind == PrinterKind::Syntect {
        use hgrep::printer::Printer;
        use rayon::prelude::*;
        let printer = SyntectPrinter::with_stdout(printer_opts)?;
        return io::BufReader::new(io::stdin())
            .grep_lines()
            .chunks_per_file(min_context, max_context)
            .par_bridge()
            .map(|file| {
                printer.print(file?)?;
                Ok(true)
            })
            .try_reduce(|| false, |a, b| Ok(a || b));
    }

    #[cfg(feature = "bat-printer")]
    if printer_kind == PrinterKind::Bat {
        let mut found = false;
        let printer = BatPrinter::new(printer_opts);
        // XXX: io::stdin().lock() is not available since bat's implementation internally takes lock of stdin
        // *even if* it does not use stdin.
        // https://github.com/sharkdp/bat/issues/1902
        for f in io::BufReader::new(io::stdin())
            .grep_lines()
            .chunks_per_file(min_context, max_context)
        {
            printer.print(f?)?;
            found = true;
        }
        return Ok(found);
    }

    unreachable!();
}

fn main() {
    #[cfg(windows)]
    {
        ansi_term::enable_ansi_support().unwrap();
    }

    let status = match app() {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(err) => {
            eprintln!("\x1b[1;91merror:\x1b[0m {}", err);
            2
        }
    };
    process::exit(status);
}
