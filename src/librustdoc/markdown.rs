use std::fmt::Write as _;
use std::fs::{create_dir_all, read_to_string, File};
use std::io::prelude::*;
use std::path::Path;

use tempfile::tempdir;

use rustc_span::edition::Edition;
use rustc_span::DUMMY_SP;

use crate::config::{Options, RenderOptions};
use crate::doctest::{generate_args_file, Collector, GlobalTestOptions};
use crate::html::escape::Escape;
use crate::html::markdown;
use crate::html::markdown::{
    find_testable_code, ErrorCodes, HeadingOffset, IdMap, Markdown, MarkdownWithToc,
};

/// Separate any lines at the start of the file that begin with `# ` or `%`.
fn extract_leading_metadata(s: &str) -> (Vec<&str>, &str) {
    let mut metadata = Vec::new();
    let mut count = 0;

    for line in s.lines() {
        if line.starts_with("# ") || line.starts_with('%') {
            // trim the whitespace after the symbol
            metadata.push(line[1..].trim_start());
            count += line.len() + 1;
        } else {
            return (metadata, &s[count..]);
        }
    }

    // if we're here, then all lines were metadata `# ` or `%` lines.
    (metadata, "")
}

/// Render `input` (e.g., "foo.md") into an HTML file in `output`
/// (e.g., output = "bar" => "bar/foo.html").
///
/// Requires session globals to be available, for symbol interning.
pub(crate) fn render<P: AsRef<Path>>(
    input: P,
    options: RenderOptions,
    edition: Edition,
) -> Result<(), String> {
    if let Err(e) = create_dir_all(&options.output) {
        return Err(format!("{output}: {e}", output = options.output.display()));
    }

    let input = input.as_ref();
    let mut output = options.output;
    output.push(input.file_name().unwrap());
    output.set_extension("html");

    let mut css = String::new();
    for name in &options.markdown_css {
        write!(css, r#"<link rel="stylesheet" href="{name}">"#)
            .expect("Writing to a String can't fail");
    }

    let input_str =
        read_to_string(input).map_err(|err| format!("{input}: {err}", input = input.display()))?;
    let playground_url = options.markdown_playground_url.or(options.playground_url);
    let playground = playground_url.map(|url| markdown::Playground { crate_name: None, url });

    let mut out =
        File::create(&output).map_err(|e| format!("{output}: {e}", output = output.display()))?;

    let (metadata, text) = extract_leading_metadata(&input_str);
    if metadata.is_empty() {
        return Err("invalid markdown file: no initial lines starting with `# ` or `%`".to_owned());
    }
    let title = metadata[0];

    let mut ids = IdMap::new();
    let error_codes = ErrorCodes::from(options.unstable_features.is_nightly_build());
    let text = if !options.markdown_no_toc {
        MarkdownWithToc {
            content: text,
            ids: &mut ids,
            error_codes,
            edition,
            playground: &playground,
            // For markdown files, it'll be disabled until the feature is enabled by default.
            custom_code_classes_in_docs: false,
        }
        .into_string()
    } else {
        Markdown {
            content: text,
            links: &[],
            ids: &mut ids,
            error_codes,
            edition,
            playground: &playground,
            heading_offset: HeadingOffset::H1,
            // For markdown files, it'll be disabled until the feature is enabled by default.
            custom_code_classes_in_docs: false,
        }
        .into_string()
    };

    let err = write!(
        &mut out,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="generator" content="rustdoc">
    <title>{title}</title>

    {css}
    {in_header}
</head>
<body class="rustdoc">
    <!--[if lte IE 8]>
    <div class="warning">
        This old browser is unsupported and will most likely display funky
        things.
    </div>
    <![endif]-->

    {before_content}
    <h1 class="title">{title}</h1>
    {text}
    {after_content}
</body>
</html>"#,
        title = Escape(title),
        css = css,
        in_header = options.external_html.in_header,
        before_content = options.external_html.before_content,
        text = text,
        after_content = options.external_html.after_content,
    );

    match err {
        Err(e) => Err(format!("cannot write to `{output}`: {e}", output = output.display())),
        Ok(_) => Ok(()),
    }
}

/// Runs any tests/code examples in the markdown file `input`.
pub(crate) fn test(options: Options) -> Result<(), String> {
    use rustc_session::config::Input;
    let input_str = match &options.input {
        Input::File(path) => {
            read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?
        }
        Input::Str { name: _, input } => input.clone(),
    };

    let mut opts = GlobalTestOptions::default();
    opts.no_crate_inject = true;

    let temp_dir =
        tempdir().map_err(|error| format!("failed to create temporary directory: {error:?}"))?;
    let file_path = temp_dir.path().join("rustdoc-cfgs");
    generate_args_file(&file_path, &options)?;

    let mut collector = Collector::new(
        options.input.filestem().to_string(),
        options.clone(),
        true,
        opts,
        None,
        options.input.opt_path().map(ToOwned::to_owned),
        options.enable_per_target_ignores,
        file_path,
    );
    collector.set_position(DUMMY_SP);
    let codes = ErrorCodes::from(options.unstable_features.is_nightly_build());

    // For markdown files, custom code classes will be disabled until the feature is enabled by default.
    find_testable_code(
        &input_str,
        &mut collector,
        codes,
        options.enable_per_target_ignores,
        None,
        false,
    );

    crate::doctest::run_tests(options.test_args, options.nocapture, collector.tests);
    Ok(())
}
