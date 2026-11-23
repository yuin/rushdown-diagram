#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use core::any::TypeId;
use core::error::Error as CoreError;
use core::fmt;
use core::fmt::Write;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::result::Result as CoreResult;
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::rc::Rc;

use rushdown::{
    as_extension_data, as_extension_data_mut, as_kind_data, as_type_data, as_type_data_mut,
    ast::{
        pp_indent, walk, Arena, Document, InlineChars, KindData, NodeKind, NodeRef, NodeType,
        PrettyPrint, WalkStatus,
    },
    context::{BoolValue, ContextKey, ContextKeyRegistry},
    matches_kind,
    parser::{
        AnyAstTransformer, AstTransformer, Parser, ParserExtension, ParserExtensionFn,
        ParserOptions,
    },
    renderer::{
        self,
        html::{self, renderer_extension, RendererExtension},
        BoxRenderNode, BoxRenderNodeDecorator, NextRenderNode, NodeRenderer, NodeRendererRegistry,
        RenderNode, RenderNodeDecorator, RendererOptions, TextWrite,
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

impl InlineChars for Diagram {}

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

impl ParserOptions for DiagramParserOptions {}

/// Options for the Mermaid diagram parser.
#[derive(Debug, Clone)]
pub struct MermaidParserOptions {
    pub enabled: bool,
}

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

impl AstTransformer for DiagramAstTransformer {
    fn transform(
        &self,
        arena: &mut Arena,
        doc_ref: NodeRef,
        reader: &mut text::BasicReader,
        _ctx: &mut rushdown::parser::Context,
    ) {
        let mut target_codes: Vec<NodeRef> = Vec::new();
        walk(arena, doc_ref, &mut |arena: &Arena,
                                   node_ref: NodeRef,
                                   entering: bool|
         -> Result<WalkStatus> {
            if entering && matches_kind!(arena[node_ref], CodeBlock) {
                let code_block = as_kind_data!(arena[node_ref], CodeBlock);
                let lang = code_block.language(reader.source());
                let lang = lang.str(reader.source()).into_owned();
                if self.is_target(&lang) {
                    target_codes.push(node_ref);
                }
            }
            Ok(WalkStatus::Continue)
        })
        .ok();
        for code_ref in target_codes {
            let code_block = as_kind_data!(arena[code_ref], CodeBlock);
            let lines = code_block.value().clone();
            let pos = arena[code_ref].pos();
            let lang = code_block.language(reader.source());
            let lang = lang.str(reader.source()).into_owned();
            let diagram_type = match lang.as_str() {
                "mermaid" => DiagramType::Mermaid,
                _ => DiagramType::PlantUml,
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

impl DiagramAstTransformer {
    fn is_target(&self, lang: &str) -> bool {
        (lang == "mermaid" && self.options.mermaid.enabled)
            || (lang == "plantuml" && self.options.plantuml.enabled)
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

impl RendererOptions for DiagramHtmlRendererOptions {}

/// Options for the Mermaid diagram HTML renderer.
#[derive(Debug, Clone)]
pub enum MermaidHtmlRenderingOptions {
    /// Use client-side rendering for Mermaid diagrams.
    Client(ClientSideMermaidHtmlRenderingOptions),
    /// Render Mermaid diagrams as SVG by invoking the `mmdc` (mermaid-cli) command.
    Server(ServerSideMermaidHtmlRenderingOptions),
}

impl Default for MermaidHtmlRenderingOptions {
    fn default() -> Self {
        Self::Client(ClientSideMermaidHtmlRenderingOptions::default())
    }
}

/// Options for client-side Mermaid rendering.
#[derive(Debug, Clone)]
pub struct ClientSideMermaidHtmlRenderingOptions {
    /// URL to the Mermaid JavaScript module. The default is the latest version from jsDelivr CDN.
    pub mermaid_url: &'static str,
}

impl Default for ClientSideMermaidHtmlRenderingOptions {
    fn default() -> Self {
        Self {
            mermaid_url: "https://cdn.jsdelivr.net/npm/mermaid@latest/dist/mermaid.esm.min.mjs",
        }
    }
}

/// Options for server-side Mermaid rendering.
#[derive(Debug, Clone, Default)]
pub struct ServerSideMermaidHtmlRenderingOptions {
    /// `mmdc` command path. If `None`, the renderer searches the `PATH`.
    pub command: Option<String>,
    /// Additional arguments passed to the `mmdc` command.
    pub args: Vec<String>,
    /// Themes the diagram is rendered with.
    pub theme: MermaidThemeOptions,
    /// If set, rendered SVGs are written as files under this directory instead of being
    /// embedded inline in the HTML output.
    pub output_dir: Option<PathBuf>,
    /// URL prefix used to reference files written to [`Self::output_dir`].
    /// Defaults to `"/" + the base name of the output directory`.
    pub url_prefix: Option<String>,
}

/// Themes used for server-side Mermaid rendering.
#[derive(Debug, Clone)]
pub enum MermaidThemeOptions {
    /// Render the diagram once.
    Single(MermaidTheme),
    /// Render the diagram twice and embed both into a `<picture>` element, so that browsers
    /// can switch between them with `prefers-color-scheme`.
    Dual {
        light: MermaidTheme,
        dark: MermaidTheme,
    },
}

impl Default for MermaidThemeOptions {
    fn default() -> Self {
        Self::Single(MermaidTheme::default())
    }
}

/// A MermaidJS theme and a background color.
#[derive(Debug, Clone)]
pub struct MermaidTheme {
    pub name: &'static str,
    pub background: &'static str,
}

impl MermaidTheme {
    /// Returns a theme suitable for dark mode.
    pub fn dark() -> Self {
        Self {
            name: "dark",
            background: "transparent",
        }
    }
}

impl Default for MermaidTheme {
    fn default() -> Self {
        Self {
            name: "default",
            background: "white",
        }
    }
}

/// Options for the PlantUML diagram HTML renderer.
#[derive(Debug, Clone, Default)]
pub struct PlantUmlHtmlRenderingOptions {
    /// `plantuml` command path. If `None`, the renderer searches the `PATH`.
    pub command: Option<String>,
    /// Additional arguments passed to the `plantuml` command.
    pub args: Vec<String>,
}

struct DiagramHtmlRenderer<W: TextWrite> {
    _phantom: PhantomData<W>,
    mermaid: MermaidHtmlRenderingOptions,
    plantuml: PlantUmlHtmlRenderingOptions,
    has_mermaid_diagram: ContextKey<BoolValue>,
}

impl<W: TextWrite> DiagramHtmlRenderer<W> {
    fn new(options: DiagramHtmlRendererOptions, reg: Rc<RefCell<ContextKeyRegistry>>) -> Self {
        let has_mermaid_diagram = reg
            .borrow_mut()
            .get_or_create::<BoolValue>(HAS_MERMAID_DIAGRAM);
        Self {
            _phantom: PhantomData,
            mermaid: options.mermaid,
            plantuml: options.plantuml,
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
        let diagram = as_extension_data!(arena, node_ref, Diagram);
        match diagram.diagram_type() {
            DiagramType::Mermaid => match (&self.mermaid, entering) {
                (MermaidHtmlRenderingOptions::Client(_), true) => {
                    ctx.insert(self.has_mermaid_diagram, true);
                    w.write_str("<pre class=\"mermaid\">\n")?;
                    for line in diagram.value().iter(source) {
                        html::write_text(w, &line)?;
                    }
                }
                (MermaidHtmlRenderingOptions::Client(_), false) => w.write_str("</pre>\n")?,
                (MermaidHtmlRenderingOptions::Server(server), true) => {
                    let mut code = String::new();
                    for line in diagram.value().iter(source) {
                        code.push_str(&line);
                    }
                    render_mermaid_server(w, server, &code)?;
                }
                (MermaidHtmlRenderingOptions::Server(_), false) => {}
            },
            DiagramType::PlantUml if entering => {
                let mut buf = String::new();
                for line in diagram.value().iter(source) {
                    buf.push_str(&line);
                }
                match plant_uml(&self.plantuml, buf.as_bytes()) {
                    Ok(svg) => w.write_str(&String::from_utf8_lossy(&svg))?,
                    Err(e) => {
                        w.write_str("<pre class=\"plantuml-error\">")?;
                        html::write_text(w, &format!("Error rendering PlantUML diagram: {e}"))?;
                        w.write_str("</pre>\n")?;
                    }
                }
            }
            DiagramType::PlantUml => {}
        }
        Ok(WalkStatus::Continue)
    }
}

struct MermaidScriptDecorator<W: TextWrite> {
    _phantom: PhantomData<W>,
    mermaid: MermaidHtmlRenderingOptions,
    has_mermaid_diagram: ContextKey<BoolValue>,
}

impl<W: TextWrite> MermaidScriptDecorator<W> {
    fn new(options: DiagramHtmlRendererOptions, reg: Rc<RefCell<ContextKeyRegistry>>) -> Self {
        let has_mermaid_diagram = reg
            .borrow_mut()
            .get_or_create::<BoolValue>(HAS_MERMAID_DIAGRAM);
        Self {
            _phantom: PhantomData,
            mermaid: options.mermaid,
            has_mermaid_diagram,
        }
    }
}

impl<W: TextWrite> RenderNodeDecorator<W> for MermaidScriptDecorator<W> {
    fn decorate_node<'a>(
        &self,
        next: &NextRenderNode<'_, W>,
        w: &mut W,
        source: &'a str,
        arena: &'a Arena,
        node_ref: NodeRef,
        entering: bool,
        ctx: &mut renderer::Context,
    ) -> Result<WalkStatus> {
        if !entering && *ctx.get(self.has_mermaid_diagram).unwrap_or(&false) {
            if let MermaidHtmlRenderingOptions::Client(client) = &self.mermaid {
                w.write_str("<script type=\"module\">\nimport mermaid from '")?;
                w.write_str(client.mermaid_url)?;
                w.write_str("';\n</script>\n")?;
            }
        }
        next.render_node(w, source, arena, node_ref, entering, ctx)
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

impl<'cb, W> NodeRenderer<'cb, W> for MermaidScriptDecorator<W>
where
    W: TextWrite + 'cb,
{
    fn register_node_renderer_fn(self, nrr: &mut impl NodeRendererRegistry<'cb, W>) {
        nrr.register_node_renderer_decorator_fn(
            TypeId::of::<Document>(),
            BoxRenderNodeDecorator::new(self),
        );
    }
}

// }}} Renderer

// Extension {{{

/// Returns a parser extension that parses diagrams.
pub fn diagram_parser_extension(options: impl Into<DiagramParserOptions>) -> impl ParserExtension {
    ParserExtensionFn::new(|p: &mut Parser| {
        p.add_ast_transformer(
            |options: DiagramParserOptions| DiagramAstTransformer { options },
            options.into(),
            100,
        );
    })
}

/// Returns a renderer extension that renders diagrams in HTML.
pub fn diagram_html_renderer_extension<'cb, W>(
    options: impl Into<DiagramHtmlRendererOptions>,
) -> impl RendererExtension<'cb, W>
where
    W: TextWrite + 'cb,
{
    let options = options.into();
    renderer_extension(move |r| {
        r.add_node_renderer(DiagramHtmlRenderer::new, options.clone());
        r.add_node_renderer(MermaidScriptDecorator::<W>::new, options);
    })
}

// }}}

// Utils {{{

fn plant_uml(
    options: &PlantUmlHtmlRenderingOptions,
    src: &[u8],
) -> CoreResult<Vec<u8>, Box<dyn CoreError>> {
    let path = match &options.command {
        Some(command) => std::path::PathBuf::from(command),
        None => which::which("plantuml")?,
    };

    let mut params = vec![
        String::from("-tsvg"),
        String::from("-p"),
        String::from("-Djava.awt.headless=true"),
    ];
    params.extend(options.args.iter().cloned());

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

// Mermaid server-side rendering {{{

struct MermaidVariant {
    label: Option<&'static str>,
    theme: &'static str,
    background: &'static str,
}

impl MermaidThemeOptions {
    fn variants(&self) -> Vec<MermaidVariant> {
        match self {
            Self::Single(theme) => vec![MermaidVariant {
                label: None,
                theme: theme.name,
                background: theme.background,
            }],
            Self::Dual { light, dark } => vec![
                MermaidVariant {
                    label: Some("light"),
                    theme: light.name,
                    background: light.background,
                },
                MermaidVariant {
                    label: Some("dark"),
                    theme: dark.name,
                    background: dark.background,
                },
            ],
        }
    }
}

enum MermaidVariantOutput {
    Inline(Vec<u8>),
    File(String),
}

fn render_mermaid_server<W: TextWrite>(
    w: &mut W,
    options: &ServerSideMermaidHtmlRenderingOptions,
    code: &str,
) -> Result<()> {
    let variants = options.theme.variants();
    let mut outputs = Vec::with_capacity(variants.len());
    let mut errors = Vec::new();
    for variant in &variants {
        match render_mermaid_variant(options, variant, code) {
            Ok(output) => outputs.push(output),
            Err(e) => errors.push(match variant.label {
                Some(label) => format!("{label}: {e}"),
                None => e,
            }),
        }
    }
    if !errors.is_empty() {
        w.write_str("<pre class=\"mermaid-error\">")?;
        html::write_text(w, &errors.join("; "))?;
        w.write_str("</pre>\n")?;
        return Ok(());
    }
    match &options.theme {
        MermaidThemeOptions::Single(_) => write_mermaid_single(w, &outputs[0]),
        MermaidThemeOptions::Dual { .. } => write_mermaid_dual(w, &outputs[0], &outputs[1]),
    }
}

fn write_mermaid_single<W: TextWrite>(w: &mut W, output: &MermaidVariantOutput) -> Result<()> {
    match output {
        MermaidVariantOutput::Inline(svg) => w.write_str(&String::from_utf8_lossy(svg)),
        MermaidVariantOutput::File(url) => {
            w.write_str("<img src=\"")?;
            html::write_text(w, url)?;
            w.write_str("\">\n")
        }
    }
}

fn write_mermaid_dual<W: TextWrite>(
    w: &mut W,
    light: &MermaidVariantOutput,
    dark: &MermaidVariantOutput,
) -> Result<()> {
    w.write_str("<picture>\n<source srcset=\"")?;
    html::write_text(w, &mermaid_src(dark))?;
    w.write_str("\" media=\"(prefers-color-scheme: dark)\">\n<img src=\"")?;
    html::write_text(w, &mermaid_src(light))?;
    w.write_str("\">\n</picture>\n")
}

fn mermaid_src(output: &MermaidVariantOutput) -> String {
    match output {
        MermaidVariantOutput::Inline(svg) => {
            format!("data:image/svg+xml;base64,{}", base64_encode(svg))
        }
        MermaidVariantOutput::File(url) => url.clone(),
    }
}

fn render_mermaid_variant(
    options: &ServerSideMermaidHtmlRenderingOptions,
    variant: &MermaidVariant,
    code: &str,
) -> CoreResult<MermaidVariantOutput, String> {
    let result = match &options.output_dir {
        Some(dir) => {
            render_mermaid_to_file(options, dir, variant, code).map(MermaidVariantOutput::File)
        }
        None => run_mermaid(options, variant, code).map(MermaidVariantOutput::Inline),
    };
    result.map_err(|e| e.to_string())
}

fn render_mermaid_to_file(
    options: &ServerSideMermaidHtmlRenderingOptions,
    dir: &Path,
    variant: &MermaidVariant,
    code: &str,
) -> CoreResult<String, Box<dyn CoreError>> {
    fs::create_dir_all(dir)?;
    let key = mermaid_cache_key(code, variant);
    let filename = match variant.label {
        Some(label) => format!("{key}-{label}.svg"),
        None => format!("{key}.svg"),
    };
    let path = dir.join(&filename);
    if fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
        return Ok(mermaid_file_url(options, &filename));
    }
    let svg = run_mermaid(options, variant, code)?;
    let tmp = dir.join(format!("{filename}.tmp-{}", std::process::id()));
    fs::write(&tmp, &svg)?;
    fs::rename(&tmp, &path)?;
    Ok(mermaid_file_url(options, &filename))
}

fn mermaid_file_url(options: &ServerSideMermaidHtmlRenderingOptions, filename: &str) -> String {
    let prefix = options
        .url_prefix
        .clone()
        .unwrap_or_else(|| default_mermaid_url_prefix(options.output_dir.as_deref()));
    format!("{}/{}", prefix.trim_end_matches('/'), filename)
}

fn default_mermaid_url_prefix(dir: Option<&Path>) -> String {
    match dir.and_then(Path::file_name) {
        Some(name) => format!("/{}", name.to_string_lossy()),
        None => String::from("/"),
    }
}

fn mermaid_cache_key(code: &str, variant: &MermaidVariant) -> String {
    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    variant.theme.hash(&mut hasher);
    variant.background.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn run_mermaid(
    options: &ServerSideMermaidHtmlRenderingOptions,
    variant: &MermaidVariant,
    code: &str,
) -> CoreResult<Vec<u8>, Box<dyn CoreError>> {
    let path = match &options.command {
        Some(command) => PathBuf::from(command),
        None => which::which("mmdc")?,
    };

    let mut cmd = Command::new(path);
    cmd.args([
        "-i",
        "-",
        "-o",
        "-",
        "-e",
        "svg",
        "-t",
        variant.theme,
        "-b",
        variant.background,
    ])
    .args(&options.args)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    {
        let stdin = child.stdin.as_mut().ok_or("Failed to open stdin")?;
        stdin.write_all(code.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = first_mermaid_error_message(&stderr);
    Err(if message.is_empty() {
        format!("mmdc: {}", output.status).into()
    } else {
        format!("mmdc: {message}").into()
    })
}

fn first_mermaid_error_message(stderr: &str) -> &str {
    let message = stderr.trim();
    match message.find("\n\n") {
        Some(index) => message[..index].trim(),
        None => message,
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let rest = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(rest[0]) << 16 | u32::from(rest[1]) << 8 | u32::from(rest[2]);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// }}} Mermaid server-side rendering

// }}} Utils

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn test_base64_encode() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input.as_bytes()), expected);
        }
    }
}
