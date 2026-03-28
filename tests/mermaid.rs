use rushdown::{
    new_markdown_to_html, parser,
    renderer::html,
    test::{MarkdownTestCase, MarkdownTestCaseOptions},
};
use rushdown_diagram::{diagram_html_renderer_extension, DiagramHtmlRendererOptions};
use rushdown_diagram::{diagram_parser_extension, DiagramParserOptions};

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
        html::Options {
            allows_unsafe: true,
            xhtml: false,
            ..html::Options::default()
        },
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
