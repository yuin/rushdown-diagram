#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use rushdown::as_extension_data;
use rushdown::as_extension_data_mut;
use rushdown::as_kind_data;
use rushdown::as_type_data;
use rushdown::as_type_data_mut;
use rushdown::ast::walk;
use rushdown::context::BoolValue;
use rushdown::context::ContextKey;
use rushdown::context::ContextKeyRegistry;
use rushdown::parser::AnyAstTransformer;
use rushdown::parser::AstTransformer;
use rushdown::parser::ParserOptions;
use rushdown::renderer::PostRender;
use rushdown::renderer::Render;

use core::any::TypeId;
use core::error::Error as CoreError;
use core::fmt;
use core::fmt::Write;
use core::result::Result as CoreResult;
use std::cell::RefCell;
use std::io::Write as _;
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;

use rushdown::{
    ast::{pp_indent, Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint, WalkStatus},
    matches_kind,
    parser::{self, Parser, ParserExtension, ParserExtensionFn},
    renderer::{
        self,
        html::{self, Renderer, RendererExtension, RendererExtensionFn},
        BoxRenderNode, NodeRenderer, NodeRendererRegistry, RenderNode, RendererOptions, TextWrite,
    },
    text::{self, Reader},
    Result,
};

// AST {{{

/// A struct representing a diagram in the AST.
#[derive(Debug)]
pub struct Diagram {
    diagram_type: DiagramType,
    value: text::Lines,
}

/// An enum representing the type of a diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagramType {
    #[default]
    Mermaid,
    PlantUml,
}

impl Diagram {
    /// Returns a new [`Diagram`] with the given diagram type.
    pub fn new(diagram_type: DiagramType) -> Self {
        Self {
            diagram_type,
            value: text::Lines::default(),
        }
    }

    /// Returns the type of the diagram.
    #[inline(always)]
    pub fn diagram_type(&self) -> DiagramType {
        self.diagram_type
    }

    /// Returns the value of the diagram as a slice of lines.
    #[inline(always)]
    pub fn value(&self) -> &text::Lines {
        &self.value
    }

    /// Sets the value of the diagram.
    pub fn set_value(&mut self, value: impl Into<text::Lines>) {
        self.value = value.into();
    }
}

impl NodeKind for Diagram {
    fn typ(&self) -> NodeType {
        NodeType::LeafBlock
    }

    fn kind_name(&self) -> &'static str {
        "Diagram"
    }
}

impl PrettyPrint for Diagram {
    fn pretty_print(&self, w: &mut dyn Write, source: &str, level: usize) -> fmt::Result {
        writeln!(
            w,
            "{}DiagramType: {:?}",
            pp_indent(level),
            self.diagram_type()
        )?;
        write!(w, "{}Value: ", pp_indent(level))?;
        writeln!(w, "[ ")?;
        for line in self.value.iter(source) {
            write!(w, "{}{}", pp_indent(level + 1), line)?;
        }
        writeln!(w)?;
        writeln!(w, "{}]", pp_indent(level))
    }
}

impl From<Diagram> for KindData {
    fn from(e: Diagram) -> Self {
        KindData::Extension(Box::new(e))
    }
}

// }}} AST

// Parser {{{

/// Options for the diagram parser.
#[derive(Debug, Clone, Default)]
pub struct DiagramParserOptions {
    pub mermaid: MermaidParserOptions,
    pub plantuml: PlantUmlParserOptions,
}

/// Options for the Mermaid diagram parser.
#[derive(Debug, Clone)]
pub struct MermaidParserOptions {
    pub enabled: bool,
}

impl ParserOptions for DiagramParserOptions {}

impl Default for MermaidParserOptions {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Options for the PlantUML diagram parser.
#[derive(Debug, Clone)]
pub struct PlantUmlParserOptions {
    pub enabled: bool,
}

impl Default for PlantUmlParserOptions {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug)]
struct DiagramAstTransformer {
    options: DiagramParserOptions,
}

impl DiagramAstTransformer {
    pub fn with_options(options: DiagramParserOptions) -> Self {
        Self { options }
    }
}

impl AstTransformer for DiagramAstTransformer {
    fn transform(
        &self,
        arena: &mut Arena,
        doc_ref: NodeRef,
        reader: &mut text::BasicReader,
        _ctx: &mut parser::Context,
    ) {
        let mut target_codes: Option<Vec<NodeRef>> = None;
        walk(arena, doc_ref, &mut |arena: &Arena,
                                   node_ref: NodeRef,
                                   entering: bool|
         -> Result<WalkStatus> {
            if entering && matches_kind!(arena[node_ref], CodeBlock) {
                let code_block = as_kind_data!(arena[node_ref], CodeBlock);
                if let Some(lang) = code_block.language_str(reader.source()) {
                    if lang == "mermaid" || lang == "plantuml" {
                        if target_codes.is_none() {
                            target_codes = Some(Vec::new());
                        }
                        target_codes.as_mut().unwrap().push(node_ref);
                    }
                }
            }
            Ok(WalkStatus::Continue)
        })
        .ok();
        if let Some(target_codes) = target_codes {
            for code_ref in target_codes {
                let code_block = as_kind_data!(arena[code_ref], CodeBlock);
                let lines = code_block.value().clone();
                let pos = arena[code_ref].pos();
                let diagram_type = match code_block.language_str(reader.source()) {
                    Some("mermaid") => {
                        if self.options.mermaid.enabled {
                            DiagramType::Mermaid
                        } else {
                            continue;
                        }
                    }
                    Some("plantuml") => {
                        if self.options.plantuml.enabled {
                            DiagramType::PlantUml
                        } else {
                            continue;
                        }
                    }
                    _ => continue,
                };
                let diagram = arena.new_node(Diagram::new(diagram_type));
                if let Some(pos) = pos {
                    arena[diagram].set_pos(pos);
                }
                as_extension_data_mut!(arena, diagram, Diagram).set_value(lines);
                let hbl = as_type_data!(arena, code_ref, Block).has_blank_previous_line();
                as_type_data_mut!(arena, diagram, Block).set_blank_previous_line(hbl);
                arena[code_ref]
                    .parent()
                    .unwrap()
                    .replace_child(arena, code_ref, diagram);
            }
        }
    }
}

impl From<DiagramAstTransformer> for AnyAstTransformer {
    fn from(t: DiagramAstTransformer) -> Self {
        AnyAstTransformer::Extension(Box::new(t))
    }
}

// }}}

// Renderer {{{

const HAS_MERMAID_DIAGRAM: &str = "rushdown-diagram-hmd";

/// Options for the diagram HTML renderer.
#[derive(Debug, Clone, Default)]
pub struct DiagramHtmlRendererOptions {
    pub mermaid: MermaidHtmlRenderingOptions,
    pub plantuml: PlantUmlHtmlRenderingOptions,
}

/// Options for the Mermaid diagram HTML renderer.
#[derive(Debug, Clone)]
pub enum MermaidHtmlRenderingOptions {
    /// Use client-side rendering for Mermaid diagrams.
    Client(ClientSideMermaidHtmlRendereringOptions),
}

impl Default for MermaidHtmlRenderingOptions {
    fn default() -> Self {
        Self::Client(ClientSideMermaidHtmlRendereringOptions::default())
    }
}

#[derive(Debug, Clone)]
pub struct ClientSideMermaidHtmlRendereringOptions {
    /// URL to the Mermaid JavaScript module. The default is the latest version from jsDelivr CDN.
    pub mermaid_url: &'static str,
}

impl Default for ClientSideMermaidHtmlRendereringOptions {
    fn default() -> Self {
        Self {
            mermaid_url: "https://cdn.jsdelivr.net/npm/mermaid@latest/dist/mermaid.esm.min.mjs",
        }
    }
}

/// Options for the PlantUML diagram HTML renderer.
#[derive(Debug, Clone, Default)]
pub struct PlantUmlHtmlRenderingOptions {
    /// `plantuml` command path. If not specified, the renderer will try to find it in the system
    /// PATH.
    pub command: String,
}

impl RendererOptions for DiagramHtmlRendererOptions {}

struct DiagramHtmlRenderer<W: TextWrite> {
    _phantom: core::marker::PhantomData<W>,
    options: DiagramHtmlRendererOptions,
    writer: html::Writer,
    has_mermaid_diagram: ContextKey<BoolValue>,
}

impl<W: TextWrite> DiagramHtmlRenderer<W> {
    fn new(
        html_opts: html::Options,
        options: DiagramHtmlRendererOptions,
        reg: Rc<RefCell<ContextKeyRegistry>>,
    ) -> Self {
        let has_mermaid_diagram = reg
            .borrow_mut()
            .get_or_create::<BoolValue>(HAS_MERMAID_DIAGRAM);
        Self {
            _phantom: core::marker::PhantomData,
            options,
            writer: html::Writer::with_options(html_opts),
            has_mermaid_diagram,
        }
    }
}

impl<W: TextWrite> RenderNode<W> for DiagramHtmlRenderer<W> {
    fn render_node<'a>(
        &self,
        w: &mut W,
        source: &'a str,
        arena: &'a Arena,
        node_ref: NodeRef,
        entering: bool,
        ctx: &mut renderer::Context,
    ) -> Result<WalkStatus> {
        let kd = as_extension_data!(arena, node_ref, Diagram);
        match kd.diagram_type {
            DiagramType::Mermaid => {
                ctx.insert(self.has_mermaid_diagram, true);
                if matches!(self.options.mermaid, MermaidHtmlRenderingOptions::Client(_)) {
                    if entering {
                        self.writer.write_safe_str(w, "<pre class=\"mermaid\">\n")?;
                        for line in kd.value().iter(source) {
                            self.writer.raw_write(w, &line)?;
                        }
                    } else {
                        self.writer.write_safe_str(w, "</pre>\n")?;
                    }
                }
            }
            DiagramType::PlantUml => {
                if entering {
                    let mut buf = String::new();
                    for line in kd.value().iter(source) {
                        buf.push_str(&line);
                    }
                    match plant_uml(&self.options.plantuml.command, buf.as_bytes(), &[]) {
                        Ok(svg) => {
                            self.writer.write_html(w, &String::from_utf8_lossy(&svg))?;
                        }
                        Err(e) => {
                            self.writer.write_html(
                                w,
                                &format!(
                                    "<pre class=\"plantuml-error\">Error rendering PlantUML diagram: {}</pre>",
                                    e
                                ),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(WalkStatus::Continue)
    }
}

struct DiagramPostRenderHook<W: TextWrite> {
    _phantom: core::marker::PhantomData<W>,
    writer: html::Writer,
    options: DiagramHtmlRendererOptions,

    has_mermaid_diagram: ContextKey<BoolValue>,
}

impl<W: TextWrite> DiagramPostRenderHook<W> {
    pub fn new(
        html_opts: html::Options,
        options: DiagramHtmlRendererOptions,
        reg: Rc<RefCell<ContextKeyRegistry>>,
    ) -> Self {
        let has_mermaid_diagram = reg
            .borrow_mut()
            .get_or_create::<BoolValue>(HAS_MERMAID_DIAGRAM);

        Self {
            _phantom: core::marker::PhantomData,
            writer: html::Writer::with_options(html_opts.clone()),
            options,
            has_mermaid_diagram,
        }
    }
}

impl<W: TextWrite> PostRender<W> for DiagramPostRenderHook<W> {
    fn post_render(
        &self,
        w: &mut W,
        _source: &str,
        _arena: &Arena,
        _node_ref: NodeRef,
        _render: &dyn Render<W>,
        ctx: &mut renderer::Context,
    ) -> Result<()> {
        if *ctx.get(self.has_mermaid_diagram).unwrap_or(&false) {
            #[allow(irrefutable_let_patterns)]
            if let MermaidHtmlRenderingOptions::Client(client_opts) = &self.options.mermaid {
                self.writer.write_html(
                    w,
                    &format!(
                        r#"<script type="module">
import mermaid from '{}';
</script>
"#,
                        client_opts.mermaid_url
                    ),
                )?;
            }
        }
        Ok(())
    }
}

impl<'cb, W> NodeRenderer<'cb, W> for DiagramHtmlRenderer<W>
where
    W: TextWrite + 'cb,
{
    fn register_node_renderer_fn(self, nrr: &mut impl NodeRendererRegistry<'cb, W>) {
        nrr.register_node_renderer_fn(TypeId::of::<Diagram>(), BoxRenderNode::new(self));
    }
}

// }}} Renderer

// Extension {{{

/// Returns a parser extension that parses diagrams.
pub fn diagram_parser_extension(options: impl Into<DiagramParserOptions>) -> impl ParserExtension {
    ParserExtensionFn::new(|p: &mut Parser| {
        p.add_ast_transformer(DiagramAstTransformer::with_options, options.into(), 100);
    })
}

/// Returns a renderer extension that renders diagrams in HTML.
pub fn diagram_html_renderer_extension<'cb, W>(
    options: impl Into<DiagramHtmlRendererOptions>,
) -> impl RendererExtension<'cb, W>
where
    W: TextWrite + 'cb,
{
    RendererExtensionFn::new(move |r: &mut Renderer<'cb, W>| {
        let options = options.into();
        r.add_post_render_hook(DiagramPostRenderHook::new, options.clone(), 500);
        r.add_node_renderer(DiagramHtmlRenderer::new, options);
    })
}

// }}}

// Utils {{{
fn plant_uml(
    command: impl AsRef<str>,
    src: &[u8],
    args: &[&str],
) -> CoreResult<Vec<u8>, Box<dyn CoreError>> {
    let path = if command.as_ref().is_empty() {
        which::which("plantuml")
    } else {
        Ok(std::path::PathBuf::from(command.as_ref()))
    }?;

    let mut params = vec!["-tsvg", "-p", "-Djava.awt.headless=true"];
    params.extend_from_slice(args);

    let mut cmd = Command::new(path);
    cmd.args(&params)
        .env("JAVA_OPTS", "-Djava.awt.headless=true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    {
        let stdin = child.stdin.as_mut().ok_or("Failed to open stdin")?;
        stdin.write_all(src)?;
    }

    let output = child.wait_with_output()?;

    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into())
    }
}
// }}} Utils
