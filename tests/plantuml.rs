use rushdown::{new_markdown_to_html, parser, renderer::html};
use rushdown_diagram::{diagram_html_renderer_extension, DiagramHtmlRendererOptions};
use rushdown_diagram::{diagram_parser_extension, DiagramParserOptions};

#[test]
fn test_plantuml() {
    if which::which("plantuml").is_err() {
        eprintln!("plantuml is not installed, skipping test");
        return;
    }

    let source = r#"
```plantuml
@startuml
Hello <|-- World
@enduml
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
    let mut out = String::new();
    markdown_to_html(&mut out, source).unwrap();
    assert!(
        out.contains("<svg"),
        "Output should contain the plantuml diagram"
    );
}
