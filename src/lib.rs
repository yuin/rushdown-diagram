#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use rushdown::as_kind_data;
use rushdown::as_type_data;
use rushdown::as_type_data_mut;
use rushdown::ast::walk;
use rushdown::parser::AnyAstTransformer;
use rushdown::parser::AstTransformer;
use rushdown::renderer::PostRender;
use rushdown::renderer::Render;

use core::any::TypeId;
use core::fmt;
use core::fmt::Write;

use rushdown::{
    ast::{pp_indent, Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint, WalkStatus},
    matches_kind,
    parser::{self, NoParserOptions, Parser, ParserExtension, ParserExtensionFn},
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
}

/// An enum representing the type of a diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiagramType {
    #[default]
    Mermaid,
}

impl Diagram {
    /// Returns a new [`Diagram`] with the given diagram type.
    pub fn new(diagram_type: DiagramType) -> Self {
        Self { diagram_type }
    }

    /// Returns the type of the diagram.
    #[inline(always)]
    pub fn diagram_type(&self) -> DiagramType {
        self.diagram_type
    }
}

impl NodeKind for Diagram {
    fn typ(&self) -> NodeType {
        NodeType::LeafBlock
    }

    fn kind_name(&self) -> &'static str {
        "Diagram"
    }

    fn is_atomic(&self) -> bool {
        true
    }
}

impl PrettyPrint for Diagram {
    fn pretty_print(&self, w: &mut dyn Write, _source: &str, level: usize) -> fmt::Result {
        writeln!(
            w,
            "{}DiagramType: {:?}",
            pp_indent(level),
            self.diagram_type()
        )
    }
}

impl From<Diagram> for KindData {
    fn from(e: Diagram) -> Self {
        KindData::Extension(Box::new(e))
    }
}

// }}} AST

// Parser {{{

#[derive(Debug)]
struct DiagramAstTransformer {}

impl DiagramAstTransformer {
    /// Returns a new [`DiagramAstTransformer`].
    pub fn new() -> Self {
        Self {}
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
                if let Some(lang) = code_block.language(reader.source()) {
                    if lang == "mermaid" {
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
                let diagram_type = match code_block.language(reader.source()) {
                    Some("mermaid") => DiagramType::Mermaid,
                    _ => continue,
                };
                let diagram = arena.new_node(Diagram::new(diagram_type));
                let lines = as_type_data_mut!(arena, code_ref, Block).take_source();
                as_type_data_mut!(arena, diagram, Block).append_source_lines(&lines);
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

/// Options for the diagram HTML renderer.
#[derive(Debug, Clone, Default)]
pub struct DiagramHtmlRendererOptions {
    rendering: DiagramHtmlRenderingOptions,
}

#[derive(Debug, Clone)]
pub enum DiagramHtmlRenderingOptions {
    Mermaid(MermaidHtmlRenderingOptions),
}

impl Default for DiagramHtmlRenderingOptions {
    fn default() -> Self {
        Self::Mermaid(MermaidHtmlRenderingOptions::default())
    }
}

#[derive(Debug, Clone)]
pub enum MermaidHtmlRenderingOptions {
    Client(ClientSideMermaidHtmlRendereringOptions),
}

impl Default for MermaidHtmlRenderingOptions {
    fn default() -> Self {
        Self::Client(ClientSideMermaidHtmlRendereringOptions::default())
    }
}

#[derive(Debug, Clone)]
pub struct ClientSideMermaidHtmlRendereringOptions {
    pub mermaid_url: &'static str,
}

impl Default for ClientSideMermaidHtmlRendereringOptions {
    fn default() -> Self {
        Self {
            mermaid_url: "https://cdn.jsdelivr.net/npm/mermaid@latest/dist/mermaid.esm.min.mjs",
        }
    }
}

impl RendererOptions for DiagramHtmlRendererOptions {}

struct DiagramHtmlRenderer<W: TextWrite> {
    _phantom: core::marker::PhantomData<W>,
    options: DiagramHtmlRendererOptions,
    writer: html::Writer,
}

impl<W: TextWrite> DiagramHtmlRenderer<W> {
    fn new(html_opts: html::Options, options: DiagramHtmlRendererOptions) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            options,
            writer: html::Writer::with_options(html_opts),
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
        _ctx: &mut renderer::Context,
    ) -> Result<WalkStatus> {
        if matches!(
            self.options.rendering,
            DiagramHtmlRenderingOptions::Mermaid(MermaidHtmlRenderingOptions::Client(_))
        ) {
            if entering {
                self.writer.write_safe_str(w, "<pre class=\"mermaid\">\n")?;
                let block = as_type_data!(arena, node_ref, Block);
                for line in block.source().iter() {
                    self.writer.raw_write(w, &line.str(source))?;
                }
            } else {
                self.writer.write_safe_str(w, "</pre>\n")?;
            }
        }
        Ok(WalkStatus::Continue)
    }
}

struct DiagramPostRenderHook<W: TextWrite> {
    _phantom: core::marker::PhantomData<W>,
    writer: html::Writer,
    options: DiagramHtmlRendererOptions,
}

impl<W: TextWrite> DiagramPostRenderHook<W> {
    pub fn new(html_opts: html::Options, options: DiagramHtmlRendererOptions) -> Self {
        Self {
            _phantom: core::marker::PhantomData,
            writer: html::Writer::with_options(html_opts.clone()),
            options,
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
        _ctx: &mut renderer::Context,
    ) -> Result<()> {
        #[allow(irrefutable_let_patterns)]
        if let DiagramHtmlRenderingOptions::Mermaid(MermaidHtmlRenderingOptions::Client(
            client_opts,
        )) = &self.options.rendering
        {
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
pub fn diagram_parser_extension() -> impl ParserExtension {
    ParserExtensionFn::new(|p: &mut Parser| {
        p.add_ast_transformer(DiagramAstTransformer::new, NoParserOptions, 100);
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
