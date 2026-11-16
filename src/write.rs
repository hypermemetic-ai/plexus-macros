//! What is written from the IR: its WIT and its TypeScript. Both move to
//! plexus-gen; the IR itself is `plexus-ir`'s.

use plexus_ir::{Field, Ir, Method, Returns, Shape, Ty};
use serde_json::Value;

// ----------------------------------------------------------------- WIT

fn doc_lines(doc: &str, indent: &str, mark: &str) -> String {
    doc.lines().map(|l| format!("{indent}{mark} {l}\n")).collect()
}

fn ty_wit(t: &Ty) -> String {
    match t {
        Ty::Prim("bytes") => "list<u8>".into(),
        Ty::Prim("json") => "string".into(),
        Ty::Prim(p) => p.to_string(),
        Ty::Named(n) => n.clone(),
        Ty::List(t) => format!("list<{}>", ty_wit(t)),
        Ty::Option(t) => format!("option<{}>", ty_wit(t)),
        Ty::Map(t) => format!("list<tuple<string, {}>>", ty_wit(t)),
        Ty::Tuple(ts) => format!("tuple<{}>", ts.iter().map(ty_wit).collect::<Vec<_>>().join(", ")),
    }
}

fn mentions(t: &Ty, prim: &str) -> bool {
    match t {
        Ty::Prim(p) => *p == prim,
        Ty::Named(_) => false,
        Ty::List(t) | Ty::Option(t) | Ty::Map(t) => mentions(t, prim),
        Ty::Tuple(ts) => ts.iter().any(|t| mentions(t, prim)),
    }
}

fn every_type(ir: &Ir) -> Vec<&Ty> {
    ir.references().into_iter().map(|(t, _)| t).collect()
}

/// What the IR says that WIT has no word for, one line each.
fn unsaid(ir: &Ir) -> Vec<String> {
    let mut out = Vec::new();
    let any = |f: &dyn Fn(&Method) -> bool| ir.methods.iter().any(|m| f(m));
    let fields_any = |f: &dyn Fn(&Field) -> bool| {
        ir.methods.iter().any(|m| m.params.iter().any(|p| f(p)))
            || ir.types.iter().any(|t| matches!(&t.shape, Shape::Record(fs) | Shape::Variant(fs) if fs.iter().any(|x| f(x) || x.record.as_ref().is_some_and(|r| r.iter().any(|y| f(y))))))
    };
    if any(&|m| m.caller.is_some()) {
        out.push("which methods read the caller".into());
    }
    if any(&|m| m.grant.is_some()) || !ir.roles.is_empty() {
        out.push("grants, targets and roles".into());
    }
    if !ir.children.is_empty() {
        out.push("the children".into());
    }
    if any(&|m| m.progress.is_some()) {
        out.push("progress".into());
    }
    if fields_any(&|f| f.default.is_some()) {
        out.push("defaults".into());
    }
    if ir.types.iter().any(|t| matches!(&t.shape, Shape::Alias(_, l) if !l.is_empty())) {
        out.push("limits on a value".into());
    }
    let ts = every_type(ir);
    if any(&|m| m.input.is_some()) {
        out.push("a stream in".into());
    }
    if fields_any(&|f| f.deprecated.is_some()) {
        out.push("deprecation".into());
    }
    if ts.iter().any(|t| mentions(t, "json")) {
        out.push("any-JSON (sent as a string)".into());
    }
    if ts.iter().any(|t| matches!(t, Ty::Map(_))) {
        out.push("a map (sent as a list of pairs)".into());
    }
    if ir.types.iter().any(|t| matches!(&t.shape, Shape::Enum(cs) | Shape::Variant(cs) if cs.iter().any(|c| c.wire.is_some()))) {
        out.push("wire names".into());
    }
    out
}

/// The IR as a WIT package: one interface, one world that exports it.
/// A stream out is WIT's own `stream<T>`.
pub fn wit(ir: &Ir) -> String {
    let name = &ir.activation;
    let unsaid = unsaid(ir);
    let note = if unsaid.is_empty() { String::new() } else { format!("// In the IR and not in WIT: {}.\n", unsaid.join("; ")) };
    let mut out = format!("package plexus:{name}@0.0.1;\n\n{note}{}interface {name} {{\n", doc_lines(&ir.doc, "", "///"));
    for t in &ir.types {
        // A case that carries a record gets a record of its own first.
        if let Shape::Variant(cases) = &t.shape {
            for c in cases {
                if let Some(fields) = &c.record {
                    out += &format!("  record {}-{} {{\n", t.name, c.name);
                    for f in fields {
                        out += &doc_lines(&f.doc, "    ", "///");
                        out += &format!("    {}: {},\n", f.name, ty_wit(f.ty.as_ref().expect("a field has a type")));
                    }
                    out += "  }\n\n";
                }
            }
        }
        out += &doc_lines(&t.doc, "  ", "///");
        let (word, fields) = match &t.shape {
            Shape::Record(f) => ("record", f),
            Shape::Variant(f) => ("variant", f),
            Shape::Enum(f) => ("enum", f),
            Shape::Alias(ty, _) => {
                out += &format!("  type {} = {};\n\n", t.name, ty_wit(ty));
                continue;
            }
        };
        out += &format!("  {word} {} {{\n", t.name);
        for f in fields {
            out += &doc_lines(&f.doc, "    ", "///");
            out += &match (&t.shape, &f.ty, &f.record) {
                (Shape::Record(_), Some(ty), _) => format!("    {}: {},\n", f.name, ty_wit(ty)),
                (_, Some(ty), _) => format!("    {}({}),\n", f.name, ty_wit(ty)),
                (_, None, Some(_)) => format!("    {0}({1}-{0}),\n", f.name, t.name),
                (_, None, None) => format!("    {},\n", f.name),
            };
        }
        out += "  }\n\n";
    }
    for m in &ir.methods {
        out += &doc_lines(&m.doc, "  ", "///");
        let params = m.params.iter().map(|p| format!("{}: {}", p.name, ty_wit(p.ty.as_ref().expect("a parameter has a type")))).collect::<Vec<_>>().join(", ");
        let ret = match (&m.returns, &m.fails) {
            (Returns::Stream(t), _) => format!(" -> stream<{}>", ty_wit(t)),
            (Returns::Nothing, None) => String::new(),
            (Returns::Nothing, Some(e)) => format!(" -> result<_, {}>", ty_wit(e)),
            (Returns::One(t), None) => format!(" -> {}", ty_wit(t)),
            (Returns::One(t), Some(e)) => format!(" -> result<{}, {}>", ty_wit(t), ty_wit(e)),
        };
        let word = if m.waits { "async func" } else { "func" };
        out += &format!("  {}: {word}({params}){ret};\n", m.name);
    }
    out + &format!("}}\n\nworld {name}-activation {{\n  export {name};\n}}\n")
}

fn camel(kebab: &str) -> String {
    kebab.split('-').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect()
}

// ---------------------------------------------------------- TypeScript

fn lower_camel(kebab: &str) -> String {
    let c = camel(kebab);
    let mut chars = c.chars();
    chars.next().map(|f| f.to_lowercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// A type as TypeScript, in the shapes a transpiled component uses.
fn ty_ts(t: &Ty) -> String {
    match t {
        Ty::Prim("string") | Ty::Prim("char") | Ty::Prim("json") => "string".into(),
        Ty::Prim("bool") => "boolean".into(),
        Ty::Prim("u64") | Ty::Prim("s64") => "bigint".into(),
        Ty::Prim("bytes") => "Uint8Array".into(),
        Ty::Prim(_) => "number".into(),
        Ty::Named(n) => camel(n),
        Ty::List(t) => format!("Array<{}>", ty_ts(t)),
        Ty::Option(t) => format!("{} | undefined", ty_ts(t)),
        Ty::Map(t) => format!("Array<[string, {}]>", ty_ts(t)),
        Ty::Tuple(ts) => format!("[{}]", ts.iter().map(ty_ts).collect::<Vec<_>>().join(", ")),
    }
}

/// The expression that turns `v` between its TypeScript shape and its
/// JSON on the wire. `out` is towards the wire.
fn conv(t: &Ty, v: &str, out: bool) -> String {
    let dir = if out { "to" } else { "from" };
    match t {
        Ty::Prim("u64") | Ty::Prim("s64") => if out { format!("Number({v})") } else { format!("BigInt({v})") },
        Ty::Prim("bytes") => if out { format!("b64({v})") } else { format!("unb64({v})") },
        Ty::Prim("json") => if out { format!("JSON.parse({v})") } else { format!("JSON.stringify({v})") },
        Ty::Prim(_) => v.to_string(),
        Ty::Named(n) => format!("{dir}{}({v})", camel(n)),
        Ty::List(t) if conv(t, "x", out) == "x" => v.to_string(),
        Ty::List(t) => format!("{v}.map((x: any) => {})", conv(t, "x", out)),
        Ty::Option(t) => format!("({v} == null ? undefined : {})", conv(t, v, out)),
        Ty::Map(t) => if out {
            format!("Object.fromEntries({v}.map(([k, x]: [string, any]) => [k, {}]))", conv(t, "x", out))
        } else {
            format!("Object.entries({v}).map(([k, x]: [string, any]) => [k, {}])", conv(t, "x", out))
        },
        Ty::Tuple(ts) => format!("[{}]", ts.iter().enumerate().map(|(i, t)| conv(t, &format!("{v}[{i}]"), out)).collect::<Vec<_>>().join(", ")),
    }
}

/// A record's fields, between an object with camelCase keys and the wire's object.
fn conv_fields(fields: &[Field], v: &str, out: bool) -> String {
    let parts: Vec<String> = fields.iter().map(|f| {
        let (ts, wire) = (lower_camel(&f.name), &f.name);
        let ty = f.ty.as_ref().expect("a field has a type");
        if out { format!("\"{wire}\": {}", conv(ty, &format!("{v}.{ts}"), true)) } else { format!("{ts}: {}", conv(ty, &format!("{v}[\"{wire}\"]"), false)) }
    }).collect();
    format!("{{ {} }}", parts.join(", "))
}

/// A bare case's name between TypeScript and the wire, when some case has
/// a wire name; else `v` as it is.
fn renamed(cases: &[Field], v: &str, out: bool) -> String {
    let pairs: Vec<String> = cases.iter().filter_map(|c| {
        let (n, w) = (Value::from(c.name.as_str()), Value::from(c.wire.as_deref()?));
        Some(if out { format!("{n}: {w}") } else { format!("{w}: {n}") })
    }).collect();
    if pairs.is_empty() { v.to_string() } else { format!("(({{ {} }} as any)[{v}] ?? {v})", pairs.join(", ")) }
}

fn ts_fields(fields: &[Field]) -> String {
    fields.iter().map(|f| {
        let ty = f.ty.as_ref().expect("a field has a type");
        match ty {
            Ty::Option(inner) => format!("{}?: {}", lower_camel(&f.name), ty_ts(inner)),
            _ => format!("{}: {}", lower_camel(&f.name), ty_ts(ty)),
        }
    }).collect::<Vec<_>>().join("; ")
}

/// A typed client and the activation's side of the same calls, as one
/// TypeScript module. Both speak frames over a `Channel`.
///
/// On the wire a record is an object keyed by the IR's names; an enum
/// is its name; a variant's case is `{"case": value}`, or its bare name
/// when it carries nothing. A case with a wire name is written and read
/// by that name instead; in TypeScript it keeps its own.
pub fn ts(ir: &Ir) -> String {
    let svc = camel(&ir.activation);
    let mut out = format!(
        "// Generated from the IR of `{}` (interface {}). Not edited by hand.\n\nimport {{ Connection, Server, each, b64, unb64 }} from \"./plexus-rt.ts\";\n\n",
        ir.activation, ir.hash()
    );
    if !every_type(ir).iter().any(|t| mentions(t, "bytes")) {
        out = out.replace(", b64, unb64", "");
    }
    // The types, in the shapes a transpiled component uses.
    for t in &ir.types {
        out += &doc_lines(&t.doc, "", "///");
        let name = camel(&t.name);
        match &t.shape {
            Shape::Record(fields) => out += &format!("export interface {name} {{ {} }}\n", ts_fields(fields)),
            Shape::Enum(names) => out += &format!("export type {name} = {};\n", names.iter().map(|n| format!("\"{}\"", n.name)).collect::<Vec<_>>().join(" | ")),
            Shape::Alias(ty, _) => out += &format!("export type {name} = {};\n", ty_ts(ty)),
            Shape::Variant(cases) => {
                out += &format!("export type {name} =\n");
                for c in cases {
                    let val = match (&c.ty, &c.record) {
                        (Some(t), _) => format!("; val: {}", ty_ts(t)),
                        (None, Some(fields)) => format!("; val: {{ {} }}", ts_fields(fields)),
                        (None, None) => String::new(),
                    };
                    out += &format!("  | {{ tag: \"{}\"{val} }}\n", c.name);
                }
                out = out.trim_end().to_string() + ";\n";
            }
        }
    }
    // Each type to the wire and back.
    out += "\n";
    for t in &ir.types {
        let name = camel(&t.name);
        for dir in [true, false] {
            let (word, arg, ret) = if dir { ("to", format!("v: {name}"), "any".to_string()) } else { ("from", "j: any".to_string(), name.clone()) };
            let v = if dir { "v" } else { "j" };
            let body = match &t.shape {
                Shape::Record(fields) => format!("({})", conv_fields(fields, v, dir)),
                Shape::Enum(cases) => renamed(cases, v, dir),
                Shape::Alias(ty, _) => conv(ty, v, dir),
                Shape::Variant(cases) => {
                    let mut arms = String::new();
                    for c in cases {
                        // `w` is quoted, since a wire name may hold anything.
                        let (n, w) = (&c.name, Value::from(c.wire.as_deref().unwrap_or(&c.name)).to_string());
                        if dir {
                            arms += &match (&c.ty, &c.record) {
                                (Some(ty), _) => format!("v.tag === \"{n}\" ? {{ {w}: {} }} : ", conv(ty, "v.val", true)),
                                (None, Some(fields)) => format!("v.tag === \"{n}\" ? {{ {w}: {} }} : ", conv_fields(fields, "v.val", true)),
                                (None, None) => String::new(),
                            };
                        } else {
                            arms += &match (&c.ty, &c.record) {
                                (Some(ty), _) => format!("{w} in j ? {{ tag: \"{n}\", val: {} }} : ", conv(ty, &format!("j[{w}]"), false)),
                                (None, Some(fields)) => format!("{w} in j ? {{ tag: \"{n}\", val: {} }} : ", conv_fields(fields, &format!("j[{w}]"), false)),
                                (None, None) => String::new(),
                            };
                        }
                    }
                    let bare: Vec<Field> = cases.iter().filter(|c| c.ty.is_none() && c.record.is_none()).cloned().collect();
                    if dir { format!("{arms}{}", renamed(&bare, "(v as any).tag", true)) } else { format!("typeof j === \"string\" ? {{ tag: {} }} as {name} : {arms}j", renamed(&bare, "j", false)) }
                }
            };
            out += &format!("const {word}{name} = ({arg}): {ret} => {body};\n");
        }
    }
    let params_obj = |m: &Method, read: bool| -> String {
        let parts: Vec<String> = m.params.iter().map(|p| {
            let ty = p.ty.as_ref().expect("a parameter has a type");
            if read {
                let got = format!("p[\"{}\"]", p.name);
                match &p.default {
                    Some(d) => conv(ty, &format!("({got} ?? {d})"), false),
                    None => conv(ty, &got, false),
                }
            } else {
                format!("\"{}\": {}", p.name, conv(ty, &lower_camel(&p.name), true))
            }
        }).collect();
        if read { parts.join(", ") } else { format!("{{ {} }}", parts.join(", ")) }
    };
    let args = |m: &Method| m.params.iter().map(|p| format!("{}: {}", lower_camel(&p.name), ty_ts(p.ty.as_ref().expect("a parameter has a type")))).collect::<Vec<_>>().join(", ");
    let fail = |m: &Method, out: bool| m.fails.as_ref().map(|e| format!(", (e: any) => {}", conv(e, "e", out))).unwrap_or_default();

    out += &format!("\n/// The IR this file was generated from: what `_ir` answers.\nexport const {}Ir = {};\n", lower_camel(&ir.activation), ir.json_line());
    // The stub: what a caller holds.
    out += &format!("\n/// The caller's side of `{}`: every method, typed, sent as frames.\nexport class {svc}Client {{\n  /// The interface this client was generated from.\n  static readonly interface = \"{}\";\n", ir.activation, ir.hash());
    out += &format!("  private conn: Connection;\n  private at: string;\n  /// `at` is where the activation is mounted: its own name unless a parent holds it.\n  constructor(conn: Connection, at: string = \"{0}\") {{\n    this.conn = conn;\n    this.at = at;\n  }}\n  /// Ask what is served at `at`, once, and refuse if it is not this interface.\n  async bind(): Promise<this> {{\n    const served = await this.conn.call(this.at + \"._hash\", {{}});\n    if (served !== {1}Client.interface) throw new Error(`${{this.at}} serves interface ${{served}}; this client was generated from ${{{1}Client.interface}}`);\n    return this;\n  }}\n", ir.activation, svc);
    for m in &ir.methods {
        let (name, path) = (lower_camel(&m.name), format!("this.at + \".{}\"", m.name));
        out += &doc_lines(&m.doc, "  ", "///");
        match &m.returns {
            Returns::Stream(item) => out += &format!("  {name}({}): AsyncIterable<{}> {{\n    return each(this.conn.stream({path}, {}{}), (j: any) => {});\n  }}\n", args(m), ty_ts(item), params_obj(m, false), fail(m, false), conv(item, "j", false)),
            Returns::One(t) => out += &format!("  async {name}({}): Promise<{}> {{\n    const j = await this.conn.call({path}, {}{});\n    return {};\n  }}\n", args(m), ty_ts(t), params_obj(m, false), fail(m, false), conv(t, "j", false)),
            Returns::Nothing => out += &format!("  async {name}({}): Promise<void> {{\n    await this.conn.call({path}, {}{});\n  }}\n", args(m), params_obj(m, false), fail(m, false)),
        }
    }
    out += "}\n";

    // The skeleton: what the activation's side runs.
    out += &format!("\n/// What `{}` must offer to be served: a transpiled component does.\nexport interface {svc}Activation {{\n", ir.activation);
    for m in &ir.methods {
        let ret = match &m.returns {
            Returns::Stream(item) => format!("AsyncIterable<{0}> | Promise<AsyncIterable<{0}>>", ty_ts(item)),
            Returns::One(t) => format!("{0} | Promise<{0}>", match t { Ty::Option(_) => format!("({})", ty_ts(t)), _ => ty_ts(t) }),
            Returns::Nothing => "void | Promise<void>".into(),
        };
        out += &format!("  {}({}): {ret};\n", lower_camel(&m.name), args(m));
    }
    out += &format!("}}\n\n/// The activation's side: each `open` frame becomes a call into `activation`.\n/// `at` is where it is mounted: its own name unless a parent holds it.\nexport function serve{svc}(activation: {svc}Activation, server: Server, at: string = \"{}\"): void {{\n", ir.activation);
    for m in &ir.methods {
        let (name, path) = (lower_camel(&m.name), format!("at + \".{}\"", m.name));
        let call = format!("activation.{name}({})", params_obj(m, true));
        match &m.returns {
            Returns::Stream(item) => out += &format!("  server.stream({path}, (p: any) => each({call}, (v: any) => {}){});\n", conv(item, "v", true), fail(m, true)),
            Returns::One(t) if conv(t, "v", true) == "v" => out += &format!("  server.method({path}, (p: any) => {call}{});\n", fail(m, true)),
            Returns::One(t) => out += &format!("  server.method({path}, async (p: any) => {{ const v = await {call}; return {}; }}{});\n", conv(t, "v", true), fail(m, true)),
            Returns::Nothing => out += &format!("  server.method({path}, async (p: any) => {{ await {call}; }}{});\n", fail(m, true)),
        }
    }
    out += &format!("  server.describe(at, {}Ir);\n}}\n", lower_camel(&ir.activation));
    out
}
