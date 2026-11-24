//! Linking an ES module graph into one classic script.
//!
//! zipp-vm has a module implementation, but it resolves and loads modules
//! through the FILESYSTEM (`std::fs::read_to_string` throughout
//! `zipp-vm/src/vm/engine/modules.rs`), with no hook for handing it source
//! fetched over HTTP. A page's modules only exist as URLs, so that loader
//! cannot reach them.
//!
//! Rather than rework the VM's module engine, the graph is linked here: every
//! module is fetched, its `import`/`export` statements are rewritten into calls
//! on a tiny runtime registry, and the whole graph is emitted as one classic
//! script that the existing pipeline already knows how to run. This is what a
//! bundler does, done at load time.
//!
//! The rewriting is driven by oxc's parse tree and applied by BYTE SPAN, not by
//! pattern-matching the text. Production bundles are minified, so `import` and
//! `export` appear inside strings, regex literals and identifiers constantly;
//! anything textual would corrupt them.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use oxc_allocator::Allocator;
use oxc_ast::ast as ox;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// One rewrite: replace `source[start..end]` with `text`.
struct Edit {
    start: u32,
    end: u32,
    text: String,
}

/// The result of linking a graph.
pub struct Linked {
    /// The emitted classic script.
    pub source: String,
    /// How many modules were fetched and linked.
    pub module_count: usize,
    /// The fetched `(url, raw_source)` for each linked module (for DevTools).
    pub loaded_modules: Vec<(String, String)>,
    /// Modules that could not be fetched or parsed, with the reason. These are
    /// reported rather than swallowed: a missing chunk means some route of the
    /// app will fail at runtime, and silently emitting a graph with a hole in
    /// it turns that into an inscrutable "undefined is not a function".
    pub failures: Vec<(String, String)>,
}

/// Cache key for rewritten ES modules: `(url, eager_dynamic, import_map_fp, source_hash, source_len)`.
type RewriteCacheKey = (String, bool, u64, u64, usize);
const MAX_REWRITE_CACHE: usize = 512;

static REWRITE_CACHE: LazyLock<Mutex<HashMap<RewriteCacheKey, Rewritten>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Drop all cached rewritten ES modules (called when the user clears browser cache).
pub fn clear_cache() {
    if let Ok(mut cache) = REWRITE_CACHE.lock() {
        cache.clear();
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// A page's `<script type="importmap">`: what `import "three"` means.
///
/// Only `imports` is honoured (not `scopes`): an exact key, or else the
/// longest key ending in `/` that prefixes the specifier. A key or specifier
/// that looks like a URL (`/`, `./`, `../`, a scheme) is resolved first, as
/// the spec says, so a map can also redirect URLs.
#[derive(Default, Clone)]
pub struct ImportMap {
    /// Keys and the absolute URLs they map to, in order: the first map to
    /// name a key keeps it.
    imports: Vec<(String, String)>,
}

fn url_like(spec: &str) -> bool {
    spec.starts_with('/')
        || spec.starts_with("./")
        || spec.starts_with("../")
        || spec.split_once(':').is_some_and(|(scheme, _)| {
            scheme.len() > 1 && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        })
}

impl ImportMap {
    /// Add one map's entries (its JSON), resolving against the page.
    pub fn add(&mut self, json: &str, page_url: &str, resolve: &dyn Fn(&str, &str) -> String) {
        let Ok(map) = serde_json::from_str::<serde_json::Value>(json) else {
            eprintln!("[ESM] import map is not JSON");
            return;
        };
        let Some(imports) = map.get("imports").and_then(|i| i.as_object()) else {
            return;
        };
        for (key, target) in imports {
            let Some(target) = target.as_str() else { continue };
            let key = if url_like(key) { resolve(page_url, key) } else { key.clone() };
            if !self.imports.iter().any(|(k, _)| *k == key) {
                self.imports.push((key, resolve(page_url, target)));
            }
        }
    }

    /// `spec`, imported from `base`, as the map rewrites it; `None` when no
    /// entry applies.
    pub fn resolve(&self, base: &str, spec: &str, resolve: &dyn Fn(&str, &str) -> String) -> Option<String> {
        if self.imports.is_empty() {
            return None;
        }
        let key = if url_like(spec) { resolve(base, spec) } else { spec.to_string() };
        if let Some((_, target)) = self.imports.iter().find(|(k, _)| *k == key) {
            return Some(target.clone());
        }
        self.imports
            .iter()
            .filter(|(k, _)| k.ends_with('/') && key.starts_with(k.as_str()))
            .max_by_key(|(k, _)| k.len())
            .map(|(k, target)| format!("{target}{}", &key[k.len()..]))
    }

    /// `spec`, imported from `base`, as a browser resolves it: through the
    /// map, else as a URL -- or, a bare name the map does not have
    /// (`import("esbuild")`), as it is, which is no module's URL: importing
    /// it fails, as it does in a browser, rather than fetching whatever the
    /// server has beside the importer.
    pub fn resolve_specifier(&self, base: &str, spec: &str, resolve: &dyn Fn(&str, &str) -> String) -> String {
        self.resolve(base, spec, resolve)
            .unwrap_or_else(|| if url_like(spec) { resolve(base, spec) } else { spec.to_string() })
    }

    /// The map as a JS object literal, for `import()` with a computed
    /// specifier, which resolves at runtime (`__zippResolveModuleUrl`).
    pub fn to_js(&self) -> String {
        let entries: serde_json::Map<String, serde_json::Value> =
            self.imports.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
        serde_json::Value::Object(entries).to_string()
    }

    /// Stable fingerprint of this map's entries for module rewrite caching.
    pub fn fingerprint(&self) -> u64 {
        if self.imports.is_empty() {
            return 0;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.imports.hash(&mut h);
        h.finish()
    }
}

/// Find dynamic `import(...)` and `import.meta` anywhere in the tree.
///
/// Both are rewritten because neither can survive into a classic script: the VM
/// rejects `import.meta` outright ("import.meta is only valid in modules"), and
/// a dynamic `import()` would reach the VM's filesystem loader and look for a
/// URL on disk.
///
/// It also notes an `await` outside any function: such a module's body has to
/// become an async function, where a plain one would not compile.
struct SetValueHoist {
    stmt_start: u32,
    stmt_end: u32,
    recv_start: u32,
    recv_end: u32,
    arg0: (u32, u32),
    arg1: (u32, u32),
    arg2: (u32, u32),
}

#[derive(Default)]
struct DynamicScan {
    dyn_imports: Vec<(u32, u32, u32, u32)>, // expr start/end, argument start/end
    meta_spans: Vec<(u32, u32)>,
    /// Three.js `StructuredUniform.setValue`: `u.setValue(gl, value[u.id], textures)`
    /// triggers a register-clobber bug in zipp-vm's JIT when the recursive method
    /// call evaluates `value[u.id]` inline in the argument list while `u` is also
    /// the receiver. Record the spans so we can hoist `value[u.id]` into a
    /// block-scoped local cleanly.
    set_value_hoists: Vec<SetValueHoist>,
    function_depth: u32,
    top_level_await: bool,
}

impl<'a> Visit<'a> for DynamicScan {
    fn visit_function_body(&mut self, it: &ox::FunctionBody<'a>) {
        self.function_depth += 1;
        oxc_ast_visit::walk::walk_function_body(self, it);
        self.function_depth -= 1;
    }

    fn visit_await_expression(&mut self, it: &ox::AwaitExpression<'a>) {
        self.top_level_await |= self.function_depth == 0;
        oxc_ast_visit::walk::walk_await_expression(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ox::ForOfStatement<'a>) {
        self.top_level_await |= it.r#await && self.function_depth == 0;
        oxc_ast_visit::walk::walk_for_of_statement(self, it);
    }

    fn visit_import_expression(&mut self, it: &ox::ImportExpression<'a>) {
        let span = it.span();
        let arg = it.source.span();
        self.dyn_imports.push((span.start, span.end, arg.start, arg.end));
        oxc_ast_visit::walk::walk_import_expression(self, it);
    }

    fn visit_meta_property(&mut self, it: &ox::MetaProperty<'a>) {
        if it.meta.name == "import" {
            let span = it.span();
            self.meta_spans.push((span.start, span.end));
        }
        oxc_ast_visit::walk::walk_meta_property(self, it);
    }

    fn visit_expression_statement(&mut self, it: &ox::ExpressionStatement<'a>) {
        if let ox::Expression::CallExpression(call) = &it.expression
            && call.arguments.len() == 3
            && let ox::Expression::StaticMemberExpression(callee) = &call.callee
            && callee.property.name == "setValue"
            && let ox::Expression::Identifier(recv) = &callee.object
            && let ox::Argument::ComputedMemberExpression(arg1) = &call.arguments[1]
            && let ox::Expression::StaticMemberExpression(idx_mem) = &arg1.expression
            && idx_mem.property.name == "id"
            && let ox::Expression::Identifier(idx_obj) = &idx_mem.object
            && idx_obj.name == recv.name
        {
            let stmt_span = it.span();
            let recv_span = recv.span();
            let a0 = call.arguments[0].span();
            let a1 = call.arguments[1].span();
            let a2 = call.arguments[2].span();
            self.set_value_hoists.push(SetValueHoist {
                stmt_start: stmt_span.start,
                stmt_end: stmt_span.end,
                recv_start: recv_span.start,
                recv_end: recv_span.end,
                arg0: (a0.start, a0.end),
                arg1: (a1.start, a1.end),
                arg2: (a2.start, a2.end),
            });
        }
        oxc_ast_visit::walk::walk_expression_statement(self, it);
    }
}

/// A module's static dependencies and the edits that turn it into a plain body.
#[derive(Clone)]
struct Rewritten {
    body: String,
    /// Specifiers this module imports, already resolved to absolute URLs.
    deps: Vec<String>,
    /// `__exp(...)` calls, emitted BEFORE the body.
    ///
    /// Export bindings are hoisted in real ESM, and the getters here are lazy
    /// (`() => local`), so hoisting them keeps cycles working: a module that
    /// re-enters a partially-initialised dependency still sees the binding, and
    /// only throws if it actually reads one still in its TDZ â€” which is exactly
    /// what a real engine does.
    export_calls: Vec<String>,
    /// The body awaits at top level, so it runs as an async function.
    top_level_await: bool,
}

fn js_string(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
    )
}

/// Rewrite one module's source into a body suitable for the registry wrapper.
///
/// When `eager_dynamic` is `true`, literal dynamic `import('./chunk.js')`
/// specifiers are added to `deps` so they are linked up front. When `false`,
/// they are rewritten to `__dyn("...")` so `__zippImportModule` fetches and
/// links them on demand via `__zippHostCall('esm_link', ...)`.
fn rewrite_module(
    url: &str,
    source: &str,
    eager_dynamic: bool,
    resolve: &dyn Fn(&str, &str) -> String,
) -> Result<Rewritten, String> {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::mjs()).parse();
    if !ret.errors.is_empty() {
        return Err(format!("{}", ret.errors[0]));
    }
    let program = &ret.program;

    let mut edits: Vec<Edit> = Vec::new();
    let mut deps: Vec<String> = Vec::new();
    let mut export_calls: Vec<String> = Vec::new();
    let add_dep = |spec: &str, deps: &mut Vec<String>| -> String {
        let abs = resolve(url, spec);
        if !deps.contains(&abs) {
            deps.push(abs.clone());
        }
        abs
    };

    // Static import/export declarations are only ever top-level.
    for stmt in &program.body {
        match stmt {
            ox::Statement::ImportDeclaration(d) => {
                let span = d.span();
                let abs = add_dep(&d.source.value, &mut deps);
                let mut out = String::new();
                let mut named: Vec<String> = Vec::new();
                let mut tail = String::new();
                if let Some(specs) = &d.specifiers {
                    for s in specs {
                        match s {
                            ox::ImportDeclarationSpecifier::ImportDefaultSpecifier(x) => {
                                named.push(format!("default:{}", x.local.name));
                            }
                            ox::ImportDeclarationSpecifier::ImportNamespaceSpecifier(x) => {
                                tail.push_str(&format!(
                                    "const {}=__imp({});",
                                    x.local.name,
                                    js_string(&abs)
                                ));
                            }
                            ox::ImportDeclarationSpecifier::ImportSpecifier(x) => {
                                let imported = match &x.imported {
                                    ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                                    ox::ModuleExportName::IdentifierReference(i) => {
                                        i.name.to_string()
                                    }
                                    ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                                };
                                named.push(format!("{}:{}", js_string(&imported), x.local.name));
                            }
                        }
                    }
                }
                if named.is_empty() {
                    // `import "./side-effect.js"` â€” still has to run.
                    out.push_str(&format!("__imp({});", js_string(&abs)));
                } else {
                    out.push_str(&format!(
                        "const{{{}}}=__imp({});",
                        named.join(","),
                        js_string(&abs)
                    ));
                }
                out.push_str(&tail);
                edits.push(Edit { start: span.start, end: span.end, text: out });
            }
            ox::Statement::ExportNamedDeclaration(d) => {
                let span = d.span();
                let out = String::new();
                if let Some(src) = &d.source {
                    // `export {a as b} from "./m"` â€” no local binding is made.
                    let abs = add_dep(&src.value, &mut deps);
                    for s in &d.specifiers {
                        let local = match &s.local {
                            ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                            ox::ModuleExportName::IdentifierReference(i) => i.name.to_string(),
                            ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                        };
                        let exported = match &s.exported {
                            ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                            ox::ModuleExportName::IdentifierReference(i) => i.name.to_string(),
                            ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                        };
                        export_calls.push(format!(
                            "__exp({},()=>__imp({})[{}]);",
                            js_string(&exported),
                            js_string(&abs),
                            js_string(&local)
                        ));
                    }
                } else if let Some(decl) = &d.declaration {
                    // `export const x = 1` / `export function f(){}` â€” keep the
                    // declaration where it is, drop the `export` keyword, record
                    // the names. Only the keyword is edited: the declaration
                    // may hold edits of its own (`import.meta`, `import()`).
                    let dspan = decl.span();
                    wrap(&mut edits, span.start, dspan.start, dspan.end, span.end, "", "");
                    for name in declared_names(decl) {
                        export_calls
                            .push(format!("__exp({},()=>{});", js_string(&name), name));
                    }
                    continue;
                } else {
                    for s in &d.specifiers {
                        let local = match &s.local {
                            ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                            ox::ModuleExportName::IdentifierReference(i) => i.name.to_string(),
                            ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                        };
                        let exported = match &s.exported {
                            ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                            ox::ModuleExportName::IdentifierReference(i) => i.name.to_string(),
                            ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                        };
                        export_calls.push(format!(
                            "__exp({},()=>{});",
                            js_string(&exported),
                            local
                        ));
                    }
                }
                edits.push(Edit { start: span.start, end: span.end, text: out });
            }
            ox::Statement::ExportDefaultDeclaration(d) => {
                // As above, the declaration or expression stays where it is and
                // only what surrounds it is edited.
                let span = d.span();
                let named = |id: Option<&str>, export_calls: &mut Vec<String>| match id {
                    Some(name) => {
                        export_calls.push(format!("__exp(\"default\",()=>{name});"));
                        ("", "")
                    }
                    None => {
                        export_calls.push("__exp(\"default\",()=>__zippDefault);".into());
                        ("const __zippDefault=", ";")
                    }
                };
                let (inner, (before, after)) = match &d.declaration {
                    ox::ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        (f.span(), named(f.id.as_ref().map(|i| i.name.as_str()), &mut export_calls))
                    }
                    ox::ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        (c.span(), named(c.id.as_ref().map(|i| i.name.as_str()), &mut export_calls))
                    }
                    other => {
                        export_calls.push("__exp(\"default\",()=>__zippDefault);".into());
                        (other.span(), ("const __zippDefault=(", ");"))
                    }
                };
                wrap(&mut edits, span.start, inner.start, inner.end, span.end, before, after);
            }
            ox::Statement::ExportAllDeclaration(d) => {
                let span = d.span();
                let abs = add_dep(&d.source.value, &mut deps);
                let out = match &d.exported {
                    Some(name) => {
                        let exported = match name {
                            ox::ModuleExportName::IdentifierName(i) => i.name.to_string(),
                            ox::ModuleExportName::IdentifierReference(i) => i.name.to_string(),
                            ox::ModuleExportName::StringLiteral(l) => l.value.to_string(),
                        };
                        export_calls.push(format!(
                            "__exp({},()=>__imp({}));",
                            js_string(&exported),
                            js_string(&abs)
                        ));
                        format!("__imp({});", js_string(&abs))
                    }
                    None => format!("__expAll(__imp({}));", js_string(&abs)),
                };
                edits.push(Edit { start: span.start, end: span.end, text: out });
            }
            _ => {}
        }
    }

    // Dynamic import + import.meta, anywhere in the tree.
    let mut scan = DynamicScan::default();
    scan.visit_program(program);
    let top_level_await = scan.top_level_await;
    for (start, end, arg_start, arg_end) in scan.dyn_imports {
        let arg_text = &source[arg_start as usize..arg_end as usize];
        // A literal specifier is resolved now, so the target is part of the
        // graph and already fetched. A computed one can only be resolved
        // against this module's URL at runtime.
        match syn_literal(arg_text) {
            Some(spec) => {
                let abs = resolve(url, &spec);
                // A bare name nothing maps (or a Node builtin like `node:fs`) is
                // not fetched: the import fails when it runs, which a page that
                // tries it is ready for. In lazy mode, literal dynamic imports
                // are deferred until `__dyn(abs)` executes at runtime.
                if eager_dynamic
                    && url_like(&abs)
                    && !abs.starts_with("node:")
                    && !deps.contains(&abs)
                {
                    deps.push(abs.clone());
                }
                edits.push(Edit { start, end, text: format!("__dyn({})", js_string(&abs)) });
            }
            None => {
                // Use `wrap` so inner `import.meta` or nested `import(...)`
                // edits inside `arg_start..arg_end` are preserved!
                wrap(&mut edits, start, arg_start, arg_end, end, &format!("__dynRel({},", js_string(url)), ")");
            }
        }
    }
    for (start, end) in scan.meta_spans {
        edits.push(Edit { start, end, text: "__meta".into() });
    }
    for h in scan.set_value_hoists {
        let recv = &source[h.recv_start as usize..h.recv_end as usize];
        let arg0 = &source[h.arg0.0 as usize..h.arg0.1 as usize];
        let arg1 = &source[h.arg1.0 as usize..h.arg1.1 as usize];
        let arg2 = &source[h.arg2.0 as usize..h.arg2.1 as usize];
        edits.push(Edit {
            start: h.stmt_start,
            end: h.stmt_end,
            text: format!("{{let __zv={arg1};{recv}.setValue({arg0},__zv,{arg2});}}"),
        });
    }

    // Apply edits in a single O(N) forward pass instead of O(M * N) `replace_range`.
    edits.sort_by_key(|e| (e.start, e.end));
    let mut body = String::with_capacity(source.len() + edits.iter().map(|e| e.text.len()).sum::<usize>());
    let mut cursor = 0usize;
    for e in edits {
        let start = e.start as usize;
        let end = e.end as usize;
        if start < cursor {
            continue;
        }
        body.push_str(&source[cursor..start]);
        body.push_str(&e.text);
        cursor = end;
    }
    body.push_str(&source[cursor..]);

    Ok(Rewritten { body, deps, export_calls, top_level_await })
}

/// Replace `outer_start..inner_start` with `before` and `inner_end..outer_end`
/// with `after`, leaving `inner_start..inner_end` -- and any edits inside it --
/// alone.
fn wrap(edits: &mut Vec<Edit>, outer_start: u32, inner_start: u32, inner_end: u32, outer_end: u32, before: &str, after: &str) {
    edits.push(Edit { start: outer_start, end: inner_start, text: before.into() });
    if inner_end < outer_end || !after.is_empty() {
        edits.push(Edit { start: inner_end, end: outer_end, text: after.into() });
    }
}

/// A quoted string literal's value, when the text is exactly that.
fn syn_literal(text: &str) -> Option<String> {
    let t = text.trim();
    let b = t.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"')
            || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
            || (b[0] == b'`' && b[b.len() - 1] == b'`'))
    {
        let inner = &t[1..t.len() - 1];
        // A template with a substitution is not a literal specifier.
        if b[0] == b'`' && inner.contains("${") {
            return None;
        }
        if inner.contains(['"', '\'', '`', '\\']) {
            return None;
        }
        return Some(inner.to_string());
    }
    None
}

/// Names bound by an exported declaration.
fn declared_names<'a>(decl: &ox::Declaration<'a>) -> Vec<String> {
    let mut out = Vec::new();
    match decl {
        ox::Declaration::VariableDeclaration(d) => {
            for dd in &d.declarations {
                collect_pattern_names(&dd.id, &mut out);
            }
        }
        ox::Declaration::FunctionDeclaration(f) => {
            if let Some(id) = &f.id {
                out.push(id.name.to_string());
            }
        }
        ox::Declaration::ClassDeclaration(c) => {
            if let Some(id) = &c.id {
                out.push(id.name.to_string());
            }
        }
        _ => {}
    }
    out
}

fn collect_pattern_names<'a>(kind: &ox::BindingPattern<'a>, out: &mut Vec<String>) {
    match kind {
        ox::BindingPattern::BindingIdentifier(i) => out.push(i.name.to_string()),
        ox::BindingPattern::ObjectPattern(o) => {
            for p in &o.properties {
                collect_pattern_names(&p.value, out);
            }
            if let Some(rest) = &o.rest {
                collect_pattern_names(&rest.argument, out);
            }
        }
        ox::BindingPattern::ArrayPattern(a) => {
            for e in a.elements.iter().flatten() {
                collect_pattern_names(e, out);
            }
            if let Some(rest) = &a.rest {
                collect_pattern_names(&rest.argument, out);
            }
        }
        ox::BindingPattern::AssignmentPattern(p) => {
            collect_pattern_names(&p.left, out)
        }
    }
}

/// Maximum parallel HTTP fetches per dependency wave when linking an ES module graph.
const MAX_PARALLEL_FETCHES: usize = 8;

/// Fetch and link the whole graph reachable from `entries` (including literal
/// dynamic `import(...)` chunks). Used by Web Workers and tests.
pub fn link_graph(
    entries: &[String],
    fetch: &(dyn Fn(&str) -> Result<String, String> + Sync),
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    link_impl(entries, &HashSet::new(), true, true, None, fetch, resolve)
}

/// Fetch and link only static dependencies reachable from `entries`, deferring
/// dynamic `import(...)` chunks to on-demand runtime linking (`esm_link`).
///
/// Modern Vite/Webpack SPAs often reference hundreds of code-split route chunks
/// via literal `import("./Route.js")`. Deferring them avoids downloading and
/// compiling megabytes of unused routes on initial page load.
pub fn link_graph_lazy(
    entries: &[String],
    import_map_fp: u64,
    fetch: &(dyn Fn(&str) -> Result<String, String> + Sync),
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    link_impl(entries, &HashSet::new(), true, false, Some(import_map_fp), fetch, resolve)
}

/// [`link_graph`], leaving out the modules in `linked` (the page has them),
/// and running the entries only if `run`.
#[allow(dead_code)]
pub fn link_more(
    entries: &[String],
    linked: &HashSet<String>,
    run: bool,
    fetch: &(dyn Fn(&str) -> Result<String, String> + Sync),
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    link_impl(entries, linked, run, true, None, fetch, resolve)
}

/// Lazy counterpart to [`link_more`]: links only static dependencies of
/// `entries` (excluding `linked`), deferring further dynamic imports and using
/// the in-memory module rewrite cache keyed by `import_map_fp`.
pub fn link_more_lazy(
    entries: &[String],
    linked: &HashSet<String>,
    run: bool,
    import_map_fp: u64,
    fetch: &(dyn Fn(&str) -> Result<String, String> + Sync),
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    link_impl(entries, linked, run, false, Some(import_map_fp), fetch, resolve)
}

fn link_impl(
    entries: &[String],
    linked: &HashSet<String>,
    run: bool,
    eager_dynamic: bool,
    cache_map_fp: Option<u64>,
    fetch: &(dyn Fn(&str) -> Result<String, String> + Sync),
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    let mut wave: Vec<String> = entries.iter().filter(|e| !linked.contains(*e)).cloned().collect();
    let mut seen: HashSet<String> = entries.iter().chain(linked.iter()).cloned().collect();
    let mut modules: Vec<(String, Rewritten)> = Vec::new();
    let mut loaded_modules: Vec<(String, String)> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    let js_from = std::env::var("ZIPP_JS_FROM").ok();
    let js_to = std::env::var("ZIPP_JS_TO").ok();

    while !wave.is_empty() {
        let mut to_fetch: Vec<String> = Vec::new();
        for url in std::mem::take(&mut wave) {
            if !url_like(&url) {
                failures.push((url, "a bare specifier, which no import map names".into()));
            } else {
                to_fetch.push(url);
            }
        }
        if to_fetch.is_empty() {
            break;
        }

        // Fetch this BFS wave in parallel (capped at MAX_PARALLEL_FETCHES) so
        // multi-module SoftN apps and Vite SPAs reuse keep-alive connections
        // instead of stalling on 50+ sequential round-trips.
        let fetched: Vec<(String, Result<String, String>)> = if to_fetch.len() <= 1 {
            to_fetch.into_iter().map(|u| { let r = fetch(&u); (u, r) }).collect()
        } else {
            let workers = to_fetch.len().min(MAX_PARALLEL_FETCHES);
            let next_idx = std::sync::atomic::AtomicUsize::new(0);
            let results: Vec<std::sync::Mutex<Option<Result<String, String>>>> =
                (0..to_fetch.len()).map(|_| std::sync::Mutex::new(None)).collect();
            std::thread::scope(|s| {
                for _ in 0..workers {
                    s.spawn(|| {
                        loop {
                            let i = next_idx.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if i >= to_fetch.len() {
                                break;
                            }
                            let res = fetch(&to_fetch[i]);
                            *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
                        }
                    });
                }
            });
            to_fetch
                .into_iter()
                .zip(results)
                .map(|(u, slot)| {
                    let res = slot
                        .into_inner()
                        .unwrap_or_else(|e| e.into_inner())
                        .unwrap_or_else(|| Err("worker did not complete".into()));
                    (u, res)
                })
                .collect()
        };

        for (url, res) in fetched {
            let source = match res {
                Ok(s) => s,
                Err(e) => {
                    failures.push((url, format!("fetch failed: {e}")));
                    continue;
                }
            };
            // Same `ZIPP_JS_FROM`/`ZIPP_JS_TO` rewrite the classic-script path has.
            let source = match (&js_from, &js_to) {
                (Some(from), Some(to)) if source.contains(from.as_str()) => {
                    eprintln!("[ESM] substituted page source in {url}");
                    source.replace(from.as_str(), to.as_str())
                }
                _ => source,
            };
            let cache_key = cache_map_fp.map(|fp| {
                (url.clone(), eager_dynamic, fp, hash_bytes(source.as_bytes()), source.len())
            });
            let cached = cache_key.as_ref().and_then(|k| {
                REWRITE_CACHE.lock().ok().and_then(|c| c.get(k).cloned())
            });
            let rw_res = match cached {
                Some(rw) => Ok(rw),
                None => {
                    let r = rewrite_module(&url, &source, eager_dynamic, resolve);
                    if let (Ok(rw), Some(key)) = (&r, cache_key)
                        && let Ok(mut cache) = REWRITE_CACHE.lock()
                    {
                        if cache.len() >= MAX_REWRITE_CACHE {
                            cache.clear();
                        }
                        cache.insert(key, rw.clone());
                    }
                    r
                }
            };
            match rw_res {
                Ok(rw) => {
                    for d in &rw.deps {
                        if seen.insert(d.clone()) {
                            wave.push(d.clone());
                        }
                    }
                    loaded_modules.push((url.clone(), source));
                    modules.push((url, rw));
                }
                Err(e) => failures.push((url, format!("parse failed: {e}"))),
            }
        }
    }

    let mut out = String::new();
    out.push_str("\n;(function(){\n");
    for (url, rw) in &modules {
        out.push_str(&format!(
            "__zippDefine({},{}function(__imp,__exp,__expAll,__dyn,__dynRel,__meta){{\n",
            js_string(url),
            if rw.top_level_await { "async " } else { "" }
        ));
        for call in &rw.export_calls {
            out.push_str(call);
            out.push('\n');
        }
        out.push_str(&rw.body);
        out.push_str("\n});\n");
    }
    if run {
        for e in entries {
            out.push_str(&format!("__zippRunModule({});\n", js_string(e)));
        }
    }
    out.push_str("})();\n");

    Linked { source: out, module_count: modules.len(), loaded_modules, failures }
}

/// Registry + instantiation, injected once alongside the linked graph.
pub const MODULE_RUNTIME_JS: &str = r#"
// ES module registry. `__zippDefine` records a module; `__zippRunModule`
// instantiates one. Namespaces are created BEFORE a module body runs and
// exports are defined as getters at the top of it, so an import cycle sees the
// binding rather than an empty object â€” the same shape real ESM has.
var __zippModuleDefs = {};
var __zippModuleNs = {};
var __zippModulePromises = {};
function __zippDefine(url, fn) { __zippModuleDefs[url] = fn; }
// The page's import map, if it has one (`ImportMap::resolve` at runtime).
function __zippResolveModuleUrl(base, spec) {
    var map = typeof __zippImportMap !== 'undefined' ? __zippImportMap : {};
    var key = spec;
    if (/^(\/|\.\.?\/|[a-z][a-z0-9+.-]+:)/i.test(spec)) {
        try { key = String(new URL(spec, base)); } catch (e) {}
    }
    if (Object.prototype.hasOwnProperty.call(map, key)) return map[key];
    var best = '';
    for (var k in map) {
        if (k.charAt(k.length - 1) === '/' && key.indexOf(k) === 0 && k.length > best.length) best = k;
    }
    if (best) return map[best] + key.slice(best.length);
    // A bare name the map does not have stays one, and fails to import.
    if (key === spec) return spec;
    try { return String(new URL(spec, base)); } catch (e) { return spec; }
}
// A module for `import()`: linked up front, or -- a chunk whose URL the page
// computes as it runs, which no reading of the source can find -- fetched and
// linked now, with whatever it imports that is not here yet. Before, such an
// import resolved to an empty namespace, and a bundler's loader reported the
// chunk as failed.
function __zippImportModule(u) {
    if (!/^[a-z][a-z0-9+.-]+:/i.test(u)) {
        return Promise.reject(new TypeError('Failed to resolve module specifier "' + u +
            '". Relative references must start with either "/", "./", or "../".'));
    }
    if (!Object.prototype.hasOwnProperty.call(__zippModuleDefs, u)) {
        var src = '';
        try { src = __zippHostCall('esm_link', u, JSON.stringify(Object.keys(__zippModuleDefs))); } catch (e) {}
        if (src) {
            try { (0, eval)(src); } catch (e) { env.log('error', 'module ' + u + ' could not be linked: ' + e); }
        }
        if (!Object.prototype.hasOwnProperty.call(__zippModuleDefs, u)) {
            return Promise.reject(new TypeError('Failed to fetch dynamically imported module: ' + u));
        }
    }
    var ns = __zippRunModule(u);
    if (Object.prototype.hasOwnProperty.call(__zippModulePromises, u)) {
        return __zippModulePromises[u];
    }
    return Promise.resolve(ns);
}
function __zippRunModule(url) {
    if (Object.prototype.hasOwnProperty.call(__zippModuleNs, url)) {
        return __zippModuleNs[url];
    }
    var fn = __zippModuleDefs[url];
    if (!fn) {
        env.log('error', 'module not linked: ' + url);
        return {};
    }
    var ns = {};
    // Registered before the body runs: a cycle back into this module gets this
    // same object rather than recursing forever.
    __zippModuleNs[url] = ns;
    var imp = function (u) { return __zippRunModule(u); };
    var exp = function (name, getter) {
        Object.defineProperty(ns, name, { get: getter, enumerable: true, configurable: true });
    };
    var expAll = function (other) {
        for (var k in other) {
            if (k !== 'default' && !Object.prototype.hasOwnProperty.call(ns, k)) {
                (function (key) {
                    Object.defineProperty(ns, key, {
                        get: function () { return other[key]; },
                        enumerable: true, configurable: true
                    });
                })(k);
            }
        }
    };
    var dbg = typeof __zippEsmDebug !== 'undefined' && __zippEsmDebug;
    var dyn = function (u) {
        u = __zippResolveModuleUrl(url, u);
        if (dbg) env.log('info', 'esm dynamic ' + u);
        return __zippImportModule(u);
    };
    var dynRel = function (base, spec) {
        var u = __zippResolveModuleUrl(base, spec);
        if (dbg) env.log('info', 'esm dynamic(rel) ' + spec + ' -> ' + u);
        return __zippImportModule(u);
    };
    var meta = { url: url, env: { MODE: 'production', PROD: true, DEV: false, BASE_URL: '/' } };
    if (typeof __zippEsmDebug !== 'undefined' && __zippEsmDebug) {
        env.log('info', 'esm run ' + url);
    }
    var threw = function (e) {
        env.log('error', 'module threw: ' + url + ': ' + ((e && (e.stack || e.message)) || e));
    };
    try {
        // A module that awaits at top level is an async function: its
        // exports are there once it settles, and a rejection is its throw.
        var done = fn(imp, exp, expAll, dyn, dynRel, meta);
        if (done && typeof done.then === 'function') {
            __zippModulePromises[url] = done.then(function () { return ns; }, function (e) { threw(e); throw e; });
        }
    } catch (e) {
        threw(e);
    }
    if (typeof __zippEsmDebug !== 'undefined' && __zippEsmDebug) {
        env.log('info', 'esm done ' + url);
    }
    return ns;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn res(base: &str, spec: &str) -> String {
        if let Some(rest) = spec.strip_prefix("./") {
            let dir = &base[..base.rfind('/').unwrap_or(0)];
            format!("{dir}/{rest}")
        } else {
            spec.to_string()
        }
    }

    fn rw(src: &str) -> Rewritten {
        rewrite_module("http://x/a.js", src, true, &res).expect("parses")
    }

    #[test]
    fn import_map_resolves_bare_and_prefixed_specifiers() {
        let mut map = ImportMap::default();
        map.add(
            r#"{"imports": {"three": "./lib/three.module.js", "three/addons/": "./lib/jsm/",
                "./old.js": "./new.js"}}"#,
            "http://x/page.html",
            &res,
        );
        let r = |spec: &str| map.resolve("http://x/page.html", spec, &res);
        assert_eq!(r("three").as_deref(), Some("http://x/lib/three.module.js"));
        assert_eq!(r("three/addons/controls/Orbit.js").as_deref(), Some("http://x/lib/jsm/controls/Orbit.js"));
        assert_eq!(r("./old.js").as_deref(), Some("http://x/new.js"));
        assert_eq!(r("./other.js"), None);
        assert_eq!(r("react"), None);
        assert!(map.to_js().contains("\"three/addons/\":\"http://x/lib/jsm/\""));
    }

    #[test]
    fn bare_specifier_the_map_lacks_stays_bare() {
        let mut map = ImportMap::default();
        map.add(r#"{"imports": {"three": "./three.js"}}"#, "http://x/page.html", &res);
        let r = |spec: &str| map.resolve_specifier("http://x/a.js", spec, &res);
        assert_eq!(r("three"), "http://x/three.js");
        assert_eq!(r("./b.js"), "http://x/b.js");
        assert_eq!(r("esbuild"), "esbuild");
        // Nor is it fetched for `import()`, which fails when it runs.
        let resolve = |base: &str, spec: &str| map.resolve_specifier(base, spec, &res);
        let rw = rewrite_module("http://x/a.js", "try { await import('esbuild'); } catch {}", true, &resolve).expect("parses");
        assert!(rw.deps.is_empty(), "{:?}", rw.deps);
        assert!(rw.body.contains(r#"__dyn("esbuild")"#), "{}", rw.body);
    }

    #[test]
    fn named_import_becomes_destructure() {
        let r = rw("import {a as b, c} from './d.js'; b();");
        assert!(r.body.contains(r#"const{"a":b,"c":c}=__imp("http://x/d.js");"#), "{}", r.body);
        assert_eq!(r.deps, vec!["http://x/d.js"]);
    }

    #[test]
    fn default_and_namespace_imports() {
        let r = rw("import d from './d.js'; import * as n from './e.js';");
        assert!(r.body.contains(r#"const{default:d}=__imp("http://x/d.js");"#), "{}", r.body);
        assert!(r.body.contains(r#"const n=__imp("http://x/e.js");"#), "{}", r.body);
    }

    #[test]
    fn top_level_await_is_noticed() {
        assert!(rw("const a = await f();").top_level_await);
        assert!(rw("for await (const x of xs) {}").top_level_await);
        assert!(!rw("async function g() { await f(); } const h = async () => await f();").top_level_await);
    }

    #[test]
    fn bare_import_still_runs_for_side_effects() {
        let r = rw("import './side.js';");
        assert!(r.body.contains(r#"__imp("http://x/side.js");"#), "{}", r.body);
    }

    #[test]
    fn exports_are_hoisted_getters() {
        let r = rw("export const q = 1; export function f(){}");
        assert!(r.export_calls.iter().any(|c| c == r#"__exp("q",()=>q);"#), "{:?}", r.export_calls);
        assert!(r.export_calls.iter().any(|c| c == r#"__exp("f",()=>f);"#), "{:?}", r.export_calls);
        // The declarations themselves survive, without the `export` keyword.
        assert!(r.body.contains("const q = 1;"), "{}", r.body);
        assert!(r.body.contains("function f(){}"), "{}", r.body);
    }

    #[test]
    fn default_export_of_expression_is_bound() {
        let r = rw("export default 1 + 2;");
        assert!(r.body.contains("const __zippDefault=(1 + 2);"), "{}", r.body);
        assert!(r.export_calls.iter().any(|c| c.contains("default")));
    }

    #[test]
    fn re_export_pulls_in_the_dependency() {
        let r = rw("export {z} from './re.js';");
        assert_eq!(r.deps, vec!["http://x/re.js"]);
        assert!(
            r.export_calls.iter().any(|c| c.contains("__imp(\"http://x/re.js\")")),
            "{:?}",
            r.export_calls
        );
    }

    #[test]
    fn literal_dynamic_import_joins_the_graph() {
        let r = rw("const p = import('./lazy.js');");
        assert!(r.body.contains(r#"__dyn("http://x/lazy.js")"#), "{}", r.body);
        assert!(r.deps.contains(&"http://x/lazy.js".to_string()));
    }

    #[test]
    fn computed_dynamic_import_resolves_at_runtime() {
        let r = rw("const p = import(name);");
        assert!(r.body.contains(r#"__dynRel("http://x/a.js",name)"#), "{}", r.body);
    }

    /// An export's own edit used to copy the declaration's text over the whole
    /// statement, overlapping these: the module came out wrong, or the linker
    /// panicked slicing mid-character.
    #[test]
    fn edits_inside_exported_declarations_survive() {
        let r = rw("export class A { static f(u = new URL('k.wasm', import.meta.url)) { return 'â€”' + u; } }");
        assert!(r.body.contains("class A { static f(u = new URL('k.wasm', __meta.url))"), "{}", r.body);
        assert!(!r.body.contains("export"), "{}", r.body);
        let r = rw("export default function () { return import('./lazy.js'); }");
        assert!(r.body.contains(r#"const __zippDefault=function () { return __dyn("http://x/lazy.js"); };"#), "{}", r.body);
        let r = rw("export default { u: import.meta.url };");
        assert!(r.body.contains("const __zippDefault=({ u: __meta.url });"), "{}", r.body);
    }

    #[test]
    fn import_meta_is_replaced() {
        let r = rw("const u = import.meta.url;");
        assert!(r.body.contains("__meta.url"), "{}", r.body);
    }

    #[test]
    fn hoists_threejs_structured_uniform_set_value_lookup() {
        let src = "class S{setValue(e,t,n){let r=this.seq;for(let i=0,a=r.length;i!==a;++i){let a=r[i];a.setValue(e,t[a.id],n)}}}";
        let r = rw(src);
        assert!(
            r.body.contains("{let __zv=t[a.id];a.setValue(e,__zv,n);}"),
            "{}",
            r.body
        );
    }

    /// The whole point of parsing rather than pattern-matching: these words
    /// appear constantly in minified bundles inside strings and identifiers.
    #[test]
    fn text_that_merely_looks_like_module_syntax_is_untouched() {
        let src = r#"const s = "import x from 'y'"; const t = 'export default 1';
                     const o = {import_meta: 1}; function important(){ return 1 }"#;
        let r = rw(src);
        assert_eq!(r.body, src, "no edit should have been made");
        assert!(r.deps.is_empty());
        assert!(r.export_calls.is_empty());
    }

    #[test]
    fn lazy_graph_defers_dynamic_imports_to_runtime() {
        let fetch = |url: &str| -> Result<String, String> {
            match url {
                "http://x/entry.js" => Ok(
                    "import { a } from './static.js'; export const load = () => import('./lazy.js'); console.log(a);"
                        .to_string(),
                ),
                "http://x/static.js" => Ok("export const a = 42;".to_string()),
                "http://x/lazy.js" => Ok("export const b = 99;".to_string()),
                other => Err(format!("unexpected fetch {other}")),
            }
        };
        let linked = link_graph_lazy(&["http://x/entry.js".to_string()], 0, &fetch, &res);
        assert_eq!(linked.module_count, 2, "only entry + static should be linked");
        assert_eq!(linked.loaded_modules.len(), 2);
        assert!(linked.failures.is_empty(), "{:?}", linked.failures);
        assert!(linked.source.contains(r#"__dyn("http://x/lazy.js")"#));
        assert!(!linked.source.contains("export const b = 99") && !linked.source.contains("const b = 99"));

        // Now link the lazy chunk on demand, excluding already linked modules.
        let already: HashSet<String> =
            ["http://x/entry.js".to_string(), "http://x/static.js".to_string()].into_iter().collect();
        let more = link_more_lazy(&["http://x/lazy.js".to_string()], &already, false, 0, &fetch, &res);
        assert_eq!(more.module_count, 1);
        assert!(more.source.contains("const b = 99;"));
    }
}

