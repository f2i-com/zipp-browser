//! Pre-compilation JS AST rewrites (via `oxc`) for `zipp-vm` compatibility.
//!
//! `zipp-vm` uses a 15-bit ordinary register space (`32,768` registers per
//! function frame) and deliberately retains staged literal blocks until the
//! enclosing statement finishes. Large real-world scripts (YouTube's
//! `ytInitialData` / `ytcfg`, Closure-compiled bundles with massive comma
//! expressions or thousands of `var` declarators, or large object/array tables)
//! can exceed 32,768 registers inside a single statement or function body.
//!
//! This module applies semantically-equivalent byte-span AST rewrites:
//! 1. Large pure-JSON object/array literals -> `JSON.parse("...")` (drops
//!    tens of thousands of registers to 2 and speeds up compilation ~50x).
//! 2. Giant `ExpressionStatement(SequenceExpression)` -> individual statements
//!    so `zipp-vm` reclaims scratch registers after every expression.
//! 3. Giant `VariableDeclaration` (`var`/`let`/`const` lists) -> individual
//!    declarations so initializer temporaries are reclaimed per declarator.
//! 4. Giant non-JSON object/array literals (containing functions/expressions,
//!    no direct `yield`/`await`) -> arrow-IIFE incremental builders, giving
//!    them an isolated frame with per-entry register reclamation.
//! 5. Three.js `StructuredUniform.setValue` lookup hoist for classic scripts.

use oxc_allocator::Allocator;
use oxc_ast::ast as ox;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

struct Edit {
    start: u32,
    end: u32,
    text: String,
}

#[derive(Clone, Copy)]
struct Config {
    json_min_bytes: u32,
    seq_min_len: usize,
    var_min_len: usize,
    obj_builder_min_props: usize,
    arr_builder_min_elems: usize,
    chunk_trigger_stmts: usize,
    chunk_max_stmts: usize,
}

const STANDARD: Config = Config {
    json_min_bytes: 4096,
    seq_min_len: 48,
    var_min_len: 48,
    obj_builder_min_props: 128,
    arr_builder_min_elems: 256,
    chunk_trigger_stmts: 3000,
    chunk_max_stmts: 500,
};

const AGGRESSIVE: Config = Config {
    json_min_bytes: 512,
    seq_min_len: 12,
    var_min_len: 12,
    obj_builder_min_props: 24,
    arr_builder_min_elems: 32,
    chunk_trigger_stmts: 600,
    chunk_max_stmts: 250,
};

/// Prepare a classic script before handing it to `zipp-vm`.
///
/// Small scripts without `.setValue(` skip parsing completely so there is no
/// overhead on ordinary pages.
pub(crate) fn prepare_script(source: &str) -> String {
    if source.len() < 4096 && !source.contains(".setValue(") {
        return source.to_string();
    }
    let (pass1, needs_chunk1) = run_pass(source, STANDARD, false, false);
    if pass1.len() >= 65_536 || needs_chunk1 {
        let (pass2, needs_chunk2) = run_pass(&pass1, STANDARD, true, false);
        if needs_chunk2 {
            run_pass(&pass2, STANDARD, false, true).0
        } else {
            pass2
        }
    } else {
        pass1
    }
}

/// Aggressive fallback used only when `zipp-vm` reports a 32,768-register
/// overflow on a huge minified bundle.
pub(crate) fn prepare_script_aggressive(source: &str) -> String {
    let (pass1, _) = run_pass(source, AGGRESSIVE, false, false);
    let (pass2, needs_chunk) = run_pass(&pass1, AGGRESSIVE, true, false);
    if needs_chunk {
        run_pass(&pass2, AGGRESSIVE, false, true).0
    } else {
        pass2
    }
}

/// True when a `zipp-vm` compile error is the 32,768-register frame limit.
pub(crate) fn is_register_limit_error(err: &str) -> bool {
    err.contains("more than 32768 registers") || err.contains("too many locals, temporaries or literal elements")
}

fn run_pass(source: &str, cfg: Config, enable_builders: bool, enable_chunking: bool) -> (String, bool) {
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::script()).parse();
    if !ret.errors.is_empty() {
        return (source.to_string(), false);
    }

    let mut edits: Vec<Edit> = Vec::new();
    let mut visitor = OptVisitor {
        source,
        cfg,
        enable_builders,
        enable_chunking,
        in_unsafe_control_flow: 0,
        saw_large_stmt_list: false,
        edits: &mut edits,
    };
    visitor.visit_program(&ret.program);
    let needs_chunk = visitor.saw_large_stmt_list;

    if edits.is_empty() {
        return (source.to_string(), needs_chunk);
    }
    (apply_edits(source, edits), needs_chunk)
}

fn apply_edits(source: &str, mut edits: Vec<Edit>) -> String {
    // Keep outer edits when spans overlap: sort by start ascending, end descending.
    edits.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));
    let extra: usize = edits.iter().map(|e| e.text.len()).sum();
    let mut out = String::with_capacity(source.len().saturating_add(extra / 2));
    let mut cursor = 0usize;
    for e in edits {
        let start = e.start as usize;
        let end = e.end as usize;
        if start < cursor || end > source.len() || start > end {
            continue;
        }
        out.push_str(&source[cursor..start]);
        out.push_str(&e.text);
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    out
}

struct OptVisitor<'a, 'b> {
    source: &'a str,
    cfg: Config,
    enable_builders: bool,
    enable_chunking: bool,
    /// Non-zero when inside a loop, switch, labeled statement, or `with`
    /// where chunking blocks into arrow IIFEs could disturb control flow or
    /// dynamic scope lookups.
    in_unsafe_control_flow: u32,
    saw_large_stmt_list: bool,
    edits: &'b mut Vec<Edit>,
}

impl<'a> OptVisitor<'a, '_> {
    fn visit_statement_list(&mut self, stmts: &[ox::Statement<'a>]) {
        if self.in_unsafe_control_flow == 0 && stmts.len() >= self.cfg.chunk_trigger_stmts {
            self.saw_large_stmt_list = true;
            // When a single flat statement list (e.g. a Closure-compiled IIFE or
            // try-block with 10,000+ statements) exceeds `chunk_trigger_stmts`,
            // partition runs of chunk-safe statements into immediately-invoked
            // arrow functions `(()=>{ ... })()`. Each chunk compiles in its own
            // fresh `zipp-vm` register frame (zero accumulated locals or stained
            // `reg_kind` slots), while hoisting simple `var` names to the
            // enclosing function scope so semantics stay identical.
            if self.enable_chunking && !block_has_direct_eval_or_with(stmts) {
                self.chunk_statement_list(stmts);
                return;
            }
        }

        for stmt in stmts {
            // 1. Split giant `var a=..., b=..., ...;` declarations in statement lists.
            if let ox::Statement::VariableDeclaration(decl) = stmt
                && decl.declarations.len() >= self.cfg.var_min_len
            {
                let span = decl.span();
                let kw = decl.kind.as_str();
                let mut rebuilt = String::with_capacity((span.end - span.start) as usize + decl.declarations.len() * 6);
                for d in &decl.declarations {
                    let ds = d.span();
                    rebuilt.push_str(kw);
                    rebuilt.push(' ');
                    rebuilt.push_str(&self.source[ds.start as usize..ds.end as usize]);
                    rebuilt.push(';');
                }
                self.edits.push(Edit {
                    start: span.start,
                    end: span.end,
                    text: rebuilt,
                });
                continue;
            }

            // 2. Split giant comma-expression statements `e1, e2, ..., eN;`.
            if let ox::Statement::ExpressionStatement(es) = stmt
                && let ox::Expression::SequenceExpression(seq) = &es.expression
                && seq.expressions.len() >= self.cfg.seq_min_len
            {
                let span = es.span();
                let mut rebuilt = String::with_capacity((span.end - span.start) as usize + seq.expressions.len() * 4);
                for ex in &seq.expressions {
                    let es_span = ex.span();
                    let frag = self.source[es_span.start as usize..es_span.end as usize].trim();
                    if frag.starts_with('{')
                        || frag.starts_with("function")
                        || frag.starts_with("class")
                        || frag.starts_with("async function")
                    {
                        rebuilt.push('(');
                        rebuilt.push_str(frag);
                        rebuilt.push_str(");");
                    } else {
                        rebuilt.push_str(frag);
                        rebuilt.push(';');
                    }
                }
                self.edits.push(Edit {
                    start: span.start,
                    end: span.end,
                    text: rebuilt,
                });
                continue;
            }

            self.visit_statement(stmt);
        }
    }

    fn chunk_statement_list(&mut self, stmts: &[ox::Statement<'a>]) {
        use std::collections::HashSet;

        struct ChunkItem<'s> {
            start: u32,
            end: u32,
            code: String,
            hoisted: Vec<&'s str>,
        }

        let flush_chunk = |items: &mut Vec<ChunkItem<'_>>, edits: &mut Vec<Edit>| {
            if items.len() < 16 {
                items.clear();
                return;
            }
            let first_start = items.first().unwrap().start;
            let last_end = items.last().unwrap().end;
            let mut seen: HashSet<&str> = HashSet::new();
            let mut vars: Vec<&str> = Vec::new();
            let mut body_bytes = 0usize;
            for item in items.iter() {
                body_bytes += item.code.len() + 1;
                for &v in &item.hoisted {
                    if seen.insert(v) {
                        vars.push(v);
                    }
                }
            }
            let mut out = String::with_capacity(body_bytes + vars.len() * 8 + 32);
            if !vars.is_empty() {
                out.push_str("var ");
                for (i, v) in vars.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(v);
                }
                out.push(';');
            }
            out.push_str("(()=>{");
            for item in items.iter() {
                out.push_str(&item.code);
                if !item.code.ends_with(';') && !item.code.ends_with('}') {
                    out.push(';');
                }
                out.push('\n');
            }
            out.push_str("})();");
            edits.push(Edit {
                start: first_start,
                end: last_end,
                text: out,
            });
            items.clear();
        };

        let mut chunk: Vec<ChunkItem<'a>> = Vec::with_capacity(self.cfg.chunk_max_stmts);
        for stmt in stmts {
            match prepare_chunk_item(stmt, self.source) {
                Some((code, hoisted)) => {
                    let sp = stmt.span();
                    chunk.push(ChunkItem {
                        start: sp.start,
                        end: sp.end,
                        code,
                        hoisted,
                    });
                    if chunk.len() >= self.cfg.chunk_max_stmts {
                        flush_chunk(&mut chunk, self.edits);
                    }
                }
                None => {
                    flush_chunk(&mut chunk, self.edits);
                    self.visit_statement(stmt);
                }
            }
        }
        flush_chunk(&mut chunk, self.edits);
    }
}

impl<'a> Visit<'a> for OptVisitor<'a, '_> {
    fn visit_program(&mut self, it: &ox::Program<'a>) {
        self.visit_statement_list(&it.body);
    }

    fn visit_block_statement(&mut self, it: &ox::BlockStatement<'a>) {
        self.visit_statement_list(&it.body);
    }

    fn visit_function_body(&mut self, it: &ox::FunctionBody<'a>) {
        let saved = self.in_unsafe_control_flow;
        self.in_unsafe_control_flow = 0;
        self.visit_statement_list(&it.statements);
        self.in_unsafe_control_flow = saved;
    }

    fn visit_for_statement(&mut self, it: &ox::ForStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_for_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_for_in_statement(&mut self, it: &ox::ForInStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_for_in_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_for_of_statement(&mut self, it: &ox::ForOfStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_for_of_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_while_statement(&mut self, it: &ox::WhileStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_while_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_do_while_statement(&mut self, it: &ox::DoWhileStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_do_while_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_switch_statement(&mut self, it: &ox::SwitchStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_switch_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_labeled_statement(&mut self, it: &ox::LabeledStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_labeled_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_with_statement(&mut self, it: &ox::WithStatement<'a>) {
        self.in_unsafe_control_flow += 1;
        oxc_ast_visit::walk::walk_with_statement(self, it);
        self.in_unsafe_control_flow -= 1;
    }

    fn visit_expression_statement(&mut self, it: &ox::ExpressionStatement<'a>) {
        // Hoist Three.js `StructuredUniform.setValue`: `u.setValue(gl, value[u.id], textures)`.
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
            let rs = recv.span();
            let a0 = call.arguments[0].span();
            let a1 = call.arguments[1].span();
            let a2 = call.arguments[2].span();
            let r_txt = &self.source[rs.start as usize..rs.end as usize];
            let a0_txt = &self.source[a0.start as usize..a0.end as usize];
            let a1_txt = &self.source[a1.start as usize..a1.end as usize];
            let a2_txt = &self.source[a2.start as usize..a2.end as usize];
            self.edits.push(Edit {
                start: stmt_span.start,
                end: stmt_span.end,
                text: format!("{{let __zv={a1_txt};{r_txt}.setValue({a0_txt},__zv,{a2_txt});}}"),
            });
            return;
        }
        oxc_ast_visit::walk::walk_expression_statement(self, it);
    }

    fn visit_object_expression(&mut self, it: &ox::ObjectExpression<'a>) {
        let span = it.span();
        let byte_len = span.end - span.start;
        if byte_len >= self.cfg.json_min_bytes {
            let mut json = String::with_capacity(byte_len as usize);
            if write_json_object(it, &mut json) {
                self.edits.push(Edit {
                    start: span.start,
                    end: span.end,
                    text: wrap_json_parse(&json),
                });
                return;
            }
        }
        if self.enable_builders
            && it.properties.len() >= self.cfg.obj_builder_min_props
            && let Some(builder) = build_object_iife(it, self.source)
        {
            self.edits.push(Edit {
                start: span.start,
                end: span.end,
                text: builder,
            });
            return;
        }
        oxc_ast_visit::walk::walk_object_expression(self, it);
    }

    fn visit_array_expression(&mut self, it: &ox::ArrayExpression<'a>) {
        let span = it.span();
        let byte_len = span.end - span.start;
        if byte_len >= self.cfg.json_min_bytes {
            let mut json = String::with_capacity(byte_len as usize);
            if write_json_array(it, &mut json) {
                self.edits.push(Edit {
                    start: span.start,
                    end: span.end,
                    text: wrap_json_parse(&json),
                });
                return;
            }
        }
        if self.enable_builders
            && it.elements.len() >= self.cfg.arr_builder_min_elems
            && let Some(builder) = build_array_iife(it, self.source)
        {
            self.edits.push(Edit {
                start: span.start,
                end: span.end,
                text: builder,
            });
            return;
        }
        oxc_ast_visit::walk::walk_array_expression(self, it);
    }
}

/// Emit `JSON.parse("...")` with safe JS string escaping (including U+2028/U+2029).
fn wrap_json_parse(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + 24);
    out.push_str("JSON.parse(\"");
    for ch in json.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out.push_str("\")");
    out
}

fn push_json_string(s: &str, out: &mut String) {
    if let Ok(encoded) = serde_json::to_string(s) {
        out.push_str(&encoded);
    }
}

fn write_json_expr(expr: &ox::Expression<'_>, out: &mut String) -> bool {
    match expr {
        ox::Expression::NullLiteral(_) => {
            out.push_str("null");
            true
        }
        ox::Expression::BooleanLiteral(b) => {
            out.push_str(if b.value { "true" } else { "false" });
            true
        }
        ox::Expression::StringLiteral(s) => {
            push_json_string(s.value.as_str(), out);
            true
        }
        ox::Expression::NumericLiteral(n) => {
            if !n.value.is_finite() || (n.value == 0.0 && n.value.is_sign_negative()) {
                return false;
            }
            if let Some(num) = serde_json::Number::from_f64(n.value) {
                out.push_str(&num.to_string());
                true
            } else {
                false
            }
        }
        ox::Expression::UnaryExpression(u) => {
            if u.operator == ox::UnaryOperator::UnaryNegation
                && let ox::Expression::NumericLiteral(n) = &u.argument
                && n.value.is_finite()
                && n.value != 0.0
                && let Some(num) = serde_json::Number::from_f64(-n.value)
            {
                out.push_str(&num.to_string());
                return true;
            }
            false
        }
        ox::Expression::ArrayExpression(arr) => write_json_array(arr, out),
        ox::Expression::ObjectExpression(obj) => write_json_object(obj, out),
        _ => false,
    }
}

fn write_json_array(arr: &ox::ArrayExpression<'_>, out: &mut String) -> bool {
    out.push('[');
    for (idx, elem) in arr.elements.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let Some(expr) = elem.as_expression() else {
            return false;
        };
        if !write_json_expr(expr, out) {
            return false;
        }
    }
    out.push(']');
    true
}

fn write_json_object(obj: &ox::ObjectExpression<'_>, out: &mut String) -> bool {
    out.push('{');
    for (idx, prop_kind) in obj.properties.iter().enumerate() {
        let ox::ObjectPropertyKind::ObjectProperty(prop) = prop_kind else {
            return false;
        };
        if prop.kind != ox::PropertyKind::Init || prop.method || prop.computed {
            return false;
        }
        let key_owned: String;
        let key: &str = match &prop.key {
            ox::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            ox::PropertyKey::StringLiteral(s) => s.value.as_str(),
            ox::PropertyKey::NumericLiteral(n) if n.value.is_finite() => {
                if n.value.fract() == 0.0 && n.value >= 0.0 && n.value <= 9_007_199_254_740_991.0 {
                    key_owned = format!("{}", n.value as u64);
                } else {
                    key_owned = n.value.to_string();
                }
                &key_owned
            }
            _ => return false,
        };
        if key == "__proto__" {
            return false;
        }
        if idx > 0 {
            out.push(',');
        }
        push_json_string(key, out);
        out.push(':');
        if !write_json_expr(&prop.value, out) {
            return false;
        }
    }
    out.push('}');
    true
}

/// Checks whether an expression contains a direct `yield` or `await` outside
/// nested functions (the only constructs that cannot cross a non-async arrow
/// boundary; `this`, `arguments`, `super`, `new.target` are lexically inherited).
#[derive(Default)]
struct DirectSuspendScan {
    depth: u32,
    has_suspend: bool,
}

impl<'a> Visit<'a> for DirectSuspendScan {
    fn visit_function_body(&mut self, it: &ox::FunctionBody<'a>) {
        self.depth += 1;
        oxc_ast_visit::walk::walk_function_body(self, it);
        self.depth -= 1;
    }

    fn visit_await_expression(&mut self, it: &ox::AwaitExpression<'a>) {
        if self.depth == 0 {
            self.has_suspend = true;
            return;
        }
        oxc_ast_visit::walk::walk_await_expression(self, it);
    }

    fn visit_yield_expression(&mut self, it: &ox::YieldExpression<'a>) {
        if self.depth == 0 {
            self.has_suspend = true;
            return;
        }
        oxc_ast_visit::walk::walk_yield_expression(self, it);
    }
}

/// Checks whether a statement list contains direct `eval(...)` or `with (...)`
/// (outside nested functions), where introducing an arrow IIFE boundary could
/// change dynamic scope resolution.
fn block_has_direct_eval_or_with<'a>(stmts: &[ox::Statement<'a>]) -> bool {
    #[derive(Default)]
    struct EvalWithScan {
        fn_depth: u32,
        found: bool,
    }
    impl<'a> Visit<'a> for EvalWithScan {
        fn visit_function_body(&mut self, it: &ox::FunctionBody<'a>) {
            self.fn_depth += 1;
            oxc_ast_visit::walk::walk_function_body(self, it);
            self.fn_depth -= 1;
        }
        fn visit_with_statement(&mut self, _it: &ox::WithStatement<'a>) {
            if self.fn_depth == 0 {
                self.found = true;
            }
        }
        fn visit_call_expression(&mut self, it: &ox::CallExpression<'a>) {
            if self.fn_depth == 0
                && let ox::Expression::Identifier(id) = &it.callee
                && id.name == "eval"
            {
                self.found = true;
                return;
            }
            oxc_ast_visit::walk::walk_call_expression(self, it);
        }
    }
    let mut scan = EvalWithScan::default();
    for s in stmts {
        scan.visit_statement(s);
        if scan.found {
            return true;
        }
    }
    false
}

/// Inspects a single statement to decide whether it can safely execute inside
/// a chunk arrow IIFE `(()=>{ ... })()`, returning `(chunk_code, hoisted_vars)`.
fn prepare_chunk_item<'a>(stmt: &ox::Statement<'a>, source: &'a str) -> Option<(String, Vec<&'a str>)> {
    // Declarations that are block-scoped or hoisted as functions stay outside chunks.
    match stmt {
        ox::Statement::FunctionDeclaration(_)
        | ox::Statement::ClassDeclaration(_)
        | ox::Statement::ImportDeclaration(_)
        | ox::Statement::ExportAllDeclaration(_)
        | ox::Statement::ExportDefaultDeclaration(_)
        | ox::Statement::ExportNamedDeclaration(_) => return None,
        ox::Statement::VariableDeclaration(decl) => {
            if decl.kind != ox::VariableDeclarationKind::Var {
                return None;
            }
            let mut hoisted: Vec<&'a str> = Vec::with_capacity(decl.declarations.len());
            let mut code = String::new();
            for d in &decl.declarations {
                let ox::BindingPattern::BindingIdentifier(id) = &d.id else {
                    return None;
                };
                let name = id.name.as_str();
                if name == "eval" || name == "arguments" {
                    return None;
                }
                hoisted.push(name);
                if let Some(init) = &d.init {
                    let mut scan = DirectSuspendScan::default();
                    scan.visit_expression(init);
                    if scan.has_suspend {
                        return None;
                    }
                    let is = init.span();
                    let init_src = source[is.start as usize..is.end as usize].trim();
                    code.push_str(name);
                    code.push('=');
                    // Preserve NamedEvaluation for `var C = class ...` / `var f = function ...`.
                    if matches!(
                        init,
                        ox::Expression::FunctionExpression(_)
                            | ox::Expression::ArrowFunctionExpression(_)
                            | ox::Expression::ClassExpression(_)
                    ) {
                        code.push_str(init_src);
                    } else {
                        code.push('(');
                        code.push_str(init_src);
                        code.push(')');
                    }
                    code.push(';');
                }
            }
            return Some((code, hoisted));
        }
        _ => {}
    }

    // Any other statement: safe verbatim iff it contains no direct return,
    // yield, await, var/function declaration, eval, with, or escaping
    // break/continue outside nested functions.
    #[derive(Default)]
    struct StmtChunkScan {
        fn_depth: u32,
        loop_depth: u32,
        switch_depth: u32,
        unsafe_stmt: bool,
    }
    impl<'a> Visit<'a> for StmtChunkScan {
        fn visit_function_body(&mut self, it: &ox::FunctionBody<'a>) {
            self.fn_depth += 1;
            oxc_ast_visit::walk::walk_function_body(self, it);
            self.fn_depth -= 1;
        }
        fn visit_return_statement(&mut self, _it: &ox::ReturnStatement<'a>) {
            if self.fn_depth == 0 {
                self.unsafe_stmt = true;
            }
        }
        fn visit_yield_expression(&mut self, _it: &ox::YieldExpression<'a>) {
            if self.fn_depth == 0 {
                self.unsafe_stmt = true;
            }
        }
        fn visit_await_expression(&mut self, _it: &ox::AwaitExpression<'a>) {
            if self.fn_depth == 0 {
                self.unsafe_stmt = true;
            }
        }
        fn visit_with_statement(&mut self, _it: &ox::WithStatement<'a>) {
            if self.fn_depth == 0 {
                self.unsafe_stmt = true;
            }
        }
        fn visit_statement(&mut self, it: &ox::Statement<'a>) {
            if self.fn_depth == 0 && matches!(it, ox::Statement::FunctionDeclaration(_)) {
                self.unsafe_stmt = true;
                return;
            }
            oxc_ast_visit::walk::walk_statement(self, it);
        }
        fn visit_variable_declaration(&mut self, it: &ox::VariableDeclaration<'a>) {
            if self.fn_depth == 0 && it.kind == ox::VariableDeclarationKind::Var {
                self.unsafe_stmt = true;
                return;
            }
            oxc_ast_visit::walk::walk_variable_declaration(self, it);
        }
        fn visit_break_statement(&mut self, it: &ox::BreakStatement<'a>) {
            if self.fn_depth == 0 && (it.label.is_some() || (self.loop_depth == 0 && self.switch_depth == 0)) {
                self.unsafe_stmt = true;
            }
        }
        fn visit_continue_statement(&mut self, it: &ox::ContinueStatement<'a>) {
            if self.fn_depth == 0 && (it.label.is_some() || self.loop_depth == 0) {
                self.unsafe_stmt = true;
            }
        }
        fn visit_for_statement(&mut self, it: &ox::ForStatement<'a>) {
            self.loop_depth += 1;
            oxc_ast_visit::walk::walk_for_statement(self, it);
            self.loop_depth -= 1;
        }
        fn visit_for_in_statement(&mut self, it: &ox::ForInStatement<'a>) {
            self.loop_depth += 1;
            oxc_ast_visit::walk::walk_for_in_statement(self, it);
            self.loop_depth -= 1;
        }
        fn visit_for_of_statement(&mut self, it: &ox::ForOfStatement<'a>) {
            self.loop_depth += 1;
            oxc_ast_visit::walk::walk_for_of_statement(self, it);
            self.loop_depth -= 1;
        }
        fn visit_while_statement(&mut self, it: &ox::WhileStatement<'a>) {
            self.loop_depth += 1;
            oxc_ast_visit::walk::walk_while_statement(self, it);
            self.loop_depth -= 1;
        }
        fn visit_do_while_statement(&mut self, it: &ox::DoWhileStatement<'a>) {
            self.loop_depth += 1;
            oxc_ast_visit::walk::walk_do_while_statement(self, it);
            self.loop_depth -= 1;
        }
        fn visit_switch_statement(&mut self, it: &ox::SwitchStatement<'a>) {
            self.switch_depth += 1;
            oxc_ast_visit::walk::walk_switch_statement(self, it);
            self.switch_depth -= 1;
        }
    }

    let mut scan = StmtChunkScan::default();
    scan.visit_statement(stmt);
    if scan.unsafe_stmt {
        return None;
    }
    let sp = stmt.span();
    Some((source[sp.start as usize..sp.end as usize].to_string(), Vec::new()))
}

fn build_object_iife(obj: &ox::ObjectExpression<'_>, source: &str) -> Option<String> {
    let mut scan = DirectSuspendScan::default();
    scan.visit_object_expression(obj);
    if scan.has_suspend {
        return None;
    }
    let span = obj.span();
    let mut out = String::with_capacity((span.end - span.start) as usize + obj.properties.len() * 12 + 32);
    out.push_str("(()=>{var __zo={};");
    for prop_kind in &obj.properties {
        match prop_kind {
            ox::ObjectPropertyKind::ObjectProperty(prop) => {
                if prop.kind != ox::PropertyKind::Init || prop.method {
                    return None;
                }
                let val_span = prop.value.span();
                let val_src = &source[val_span.start as usize..val_span.end as usize];
                if prop.computed {
                    let ks = prop.key.span();
                    let key_src = &source[ks.start as usize..ks.end as usize];
                    out.push_str("__zo[");
                    out.push_str(key_src);
                    out.push_str("]=(");
                    out.push_str(val_src);
                    out.push_str(");");
                } else {
                    let key_owned: String;
                    let key: &str = match &prop.key {
                        ox::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                        ox::PropertyKey::StringLiteral(s) => s.value.as_str(),
                        ox::PropertyKey::NumericLiteral(n) if n.value.is_finite() => {
                            key_owned = n.value.to_string();
                            &key_owned
                        }
                        _ => return None,
                    };
                    if key == "__proto__" {
                        return None;
                    }
                    out.push_str("__zo[");
                    push_json_string(key, &mut out);
                    out.push_str("]=(");
                    out.push_str(val_src);
                    out.push_str(");");
                }
            }
            ox::ObjectPropertyKind::SpreadProperty(sp) => {
                let s = sp.argument.span();
                out.push_str("Object.assign(__zo,");
                out.push_str(&source[s.start as usize..s.end as usize]);
                out.push_str(");");
            }
        }
    }
    out.push_str("return __zo;})()");
    Some(out)
}

fn build_array_iife(arr: &ox::ArrayExpression<'_>, source: &str) -> Option<String> {
    let mut scan = DirectSuspendScan::default();
    scan.visit_array_expression(arr);
    if scan.has_suspend {
        return None;
    }
    let span = arr.span();
    let mut out = String::with_capacity((span.end - span.start) as usize + arr.elements.len() * 10 + 32);
    out.push_str("(()=>{var __za=[];");
    for elem in &arr.elements {
        match elem {
            ox::ArrayExpressionElement::Elision(_) => return None,
            ox::ArrayExpressionElement::SpreadElement(sp) => {
                let s = sp.argument.span();
                out.push_str("for(var __zs of (");
                out.push_str(&source[s.start as usize..s.end as usize]);
                out.push_str("))__za.push(__zs);");
            }
            other => {
                let expr = other.as_expression()?;
                let s = expr.span();
                out.push_str("__za.push(");
                out.push_str(&source[s.start as usize..s.end as usize]);
                out.push_str(");");
            }
        }
    }
    out.push_str("return __za;})()");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zipp_vm::embed::{JsValue, compile_script};

    #[test]
    fn rewrites_huge_json_object_literal_to_json_parse() {
        let mut src = String::from("var ytInitialData = { contents: [");
        for i in 0..60 {
            if i > 0 {
                src.push(',');
            }
            src.push_str(&format!(r##"{{ id: "vid_{i}", title: "Video \"#{i}\"", blocks: ["##));
            for j in 0..20 {
                if j > 0 {
                    src.push(',');
                }
                src.push_str(&format!(
                    r##"{{ a: {j}, b: "t{j}", c: true, d: null, e: [1, 2, 3, 4], f: {{ x: {i}, y: {j}, z: "ok" }} }}"##
                ));
            }
            src.push_str("] }");
        }
        src.push_str("] };");

        let prepared = prepare_script(&src);
        assert!(prepared.contains("JSON.parse("), "expected JSON.parse rewrite");
        let mut state = compile_script("").expect("empty realm compiles");
        state.run_init().unwrap();
        state.run_script(&prepared).expect("prepared script executes cleanly");
        let res = state.run_script("ytInitialData.contents.length").unwrap();
        assert_eq!(res, JsValue::Number(60.0));
        let title = state.run_script("ytInitialData.contents[5].title").unwrap();
        assert_eq!(title, JsValue::String("Video \"#5\"".into()));
    }

    #[test]
    fn chunks_giant_statement_list_past_32768_register_limit() {
        // Reproduce zipp-vm's 32768-register overflow on Closure-style IIFEs:
        // a single function whose try-block has thousands of statements with
        // alternating numeric/boolean argument calls and hoisted `var` bindings
        // stains `reg_kind` slots across statements until `alloc_block` crosses
        // 32768 registers.
        let mut src = String::from(
            "var out = {};\n\
             (function(_) {\n\
               var selfRef = this;\n\
               try {\n\
                 var sum = 0;\n\
                 function sink(a, b, c, d) { sum += (a ? 1 : 0) + (b ? 1 : 0); }\n",
        );
        // 6,000 alternating boolean/numeric call statements plus `var` bindings
        // and a NamedEvaluation class expression inside the try block.
        src.push_str("var C0 = class { static tag() { return 7; } };\n");
        for i in 0..4000 {
            if i % 2 == 0 {
                src.push_str(&format!("var v{i} = {i}; sink(1, !0, {i}, !1);\n"));
            } else {
                src.push_str(&format!("sink(!0, {i}, !1, 2);\n"));
            }
        }
        src.push_str(
            "_.sum = sum;\n\
             _.last = v3998;\n\
             _.clsName = C0.name;\n\
             _.clsTag = C0.tag();\n\
             _.sameThis = (selfRef === this);\n\
             try { throw new Error('boom'); } catch (inner) { _.caughtInner = inner.message; }\n\
           } catch (e) { _.caughtOuter = String(e); }\n\
         }).call(out, out);",
        );

        let prepared = prepare_script_aggressive(&src);
        assert!(prepared.contains("(()=>{"), "expected arrow-IIFE statement chunking");
        let mut state = compile_script("").expect("empty realm compiles");
        state.run_init().unwrap();
        state
            .run_script(&prepared)
            .expect("chunked bundle compiles and executes cleanly");
        assert_eq!(state.run_script("out.sum").unwrap(), JsValue::Number(8000.0));
        assert_eq!(state.run_script("out.last").unwrap(), JsValue::Number(3998.0));
        assert_eq!(state.run_script("out.clsName").unwrap(), JsValue::String("C0".into()));
        assert_eq!(state.run_script("out.clsTag").unwrap(), JsValue::Number(7.0));
        assert_eq!(state.run_script("out.sameThis").unwrap(), JsValue::Bool(true));
        assert_eq!(
            state.run_script("out.caughtInner").unwrap(),
            JsValue::String("boom".into())
        );
    }

    #[test]
    fn splits_huge_comma_sequence_and_non_json_object_table() {
        let mut seq = String::from("var acc = 0;\n");
        for i in 0..2000 {
            if i > 0 {
                seq.push(',');
            }
            seq.push_str("(acc += 1)");
        }
        seq.push_str(";\nvar tbl = {");
        for i in 0..200 {
            if i > 0 {
                seq.push(',');
            }
            seq.push_str(&format!("k{i}: function() {{ return {i}; }}"));
        }
        seq.push_str("};\nacc + tbl.k199();");

        let prepared = prepare_script_aggressive(&seq);
        assert!(prepared.contains("var __zo={};"));
        let mut state = compile_script("").expect("empty realm compiles");
        state.run_init().unwrap();
        let res = state.run_script(&prepared).expect("prepared bundle runs");
        assert_eq!(res, JsValue::Number(2199.0));
    }

    #[test]
    fn preserves_proto_and_key_order_without_misrewriting() {
        let src = format!(
            "var o = {{ __proto__: null, pad: \"{}\" }}; Object.getPrototypeOf(o) === null;",
            "x".repeat(5000)
        );
        let prepared = prepare_script(&src);
        assert!(!prepared.contains("JSON.parse("), "must not rewrite __proto__ object");
        let mut state = compile_script("").expect("empty realm compiles");
        state.run_init().unwrap();
        assert_eq!(state.run_script(&prepared).unwrap(), JsValue::Bool(true));
    }
}
