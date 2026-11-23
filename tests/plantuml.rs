use rushdown::{new_markdown_to_html, parser, renderer::html};
use rushdown_diagram::{
    diagram_html_renderer_extension, diagram_parser_extension, DiagramHtmlRendererOptions,
    DiagramParserOptions, PlantUmlHtmlRenderingOptions,
};

fn render_with(source: &str, renderer_options: DiagramHtmlRendererOptions) -> String {
    let markdown_to_html = new_markdown_to_html(
        parser::Options::default(),
        html::Options::default(),
        diagram_parser_extension(DiagramParserOptions::default()),
        diagram_html_renderer_extension(renderer_options),
    );
    let mut out = String::new();
    markdown_to_html(&mut out, source).unwrap();
    out
}

#[test]
fn test_plantuml() {
    if which::which("plantuml").is_err() {
        eprintln!("plantuml is not installed, skipping test");
        return;
    }

    let out = render_with(
        "```plantuml\n@startuml\nHello <|-- World\n@enduml\n```\n",
        DiagramHtmlRendererOptions::default(),
    );
    assert!(out.contains("<svg"), "{out}");
}

#[test]
fn test_plantuml_command_not_found() {
    let out = render_with(
        "```plantuml\n@startuml\nA -> B\n@enduml\n```\n",
        DiagramHtmlRendererOptions {
            plantuml: PlantUmlHtmlRenderingOptions {
                command: Some(String::from("rushdown-diagram-nonexistent-command")),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(out.contains("<pre class=\"plantuml-error\">"), "{out}");
}
