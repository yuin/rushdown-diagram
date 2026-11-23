use rushdown::{
    new_markdown_to_html, parser,
    renderer::html,
    test::{MarkdownTestCase, MarkdownTestCaseOptions},
};
use std::fs;
use std::path::{Path, PathBuf};

use rushdown_diagram::{
    diagram_html_renderer_extension, diagram_parser_extension,
    ClientSideMermaidHtmlRenderingOptions, DiagramHtmlRendererOptions, DiagramParserOptions,
    MermaidHtmlRenderingOptions, MermaidParserOptions, MermaidTheme, MermaidThemeOptions,
    ServerSideMermaidHtmlRenderingOptions,
};

fn render_with(
    source: &str,
    parser_options: DiagramParserOptions,
    renderer_options: DiagramHtmlRendererOptions,
) -> String {
    let markdown_to_html = new_markdown_to_html(
        parser::Options::default(),
        html::Options::default(),
        diagram_parser_extension(parser_options),
        diagram_html_renderer_extension(renderer_options),
    );
    let mut out = String::new();
    markdown_to_html(&mut out, source).unwrap();
    out
}

fn render(source: &str) -> String {
    render_with(
        source,
        DiagramParserOptions::default(),
        DiagramHtmlRendererOptions::default(),
    )
}

#[test]
fn test_mermaid() {
    let source = r#"
```mermaid
graph LR
    A --- B
    B-->C[fa:fa-ban forbidden]
    B-->D(fa:fa-spinner);
```
"#;
    let markdown_to_html = new_markdown_to_html(
        parser::Options::default(),
        html::Options::default(),
        diagram_parser_extension(DiagramParserOptions::default()),
        diagram_html_renderer_extension(DiagramHtmlRendererOptions::default()),
    );
    MarkdownTestCase::new(
        1,
        "ok",
        source,
        r#"<pre class="mermaid">
graph LR
    A --- B
    B--&gt;C[fa:fa-ban forbidden]
    B--&gt;D(fa:fa-spinner);
</pre>
<script type="module">
import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@latest/dist/mermaid.esm.min.mjs';
</script>
"#,
        MarkdownTestCaseOptions::default(),
    )
    .execute(&markdown_to_html);
}

#[test]
fn test_escapes_source() {
    let out = render("```mermaid\nA --\"x\"--> B\n```\n");
    assert!(out.contains("&quot;x&quot;"), "{out}");
}

#[test]
fn test_script_emitted_once() {
    let out = render(
        "```mermaid\ngraph LR\n    A --> B\n```\n\n```mermaid\ngraph LR\n    C --> D\n```\n",
    );
    assert_eq!(out.matches("<script type=\"module\">").count(), 1, "{out}");
}

#[test]
fn test_custom_mermaid_url() {
    let out = render_with(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        DiagramParserOptions::default(),
        DiagramHtmlRendererOptions {
            mermaid: MermaidHtmlRenderingOptions::Client(ClientSideMermaidHtmlRenderingOptions {
                mermaid_url: "https://example.com/mermaid.mjs",
            }),
            ..Default::default()
        },
    );
    assert!(
        out.contains("import mermaid from 'https://example.com/mermaid.mjs';"),
        "{out}"
    );
}

#[test]
fn test_non_diagram_code_block_is_unaffected() {
    let out = render("```go\nfunc main() {}\n```\n");
    assert_eq!(
        out,
        "<pre><code class=\"language-go\">func main() {}\n</code></pre>\n"
    );
}

#[test]
fn test_disabled_by_parser_options() {
    let out = render_with(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        DiagramParserOptions {
            mermaid: MermaidParserOptions { enabled: false },
            ..Default::default()
        },
        DiagramHtmlRendererOptions::default(),
    );
    assert_eq!(
        out,
        "<pre><code class=\"language-mermaid\">graph LR\n    A --&gt; B\n</code></pre>\n"
    );
}

fn mmdc_available() -> bool {
    if which::which("mmdc").is_err() {
        eprintln!("mmdc is not installed, skipping test");
        return false;
    }
    true
}

fn render_server(source: &str, server: ServerSideMermaidHtmlRenderingOptions) -> String {
    let mut server = server;
    server.args.extend([
        String::from("-p"),
        String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/puppeteer-config.json"
        )),
    ]);
    render_with(
        source,
        DiagramParserOptions::default(),
        DiagramHtmlRendererOptions {
            mermaid: MermaidHtmlRenderingOptions::Server(server),
            ..Default::default()
        },
    )
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rushdown-diagram-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn test_mermaid_server_rendering_inline() {
    if !mmdc_available() {
        return;
    }
    let out = render_server(
        "```mermaid\ngraph LR\n    A --- B\n```\n",
        ServerSideMermaidHtmlRenderingOptions::default(),
    );
    assert!(out.contains("<svg"), "{out}");
    assert!(!out.contains("<pre class=\"mermaid\">"), "{out}");
    assert!(!out.contains("<script type=\"module\">"), "{out}");
}

#[test]
fn test_mermaid_server_rendering_command_not_found() {
    let out = render_server(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        ServerSideMermaidHtmlRenderingOptions {
            command: Some(String::from("rushdown-diagram-nonexistent-command")),
            ..Default::default()
        },
    );
    assert!(out.contains("<pre class=\"mermaid-error\">"), "{out}");
}

#[test]
fn test_mermaid_server_rendering_dual_theme_inline() {
    if !mmdc_available() {
        return;
    }
    let out = render_server(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        ServerSideMermaidHtmlRenderingOptions {
            theme: MermaidThemeOptions::Dual {
                light: MermaidTheme::default(),
                dark: MermaidTheme::dark(),
            },
            ..Default::default()
        },
    );
    assert!(out.contains("<picture>"), "{out}");
    assert!(
        out.contains("media=\"(prefers-color-scheme: dark)\""),
        "{out}"
    );
    assert_eq!(
        out.matches("data:image/svg+xml;base64,").count(),
        2,
        "{out}"
    );
}

#[test]
fn test_mermaid_server_rendering_output_dir() {
    if !mmdc_available() {
        return;
    }
    let dir = temp_dir("output-dir");
    let options = ServerSideMermaidHtmlRenderingOptions {
        output_dir: Some(dir.clone()),
        ..Default::default()
    };
    let source = "```mermaid\ngraph LR\n    A --> B\n```\n";

    let out = render_server(source, options.clone());
    let prefix = format!(
        "<img src=\"/{}/",
        dir.file_name().unwrap().to_string_lossy()
    );
    assert!(out.contains(&prefix), "{out}");

    let entries = file_names(&dir);
    assert_eq!(entries.len(), 1, "{entries:?}");
    let path = dir.join(&entries[0]);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();

    render_server(source, options);

    let entries_after = file_names(&dir);
    assert_eq!(entries_after.len(), 1, "{entries_after:?}");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_mermaid_server_rendering_url_prefix() {
    if !mmdc_available() {
        return;
    }
    let dir = temp_dir("url-prefix");
    let out = render_server(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        ServerSideMermaidHtmlRenderingOptions {
            output_dir: Some(dir.clone()),
            url_prefix: Some(String::from("/static/diagrams")),
            ..Default::default()
        },
    );
    assert!(out.contains("<img src=\"/static/diagrams/"), "{out}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_mermaid_server_rendering_dual_output_dir() {
    if !mmdc_available() {
        return;
    }
    let dir = temp_dir("dual-output-dir");
    let out = render_server(
        "```mermaid\ngraph LR\n    A --> B\n```\n",
        ServerSideMermaidHtmlRenderingOptions {
            theme: MermaidThemeOptions::Dual {
                light: MermaidTheme::default(),
                dark: MermaidTheme::dark(),
            },
            output_dir: Some(dir.clone()),
            ..Default::default()
        },
    );
    assert!(out.contains("-light.svg"), "{out}");
    assert!(out.contains("-dark.svg"), "{out}");
    assert_eq!(file_names(&dir).len(), 2);
    fs::remove_dir_all(&dir).unwrap();
}
