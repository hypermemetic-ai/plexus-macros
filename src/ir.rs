//! The IR: what an activation is, as data. Everything else is written from it.
//!
//! Names in the IR are kebab-case (`loaded-by`, `kiln-error`); each target
//! respells them its own way.

use sha2::{Digest, Sha256};

/// A type, by reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// `string`, `bool`, `u8`..`u64`, `s8`..`s64`, `f32`, `f64`, `char`,
    /// `bytes` (raw octets) and `json` (any JSON value).
    Prim(&'static str),
    /// A type of this activation, by name.
    Named(String),
    List(Box<Ty>),
    Option(Box<Ty>),
    Tuple(Vec<Ty>),
    /// A map keyed by strings.
    Map(Box<Ty>),
}

#[derive(Debug, Clone)]
pub struct Deprecated {
    pub since: String,
    pub note: String,
}

/// A field of a record, a case of a variant or an enum, or a parameter.
#[derive(Debug, Clone, Default)]
pub struct Field {
    pub name: String,
    pub doc: String,
    /// `None` for a case that carries nothing, or one that carries a record.
    pub ty: Option<Ty>,
    /// A case's own fields, when it carries a record.
    pub record: Option<Vec<Field>>,
    /// What an absent value is taken as: a JSON literal.
    pub default: Option<String>,
    pub deprecated: Option<Deprecated>,
}

#[derive(Debug, Clone)]
pub enum Shape {
    Record(Vec<Field>),
    /// One of several cases, each carrying nothing, one value or a record.
    Variant(Vec<Field>),
    /// One of several names.
    Enum(Vec<Field>),
    /// Another name for a type, and the limits its values keep: each a
    /// name (`min`, `max`, `min-len`, `max-len`, `pattern`) and a JSON literal.
    Alias(Ty, Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub doc: String,
    pub shape: Shape,
}

/// What a method gives back.
#[derive(Debug, Clone)]
pub enum Returns {
    Nothing,
    One(Ty),
    /// Many, one at a time, until there are no more.
    Stream(Ty),
}

#[derive(Debug, Clone)]
pub struct Method {
    pub name: String,
    pub doc: String,
    pub params: Vec<Field>,
    pub returns: Returns,
    /// The error a call can end with, when it has one.
    pub fails: Option<Ty>,
    /// The items the caller streams in while the call runs.
    pub input: Option<Ty>,
    /// What the method reports while the caller waits.
    pub progress: Option<Ty>,
    /// `required` or `optional`, when the method reads who is calling.
    pub caller: Option<&'static str>,
    /// The grant a caller must hold.
    pub grant: Option<String>,
    /// The parameter naming what the grant is held on.
    pub target: Option<String>,
    /// Written `async fn`, or returns a stream. Not in the IR's JSON: over
    /// a wire every call is awaited. WIT says it, because a caller in the
    /// same process gets a promise or a plain value.
    pub waits: bool,
}

/// A named set of grants.
#[derive(Debug, Clone)]
pub struct Role {
    pub name: String,
    /// Roles whose grants this one also holds.
    pub includes: Vec<String>,
    pub grants: Vec<String>,
}

/// An activation held under this one.
#[derive(Debug, Clone)]
pub struct Child {
    pub name: String,
    pub doc: String,
    /// The child's own interface, by name.
    pub activation: String,
    /// `activation` names an activation written here (`Fixed`), an interface any
    /// activation may fit (`Interface`), or nothing: any activation at all (`Any`).
    pub held: Held,
    /// For one child per key: the key.
    pub key: Option<Field>,
    /// The method that lists the keys, and the one that searches them.
    pub list: Option<String>,
    pub search: Option<String>,
    pub fails: Option<Ty>,
}

/// What is known, when the parent is compiled, of a child it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Held {
    Fixed,
    Interface,
    Any,
}

#[derive(Debug, Clone)]
pub struct Ir {
    pub activation: String,
    pub doc: String,
    pub types: Vec<TypeDef>,
    pub methods: Vec<Method>,
    pub children: Vec<Child>,
    pub roles: Vec<Role>,
}

// ---------------------------------------------------------------- JSON

/// JSON, kept in the order it was built.
enum J {
    Raw(String),
    Arr(Vec<J>),
    Obj(Vec<(&'static str, J)>),
}

fn s(text: &str) -> J {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    J::Raw(out + "\"")
}

impl J {
    fn flat(&self) -> String {
        match self {
            J::Raw(r) => r.clone(),
            J::Arr(a) => format!("[{}]", a.iter().map(J::flat).collect::<Vec<_>>().join(", ")),
            J::Obj(o) => format!("{{{}}}", o.iter().map(|(k, v)| format!("\"{k}\": {}", v.flat())).collect::<Vec<_>>().join(", ")),
        }
    }
    /// One line when it fits in 96 columns, else one entry a line.
    fn pretty(&self, indent: usize) -> String {
        let flat = self.flat();
        if flat.len() + indent <= 96 {
            return flat;
        }
        let pad = " ".repeat(indent + 2);
        let end = " ".repeat(indent);
        match self {
            J::Raw(r) => r.clone(),
            J::Arr(a) => format!("[\n{}\n{end}]", a.iter().map(|v| format!("{pad}{}", v.pretty(indent + 2))).collect::<Vec<_>>().join(",\n")),
            J::Obj(o) => format!("{{\n{}\n{end}}}", o.iter().map(|(k, v)| format!("{pad}\"{k}\": {}", v.pretty(indent + 2))).collect::<Vec<_>>().join(",\n")),
        }
    }
}

fn ty_json(t: &Ty) -> J {
    match t {
        Ty::Prim(p) => s(p),
        Ty::Named(n) => s(n),
        Ty::List(t) => J::Obj(vec![("list", ty_json(t))]),
        Ty::Option(t) => J::Obj(vec![("option", ty_json(t))]),
        Ty::Map(t) => J::Obj(vec![("map", ty_json(t))]),
        Ty::Tuple(ts) => J::Obj(vec![("tuple", J::Arr(ts.iter().map(ty_json).collect()))]),
    }
}

fn field_json(f: &Field) -> J {
    let mut o = vec![("name", s(&f.name))];
    if let Some(t) = &f.ty {
        o.push(("type", ty_json(t)));
    }
    if let Some(r) = &f.record {
        o.push(("record", J::Arr(r.iter().map(field_json).collect())));
    }
    if let Some(d) = &f.default {
        o.push(("default", J::Raw(d.clone())));
    }
    if let Some(d) = &f.deprecated {
        o.push(("deprecated", J::Obj(vec![("since", s(&d.since)), ("note", s(&d.note))])));
    }
    if !f.doc.is_empty() {
        o.push(("doc", s(&f.doc)));
    }
    J::Obj(o)
}

impl Ir {
    fn tree(&self, hash: Option<&str>) -> J {
        let mut o = vec![("ir", s("plexus/0")), ("activation", s(&self.activation))];
        if let Some(h) = hash {
            o.push(("hash", s(h)));
        }
        if !self.doc.is_empty() {
            o.push(("doc", s(&self.doc)));
        }
        if !self.roles.is_empty() {
            o.push(("roles", J::Arr(self.roles.iter().map(|r| {
                let mut o = vec![("name", s(&r.name))];
                if !r.includes.is_empty() {
                    o.push(("includes", J::Arr(r.includes.iter().map(|i| s(i)).collect())));
                }
                o.push(("grants", J::Arr(r.grants.iter().map(|g| s(g)).collect())));
                J::Obj(o)
            }).collect())));
        }
        o.push(("types", J::Arr(self.types.iter().map(|t| {
            let mut o = vec![("name", s(&t.name))];
            if !t.doc.is_empty() {
                o.push(("doc", s(&t.doc)));
            }
            match &t.shape {
                Shape::Record(f) => o.push(("record", J::Arr(f.iter().map(field_json).collect()))),
                Shape::Variant(f) => o.push(("variant", J::Arr(f.iter().map(field_json).collect()))),
                Shape::Enum(f) => o.push(("enum", J::Arr(f.iter().map(field_json).collect()))),
                Shape::Alias(ty, limits) => {
                    o.push(("alias", ty_json(ty)));
                    if !limits.is_empty() {
                        o.push(("limits", J::Raw(format!("{{{}}}", limits.iter().map(|(k, v)| format!("\"{k}\": {v}")).collect::<Vec<_>>().join(", ")))));
                    }
                }
            }
            J::Obj(o)
        }).collect())));
        o.push(("methods", J::Arr(self.methods.iter().map(|m| {
            let mut o = vec![("name", s(&m.name))];
            if !m.doc.is_empty() {
                o.push(("doc", s(&m.doc)));
            }
            if let Some(c) = m.caller {
                o.push(("caller", s(c)));
            }
            if let Some(g) = &m.grant {
                o.push(("grant", s(g)));
            }
            if let Some(t) = &m.target {
                o.push(("target", s(t)));
            }
            o.push(("params", J::Arr(m.params.iter().map(field_json).collect())));
            if let Some(i) = &m.input {
                o.push(("input", ty_json(i)));
            }
            if let Some(p) = &m.progress {
                o.push(("progress", ty_json(p)));
            }
            match &m.returns {
                Returns::Nothing => {}
                Returns::One(t) => o.push(("returns", ty_json(t))),
                Returns::Stream(t) => o.push(("stream", ty_json(t))),
            }
            if let Some(e) = &m.fails {
                o.push(("fails", ty_json(e)));
            }
            J::Obj(o)
        }).collect())));
        if !self.children.is_empty() {
            o.push(("children", J::Arr(self.children.iter().map(|c| {
                let mut o = vec![("name", s(&c.name))];
                match c.held {
                    Held::Fixed => o.push(("activation", s(&c.activation))),
                    Held::Interface => o.push(("interface", s(&c.activation))),
                    Held::Any => o.push(("any", J::Raw("true".into()))),
                }
                if let Some(k) = &c.key {
                    o.push(("key", field_json(k)));
                }
                if let Some(l) = &c.list {
                    o.push(("list", s(l)));
                }
                if let Some(l) = &c.search {
                    o.push(("search", s(l)));
                }
                if let Some(e) = &c.fails {
                    o.push(("fails", ty_json(e)));
                }
                if !c.doc.is_empty() {
                    o.push(("doc", s(&c.doc)));
                }
                J::Obj(o)
            }).collect())));
        }
        J::Obj(o)
    }

    /// The interface's hash: the first 16 hex digits of the SHA-256 of the
    /// IR written on one line without its hash. It changes when anything a
    /// caller can see changes.
    pub fn hash(&self) -> String {
        let digest = Sha256::digest(self.tree(None).flat().as_bytes());
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    /// The IR on one line.
    #[cfg(test)]
    pub fn json_flat(&self) -> String {
        self.tree(Some(&self.hash())).flat()
    }

    /// The IR as JSON: what a tool reads.
    pub fn json(&self) -> String {
        self.tree(Some(&self.hash())).pretty(0) + "\n"
    }
}

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

impl Ir {
    fn every_type(&self) -> Vec<&Ty> {
        let mut out = Vec::new();
        fn fields<'a>(fs: &'a [Field], out: &mut Vec<&'a Ty>) {
            for f in fs {
                out.extend(f.ty.as_ref());
                if let Some(r) = &f.record {
                    fields(r, out);
                }
            }
        }
        for t in &self.types {
            match &t.shape {
                Shape::Record(f) | Shape::Variant(f) | Shape::Enum(f) => fields(f, &mut out),
                Shape::Alias(ty, _) => out.push(ty),
            }
        }
        for m in &self.methods {
            fields(&m.params, &mut out);
            match &m.returns {
                Returns::One(t) | Returns::Stream(t) => out.push(t),
                Returns::Nothing => {}
            }
            out.extend(m.fails.as_ref());
            out.extend(m.input.as_ref());
            out.extend(m.progress.as_ref());
        }
        out
    }

    /// What the IR says that `target` has no word for, one line each.
    fn unsaid(&self, target: &str) -> Vec<String> {
        let mut out = Vec::new();
        let any = |f: &dyn Fn(&Method) -> bool| self.methods.iter().any(|m| f(m));
        let fields_any = |f: &dyn Fn(&Field) -> bool| {
            self.methods.iter().any(|m| m.params.iter().any(|p| f(p)))
                || self.types.iter().any(|t| matches!(&t.shape, Shape::Record(fs) | Shape::Variant(fs) if fs.iter().any(|x| f(x) || x.record.as_ref().is_some_and(|r| r.iter().any(|y| f(y))))))
        };
        if any(&|m| m.caller.is_some()) {
            out.push("which methods read the caller".into());
        }
        if any(&|m| m.grant.is_some()) || !self.roles.is_empty() {
            out.push("grants, targets and roles".into());
        }
        if !self.children.is_empty() {
            out.push("the children".into());
        }
        if any(&|m| m.progress.is_some()) {
            out.push("progress".into());
        }
        if fields_any(&|f| f.default.is_some()) {
            out.push("defaults".into());
        }
        if self.types.iter().any(|t| matches!(&t.shape, Shape::Alias(_, l) if !l.is_empty())) {
            out.push("limits on a value".into());
        }
        let ts = self.every_type();
        if target == "wit" {
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
        } else {
            if ts.iter().any(|t| mentions(t, "u8") || mentions(t, "u16") || mentions(t, "s8") || mentions(t, "s16")) {
                out.push("integers under 32 bits (sent as 32)".into());
            }
            if self.types.iter().any(|t| matches!(t.shape, Shape::Alias(..))) {
                out.push("an alias's name (its type is written in place)".into());
            }
        }
        out
    }

    /// The IR as a WIT package: one interface, one world that exports it.
    /// A stream out is WIT's own `stream<T>`.
    pub fn wit(&self) -> String {
        let name = &self.activation;
        let unsaid = self.unsaid("wit");
        let note = if unsaid.is_empty() { String::new() } else { format!("// In the IR and not in WIT: {}.\n", unsaid.join("; ")) };
        let mut out = format!("package plexus:{name}@0.0.1;\n\n{note}{}interface {name} {{\n", doc_lines(&self.doc, "", "///"));
        for t in &self.types {
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
        for m in &self.methods {
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
}

// --------------------------------------------------------------- proto

fn camel(kebab: &str) -> String {
    kebab.split('-').map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }).collect()
}
fn snake(kebab: &str) -> String {
    kebab.replace('-', "_")
}

impl Ir {
    /// An alias is written as the type it names.
    fn resolve<'a>(&'a self, t: &'a Ty) -> &'a Ty {
        if let Ty::Named(n) = t {
            if let Some(TypeDef { shape: Shape::Alias(inner, _), .. }) = self.types.iter().find(|d| &d.name == n) {
                return self.resolve(inner);
            }
        }
        t
    }

    /// A type as a protobuf field: its label (`repeated `, `optional ` or
    /// none) and its type.
    fn ty_proto(&self, t: &Ty) -> (&'static str, String) {
        let scalar = |t: &Ty| -> String {
            match self.resolve(t) {
                Ty::Prim("string") | Ty::Prim("char") => "string".into(),
                Ty::Prim("bool") => "bool".into(),
                Ty::Prim("u8") | Ty::Prim("u16") | Ty::Prim("u32") => "uint32".into(),
                Ty::Prim("u64") => "uint64".into(),
                Ty::Prim("s8") | Ty::Prim("s16") | Ty::Prim("s32") => "sint32".into(),
                Ty::Prim("s64") => "sint64".into(),
                Ty::Prim("f32") => "float".into(),
                Ty::Prim("f64") => "double".into(),
                Ty::Prim("bytes") => "bytes".into(),
                Ty::Prim("json") => "google.protobuf.Value".into(),
                Ty::Named(n) => camel(n),
                Ty::Map(v) => format!("map<string, {}>", self.ty_proto(v).1),
                // A list of lists, an option of a list, a tuple: no one field says it.
                _ => "google.protobuf.Value".into(),
            }
        };
        match self.resolve(t) {
            Ty::List(inner) if matches!(self.resolve(inner), Ty::Prim(_) | Ty::Named(_)) => ("repeated ", scalar(inner)),
            Ty::Option(inner) if matches!(self.resolve(inner), Ty::Prim(_) | Ty::Named(_)) => ("optional ", scalar(inner)),
            other => ("", scalar(other)),
        }
    }

    fn proto_fields(&self, fields: &[Field], indent: &str) -> String {
        let mut out = String::new();
        for (i, f) in fields.iter().enumerate() {
            let (label, ty) = self.ty_proto(f.ty.as_ref().expect("a field has a type"));
            out += &doc_lines(&f.doc, indent, "//");
            let dep = if f.deprecated.is_some() { " [deprecated = true]" } else { "" };
            out += &format!("{indent}{label}{ty} {} = {}{dep};\n", snake(&f.name), i + 1);
        }
        out
    }

    /// The IR as a proto3 file with a gRPC service. Protobuf has no
    /// `result`, so a method that can fail answers a message with a `oneof`.
    pub fn proto(&self) -> String {
        let svc = camel(&self.activation);
        let mut messages = String::new();
        for t in &self.types {
            match &t.shape {
                Shape::Alias(..) => continue,
                Shape::Record(fields) => {
                    messages += &doc_lines(&t.doc, "", "//");
                    messages += &format!("message {} {{\n{}}}\n\n", camel(&t.name), self.proto_fields(fields, "  "));
                }
                Shape::Variant(cases) => {
                    messages += &doc_lines(&t.doc, "", "//");
                    messages += &format!("message {} {{\n", camel(&t.name));
                    for c in cases {
                        if let Some(fields) = &c.record {
                            messages += &format!("  message {} {{\n{}  }}\n", camel(&c.name), self.proto_fields(fields, "    "));
                        }
                    }
                    messages += "  oneof case {\n";
                    for (i, c) in cases.iter().enumerate() {
                        messages += &doc_lines(&c.doc, "    ", "//");
                        let ty = match (&c.ty, &c.record) {
                            (Some(t), _) => self.ty_proto(t).1,
                            (None, Some(_)) => camel(&c.name),
                            (None, None) => "google.protobuf.Empty".into(),
                        };
                        messages += &format!("    {ty} {} = {};\n", snake(&c.name), i + 1);
                    }
                    messages += "  }\n}\n\n";
                }
                Shape::Enum(names) => {
                    messages += &doc_lines(&t.doc, "", "//");
                    let prefix = snake(&t.name).to_uppercase();
                    messages += &format!("enum {} {{\n", camel(&t.name));
                    for (i, n) in names.iter().enumerate() {
                        messages += &doc_lines(&n.doc, "  ", "//");
                        messages += &format!("  {prefix}_{} = {i};\n", snake(&n.name).to_uppercase());
                    }
                    messages += "}\n\n";
                }
            }
        }
        let mut rpcs = String::new();
        for m in &self.methods {
            let name = camel(&m.name);
            messages += &format!("message {name}Request {{\n{}}}\n\n", self.proto_fields(&m.params, "  "));
            let asks = match &m.input {
                Some(item) => {
                    messages += &format!("// The call opens with `open`; every message after it carries one `item`.\nmessage {name}Upload {{\n  oneof part {{\n    {name}Request open = 1;\n    {} item = 2;\n  }}\n}}\n\n", self.ty_proto(item).1);
                    format!("stream {name}Upload")
                }
                None => format!("{name}Request"),
            };
            let field = |t: &Ty, name: &str| {
                let (label, ty) = self.ty_proto(t);
                format!("{label}{ty} {name} = 1;")
            };
            let answer = match (&m.returns, &m.fails) {
                (Returns::Stream(Ty::Named(item)), _) if !matches!(self.resolve(&Ty::Named(item.clone())), Ty::Prim(_)) => format!("stream {}", camel(item)),
                (Returns::Stream(item), _) => {
                    messages += &format!("message {name}Item {{\n  {}\n}}\n\n", field(item, "value"));
                    format!("stream {name}Item")
                }
                (ret, fails) => {
                    messages += &format!("message {name}Response {{\n");
                    match (ret, fails) {
                        (Returns::One(t), None) => messages += &format!("  {}\n", field(t, "value")),
                        (ret, Some(e)) => {
                            let ok = match ret {
                                Returns::One(t) => match self.ty_proto(t) {
                                    ("", ty) => ty,
                                    // `repeated` and `optional` cannot sit in a oneof.
                                    _ => "google.protobuf.Value".into(),
                                },
                                _ => "google.protobuf.Empty".into(),
                            };
                            messages += &format!("  oneof result {{\n    {ok} ok = 1;\n    {} err = 2;\n  }}\n", self.ty_proto(e).1);
                        }
                        _ => {}
                    }
                    messages += "}\n\n";
                    format!("{name}Response")
                }
            };
            rpcs += &doc_lines(&m.doc, "  ", "//");
            rpcs += &format!("  rpc {name}({asks}) returns ({answer});\n");
        }
        let mut imports = String::new();
        for (word, file) in [("google.protobuf.Empty", "empty"), ("google.protobuf.Value", "struct")] {
            if messages.contains(word) {
                imports += &format!("import \"google/protobuf/{file}.proto\";\n");
            }
        }
        if !imports.is_empty() {
            imports += "\n";
        }
        let unsaid = self.unsaid("proto");
        let note = if unsaid.is_empty() { String::new() } else { format!("// In the IR and not in protobuf: {}.\n", unsaid.join("; ")) };
        let (pkg, doc) = (snake(&self.activation), doc_lines(&self.doc, "", "//"));
        let messages = messages.trim_end();
        format!("syntax = \"proto3\";\n\npackage plexus.{pkg}.v0;\n\n{imports}{note}{doc}service {svc} {{\n{rpcs}}}\n\n{messages}\n")
    }
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

fn ts_fields(fields: &[Field]) -> String {
    fields.iter().map(|f| {
        let ty = f.ty.as_ref().expect("a field has a type");
        match ty {
            Ty::Option(inner) => format!("{}?: {}", lower_camel(&f.name), ty_ts(inner)),
            _ => format!("{}: {}", lower_camel(&f.name), ty_ts(ty)),
        }
    }).collect::<Vec<_>>().join("; ")
}

impl Ir {
    /// A typed client and the activation's side of the same calls, as one
    /// TypeScript module. Both speak frames over a `Channel`.
    ///
    /// On the wire a record is an object keyed by the IR's names; an enum
    /// is its name; a variant's case is `{"case": value}`, or its bare name
    /// when it carries nothing.
    pub fn ts(&self) -> String {
        let svc = camel(&self.activation);
        let mut out = format!(
            "// Generated from the IR of `{}` (interface {}). Not edited by hand.\n\nimport {{ Connection, Server, each, b64, unb64 }} from \"./plexus-rt.ts\";\n\n",
            self.activation, self.hash()
        );
        if !self.every_type().iter().any(|t| mentions(t, "bytes")) {
            out = out.replace(", b64, unb64", "");
        }
        // The types, in the shapes a transpiled component uses.
        for t in &self.types {
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
        for t in &self.types {
            let name = camel(&t.name);
            for dir in [true, false] {
                let (word, arg, ret) = if dir { ("to", format!("v: {name}"), "any".to_string()) } else { ("from", "j: any".to_string(), name.clone()) };
                let v = if dir { "v" } else { "j" };
                let body = match &t.shape {
                    Shape::Record(fields) => format!("({})", conv_fields(fields, v, dir)),
                    Shape::Enum(_) => v.to_string(),
                    Shape::Alias(ty, _) => conv(ty, v, dir),
                    Shape::Variant(cases) => {
                        let mut arms = String::new();
                        for c in cases {
                            let n = &c.name;
                            if dir {
                                arms += &match (&c.ty, &c.record) {
                                    (Some(ty), _) => format!("v.tag === \"{n}\" ? {{ \"{n}\": {} }} : ", conv(ty, "v.val", true)),
                                    (None, Some(fields)) => format!("v.tag === \"{n}\" ? {{ \"{n}\": {} }} : ", conv_fields(fields, "v.val", true)),
                                    (None, None) => String::new(),
                                };
                            } else {
                                arms += &match (&c.ty, &c.record) {
                                    (Some(ty), _) => format!("\"{n}\" in j ? {{ tag: \"{n}\", val: {} }} : ", conv(ty, &format!("j[\"{n}\"]"), false)),
                                    (None, Some(fields)) => format!("\"{n}\" in j ? {{ tag: \"{n}\", val: {} }} : ", conv_fields(fields, &format!("j[\"{n}\"]"), false)),
                                    (None, None) => String::new(),
                                };
                            }
                        }
                        if dir { format!("{arms}(v as any).tag") } else { format!("typeof j === \"string\" ? {{ tag: j }} as {name} : {arms}j") }
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

        out += &format!("\n/// The IR this file was generated from: what `_ir` answers.\nexport const {}Ir = {};\n", lower_camel(&self.activation), self.tree(Some(&self.hash())).flat());
        // The stub: what a caller holds.
        out += &format!("\n/// The caller's side of `{}`: every method, typed, sent as frames.\nexport class {svc}Client {{\n  /// The interface this client was generated from.\n  static readonly interface = \"{}\";\n", self.activation, self.hash());
        out += &format!("  private conn: Connection;\n  private at: string;\n  /// `at` is where the activation is mounted: its own name unless a parent holds it.\n  constructor(conn: Connection, at: string = \"{0}\") {{\n    this.conn = conn;\n    this.at = at;\n  }}\n  /// Ask what is served at `at`, once, and refuse if it is not this interface.\n  async bind(): Promise<this> {{\n    const served = await this.conn.call(this.at + \"._hash\", {{}});\n    if (served !== {1}Client.interface) throw new Error(`${{this.at}} serves interface ${{served}}; this client was generated from ${{{1}Client.interface}}`);\n    return this;\n  }}\n", self.activation, svc);
        for m in &self.methods {
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
        out += &format!("\n/// What `{}` must offer to be served: a transpiled component does.\nexport interface {svc}Activation {{\n", self.activation);
        for m in &self.methods {
            let ret = match &m.returns {
                Returns::Stream(item) => format!("AsyncIterable<{0}> | Promise<AsyncIterable<{0}>>", ty_ts(item)),
                Returns::One(t) => format!("{0} | Promise<{0}>", match t { Ty::Option(_) => format!("({})", ty_ts(t)), _ => ty_ts(t) }),
                Returns::Nothing => "void | Promise<void>".into(),
            };
            out += &format!("  {}({}): {ret};\n", lower_camel(&m.name), args(m));
        }
        out += &format!("}}\n\n/// The activation's side: each `open` frame becomes a call into `activation`.\n/// `at` is where it is mounted: its own name unless a parent holds it.\nexport function serve{svc}(activation: {svc}Activation, server: Server, at: string = \"{}\"): void {{\n", self.activation);
        for m in &self.methods {
            let (name, path) = (lower_camel(&m.name), format!("at + \".{}\"", m.name));
            let call = format!("activation.{name}({})", params_obj(m, true));
            match &m.returns {
                Returns::Stream(item) => out += &format!("  server.stream({path}, (p: any) => each({call}, (v: any) => {}){});\n", conv(item, "v", true), fail(m, true)),
                Returns::One(t) if conv(t, "v", true) == "v" => out += &format!("  server.method({path}, (p: any) => {call}{});\n", fail(m, true)),
                Returns::One(t) => out += &format!("  server.method({path}, async (p: any) => {{ const v = await {call}; return {}; }}{});\n", conv(t, "v", true), fail(m, true)),
                Returns::Nothing => out += &format!("  server.method({path}, async (p: any) => {{ await {call}; }}{});\n", fail(m, true)),
            }
        }
        out += &format!("  server.describe(at, {}Ir);\n}}\n", lower_camel(&self.activation));
        out
    }
}
