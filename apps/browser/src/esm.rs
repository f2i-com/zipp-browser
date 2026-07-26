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

use std::collections::HashSet;

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
    /// Modules that could not be fetched or parsed, with the reason. These are
    /// reported rather than swallowed: a missing chunk means some route of the
    /// app will fail at runtime, and silently emitting a graph with a hole in
    /// it turns that into an inscrutable "undefined is not a function".
    pub failures: Vec<(String, String)>,
}

/// Find dynamic `import(...)` and `import.meta` anywhere in the tree.
///
/// Both are rewritten because neither can survive into a classic script: the VM
/// rejects `import.meta` outright ("import.meta is only valid in modules"), and
/// a dynamic `import()` would reach the VM's filesystem loader and look for a
/// URL on disk.
#[derive(Default)]
struct DynamicScan {
    dyn_imports: Vec<(u32, u32, u32, u32)>, // expr start/end, argument start/end
    meta_spans: Vec<(u32, u32)>,
}

impl<'a> Visit<'a> for DynamicScan {
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
}

/// A module's static dependencies and the edits that turn it into a plain body.
struct Rewritten {
    body: String,
    /// Specifiers this module imports, already resolved to absolute URLs.
    deps: Vec<String>,
    /// `__exp(...)` calls, emitted BEFORE the body.
    ///
    /// Export bindings are hoisted in real ESM, and the getters here are lazy
    /// (`() => local`), so hoisting them keeps cycles working: a module that
    /// re-enters a partially-initialised dependency still sees the binding, and
    /// only throws if it actually reads one still in its TDZ — which is exactly
    /// what a real engine does.
    export_calls: Vec<String>,
}

fn js_string(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
    )
}

/// Rewrite one module's source into a body suitable for the registry wrapper.
fn rewrite_module(
    url: &str,
    source: &str,
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
                    // `import "./side-effect.js"` — still has to run.
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
                let mut out = String::new();
                if let Some(src) = &d.source {
                    // `export {a as b} from "./m"` — no local binding is made.
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
                    // `export const x = 1` / `export function f(){}` — keep the
                    // declaration, drop the `export` keyword, record the names.
                    let dspan = decl.span();
                    out.push_str(&source[dspan.start as usize..dspan.end as usize]);
                    for name in declared_names(decl) {
                        export_calls
                            .push(format!("__exp({},()=>{});", js_string(&name), name));
                    }
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
                let span = d.span();
                let out = match &d.declaration {
                    ox::ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        let fspan = f.span();
                        let text = &source[fspan.start as usize..fspan.end as usize];
                        match &f.id {
                            Some(id) => {
                                export_calls
                                    .push(format!("__exp(\"default\",()=>{});", id.name));
                                text.to_string()
                            }
                            None => {
                                export_calls
                                    .push("__exp(\"default\",()=>__zippDefault);".into());
                                format!("const __zippDefault={text};")
                            }
                        }
                    }
                    ox::ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        let cspan = c.span();
                        let text = &source[cspan.start as usize..cspan.end as usize];
                        match &c.id {
                            Some(id) => {
                                export_calls
                                    .push(format!("__exp(\"default\",()=>{});", id.name));
                                text.to_string()
                            }
                            None => {
                                export_calls
                                    .push("__exp(\"default\",()=>__zippDefault);".into());
                                format!("const __zippDefault={text};")
                            }
                        }
                    }
                    other => {
                        let ospan = other.span();
                        let text = &source[ospan.start as usize..ospan.end as usize];
                        export_calls.push("__exp(\"default\",()=>__zippDefault);".into());
                        format!("const __zippDefault=({text});")
                    }
                };
                edits.push(Edit { start: span.start, end: span.end, text: out });
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
                        String::new()
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
    for (start, end, arg_start, arg_end) in scan.dyn_imports {
        let arg_text = &source[arg_start as usize..arg_end as usize];
        // A literal specifier is resolved now, so the target is part of the
        // graph and already fetched. A computed one can only be resolved
        // against this module's URL at runtime.
        let replacement = match syn_literal(arg_text) {
            Some(spec) => {
                let abs = resolve(url, &spec);
                deps.push(abs.clone());
                format!("__dyn({})", js_string(&abs))
            }
            None => format!("__dynRel({},{})", js_string(url), arg_text),
        };
        edits.push(Edit { start, end, text: replacement });
    }
    for (start, end) in scan.meta_spans {
        edits.push(Edit { start, end, text: "__meta".into() });
    }

    // Apply edits back-to-front so earlier offsets stay valid.
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut body = source.to_string();
    for e in edits {
        body.replace_range(e.start as usize..e.end as usize, &e.text);
    }

    Ok(Rewritten { body, deps, export_calls })
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
                collect_pattern_names(&e, out);
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

/// Fetch and link the whole graph reachable from `entries`.
///
/// Dynamic-import targets with literal specifiers are pulled in too, so a
/// lazily-routed app has every chunk present before it runs. Our host HTTP is
/// synchronous and there is no way to fetch from inside the VM mid-execution,
/// so a chunk that is not linked up front could never be loaded at all.
pub fn link_graph(
    entries: &[String],
    fetch: &dyn Fn(&str) -> Result<String, String>,
    resolve: &dyn Fn(&str, &str) -> String,
) -> Linked {
    let mut queue: Vec<String> = entries.to_vec();
    let mut seen: HashSet<String> = entries.iter().cloned().collect();
    let mut modules: Vec<(String, Rewritten)> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    while let Some(url) = queue.pop() {
        let source = match fetch(&url) {
            Ok(s) => s,
            Err(e) => {
                failures.push((url.clone(), format!("fetch failed: {e}")));
                continue;
            }
        };
        match rewrite_module(&url, &source, resolve) {
            Ok(rw) => {
                for d in &rw.deps {
                    if seen.insert(d.clone()) {
                        queue.push(d.clone());
                    }
                }
                modules.push((url, rw));
            }
            Err(e) => failures.push((url.clone(), format!("parse failed: {e}"))),
        }
    }

    let mut out = String::new();
    out.push_str("\n;(function(){\n");
    for (url, rw) in &modules {
        out.push_str(&format!(
            "__zippDefine({},function(__imp,__exp,__expAll,__dyn,__dynRel,__meta){{\n",
            js_string(url)
        ));
        for call in &rw.export_calls {
            out.push_str(call);
            out.push('\n');
        }
        out.push_str(&rw.body);
        out.push_str("\n});\n");
    }
    for e in entries {
        out.push_str(&format!("__zippRunModule({});\n", js_string(e)));
    }
    out.push_str("})();\n");

    Linked { source: out, module_count: modules.len(), failures }
}

/// Registry + instantiation, injected once alongside the linked graph.
pub const MODULE_RUNTIME_JS: &str = r#"
// ES module registry. `__zippDefine` records a module; `__zippRunModule`
// instantiates one. Namespaces are created BEFORE a module body runs and
// exports are defined as getters at the top of it, so an import cycle sees the
// binding rather than an empty object — the same shape real ESM has.
var __zippModuleDefs = {};
var __zippModuleNs = {};
function __zippDefine(url, fn) { __zippModuleDefs[url] = fn; }
function __zippResolveModuleUrl(base, spec) {
    try { return String(new URL(spec, base)); } catch (e) { return spec; }
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
            if (k !== 'default') {
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
        if (dbg) env.log('info', 'esm dynamic ' + u);
        return Promise.resolve(__zippRunModule(u));
    };
    var dynRel = function (base, spec) {
        var u = __zippResolveModuleUrl(base, spec);
        if (dbg) env.log('info', 'esm dynamic(rel) ' + spec + ' -> ' + u);
        return Promise.resolve(__zippRunModule(u));
    };
    var meta = { url: url, env: { MODE: 'production', PROD: true, DEV: false, BASE_URL: '/' } };
    if (typeof __zippEsmDebug !== 'undefined' && __zippEsmDebug) {
        env.log('info', 'esm run ' + url);
    }
    try {
        fn(imp, exp, expAll, dyn, dynRel, meta);
    } catch (e) {
        env.log('error', 'module threw: ' + url + ': ' + ((e && (e.stack || e.message)) || e));
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
        if spec.starts_with("./") {
            let dir = &base[..base.rfind('/').unwrap_or(0)];
            format!("{dir}/{}", &spec[2..])
        } else {
            spec.to_string()
        }
    }

    fn rw(src: &str) -> Rewritten {
        rewrite_module("http://x/a.js", src, &res).expect("parses")
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

    #[test]
    fn import_meta_is_replaced() {
        let r = rw("const u = import.meta.url;");
        assert!(r.body.contains("__meta.url"), "{}", r.body);
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
}
