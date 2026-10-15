//! Loader and validator for the protocol schema subset.
//!
//! The subset is defined in `docs/contracts/autospec-protocol-v1.md` (Schema
//! subset). The loader compiles each schema file into a small [`Node`] tree and
//! fails closed: a keyword outside the subset is an error naming the file and
//! the keyword, never an ignored rule. `$ref` is resolved at load time, so the
//! validator itself never follows a reference.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use super::ProtocolError;

/// The only keywords a protocol schema may use.
const SUBSET: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "const",
    "items",
    "$ref",
    "minimum",
    "default",
    "title",
    "description",
    "$schema",
    "$id",
    "$defs",
];

/// Keywords that annotate a node and may sit beside a `$ref`.
const ANNOTATIONS: &[&str] = &["title", "description", "default"];

const FILE_PREFIX: &str = "autospec-protocol-";
const FILE_SUFFIX: &str = ".v1.schema.json";

/// One compiled schema node.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// `type: object` with named members; `closed` is `additionalProperties: false`.
    Object {
        properties: BTreeMap<String, Node>,
        required: Vec<String>,
        closed: bool,
    },
    /// `type: object` whose `additionalProperties` is a schema for every value.
    Map(Box<Node>),
    Array(Box<Node>),
    Str {
        enumeration: Option<Vec<String>>,
        constant: Option<String>,
    },
    Integer {
        minimum: Option<f64>,
    },
    Number {
        minimum: Option<f64>,
    },
    Boolean,
}

/// One failed rule: where (JSON pointer) and which keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub pointer: String,
    pub rule: String,
}

/// The compiled protocol schemas, keyed by entity name (`envelope`, `event`, ...).
#[derive(Debug)]
pub struct SchemaSet {
    entities: BTreeMap<String, Node>,
}

/// Load every `autospec-protocol-<entity>.v1.schema.json` in `dir`.
///
/// Other files in the directory are ignored. Any file that fails to compile
/// fails the whole load.
// linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
pub fn load_dir(dir: &Path) -> Result<SchemaSet, ProtocolError> {
    let load_err = |file: String, detail: String| ProtocolError::SchemaLoad { file, detail };
    let listing = std::fs::read_dir(dir).map_err(|e| {
        load_err(
            dir.display().to_string(),
            format!("cannot list directory: {e}"),
        )
    })?;
    let mut files: Vec<(String, String)> = Vec::new();
    for entry in listing {
        let entry =
            entry.map_err(|e| load_err(dir.display().to_string(), format!("cannot list: {e}")))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(entity) = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|rest| rest.strip_suffix(FILE_SUFFIX))
        {
            files.push((entity.to_string(), name));
        }
    }
    files.sort();
    let mut entities = BTreeMap::new();
    for (entity, name) in files {
        let text = std::fs::read_to_string(dir.join(&name))
            .map_err(|e| load_err(name.clone(), format!("cannot read: {e}")))?;
        let root: Value = serde_json::from_str(&text)
            .map_err(|e| load_err(name.clone(), format!("not valid JSON: {e}")))?;
        entities.insert(entity, compile_root(&name, &root)?);
    }
    Ok(SchemaSet { entities })
}

impl SchemaSet {
    /// Entity names, sorted.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn entities(&self) -> Vec<&str> {
        self.entities.keys().map(String::as_str).collect()
    }

    /// The compiled root node of an entity.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn node(&self, entity: &str) -> Option<&Node> {
        self.entities.get(entity)
    }

    /// Every violation of `value` against the entity's schema, not just the first.
    // linter:allow-UNWIRED_PUB_ITEM deliberate staging: the Rust protocol library lands first; its callers arrive in later T08 issues
    pub fn validate(&self, entity: &str, value: &Value) -> Vec<Violation> {
        let mut out = Vec::new();
        match self.entities.get(entity) {
            Some(node) => check(node, value, "", &mut out),
            None => out.push(violation("", "unknown-entity")),
        }
        out
    }
}

fn violation(pointer: &str, rule: &str) -> Violation {
    Violation {
        pointer: pointer.to_string(),
        rule: rule.to_string(),
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn check(node: &Node, value: &Value, ptr: &str, out: &mut Vec<Violation>) {
    match node {
        Node::Object {
            properties,
            required,
            closed,
        } => {
            let Some(obj) = value.as_object() else {
                out.push(violation(ptr, "type"));
                return;
            };
            for key in required {
                if !obj.contains_key(key) {
                    out.push(violation(&format!("{ptr}/{}", escape(key)), "required"));
                }
            }
            for (key, member) in obj {
                let child = format!("{ptr}/{}", escape(key));
                match properties.get(key) {
                    Some(sub) => check(sub, member, &child, out),
                    None if *closed => out.push(violation(&child, "additionalProperties")),
                    None => {}
                }
            }
        }
        Node::Map(sub) => {
            let Some(obj) = value.as_object() else {
                out.push(violation(ptr, "type"));
                return;
            };
            for (key, member) in obj {
                check(sub, member, &format!("{ptr}/{}", escape(key)), out);
            }
        }
        Node::Array(sub) => {
            let Some(items) = value.as_array() else {
                out.push(violation(ptr, "type"));
                return;
            };
            for (i, item) in items.iter().enumerate() {
                check(sub, item, &format!("{ptr}/{i}"), out);
            }
        }
        Node::Str {
            enumeration,
            constant,
        } => {
            let Some(s) = value.as_str() else {
                out.push(violation(ptr, "type"));
                return;
            };
            if let Some(allowed) = enumeration {
                if !allowed.iter().any(|a| a == s) {
                    out.push(violation(ptr, "enum"));
                }
            }
            if let Some(expected) = constant {
                if expected != s {
                    out.push(violation(ptr, "const"));
                }
            }
        }
        Node::Integer { minimum } => {
            let number = value.as_f64().filter(|n| n.is_finite());
            let is_integer =
                value.is_i64() || value.is_u64() || number.is_some_and(|n| n.fract() == 0.0);
            match number {
                Some(n) if is_integer => {
                    if minimum.is_some_and(|m| n < m) {
                        out.push(violation(ptr, "minimum"));
                    }
                }
                _ => out.push(violation(ptr, "type")),
            }
        }
        Node::Number { minimum } => match value.as_f64() {
            Some(n) => {
                if minimum.is_some_and(|m| n < m) {
                    out.push(violation(ptr, "minimum"));
                }
            }
            None => out.push(violation(ptr, "type")),
        },
        Node::Boolean => {
            if !value.is_boolean() {
                out.push(violation(ptr, "type"));
            }
        }
    }
}

struct Compiler<'a> {
    file: &'a str,
    defs: &'a Map<String, Value>,
    /// `$defs` names being expanded, to refuse a reference cycle.
    stack: Vec<String>,
}

impl Compiler<'_> {
    fn load(&self, detail: impl Into<String>) -> ProtocolError {
        ProtocolError::SchemaLoad {
            file: self.file.to_string(),
            detail: detail.into(),
        }
    }

    fn unsupported(&self, keyword: &str) -> ProtocolError {
        ProtocolError::UnsupportedSchemaKeyword {
            file: self.file.to_string(),
            keyword: keyword.to_string(),
        }
    }

    fn compile(&mut self, value: &Value, at: &str, root: bool) -> Result<Node, ProtocolError> {
        let obj = value
            .as_object()
            .ok_or_else(|| self.load(format!("{at}: a schema node must be an object")))?;
        for key in obj.keys() {
            let root_only = matches!(key.as_str(), "$schema" | "$id" | "$defs");
            if !SUBSET.contains(&key.as_str()) || (root_only && !root) {
                return Err(self.unsupported(key));
            }
        }
        if let Some(reference) = obj.get("$ref") {
            return self.reference(obj, reference, at);
        }
        let kind = obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| self.load(format!("{at}: `type` must be one string")))?;
        let only = |keys: &[&str]| -> Result<(), ProtocolError> {
            for key in [
                "properties",
                "required",
                "additionalProperties",
                "items",
                "enum",
                "const",
                "minimum",
            ] {
                if obj.contains_key(key) && !keys.contains(&key) {
                    return Err(self.load(format!("{at}: `{key}` does not apply to type `{kind}`")));
                }
            }
            Ok(())
        };
        match kind {
            "object" => {
                only(&["properties", "required", "additionalProperties"])?;
                self.object(obj, at)
            }
            "array" => {
                only(&["items"])?;
                let items = obj
                    .get("items")
                    .ok_or_else(|| self.load(format!("{at}: an array needs `items`")))?;
                Ok(Node::Array(Box::new(self.compile(
                    items,
                    &format!("{at}/items"),
                    false,
                )?)))
            }
            "string" => {
                only(&["enum", "const"])?;
                let enumeration = match obj.get("enum") {
                    None => None,
                    Some(list) => Some(self.strings(list, at, "enum")?),
                };
                let constant = match obj.get("const") {
                    None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(_) => return Err(self.load(format!("{at}: `const` must be a string"))),
                };
                Ok(Node::Str {
                    enumeration,
                    constant,
                })
            }
            "integer" | "number" => {
                only(&["minimum"])?;
                let minimum =
                    match obj.get("minimum") {
                        None => None,
                        Some(m) => Some(m.as_f64().ok_or_else(|| {
                            self.load(format!("{at}: `minimum` must be a number"))
                        })?),
                    };
                Ok(if kind == "integer" {
                    Node::Integer { minimum }
                } else {
                    Node::Number { minimum }
                })
            }
            "boolean" => {
                only(&[])?;
                Ok(Node::Boolean)
            }
            other => Err(self.load(format!("{at}: type `{other}` is not in the subset"))),
        }
    }

    fn strings(&self, list: &Value, at: &str, keyword: &str) -> Result<Vec<String>, ProtocolError> {
        list.as_array()
            .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect())
            .ok_or_else(|| self.load(format!("{at}: `{keyword}` must be an array of strings")))
    }

    fn object(&mut self, obj: &Map<String, Value>, at: &str) -> Result<Node, ProtocolError> {
        let mut properties = BTreeMap::new();
        if let Some(props) = obj.get("properties") {
            let props = props
                .as_object()
                .ok_or_else(|| self.load(format!("{at}: `properties` must be an object")))?;
            for (name, sub) in props {
                let node = self.compile(sub, &format!("{at}/properties/{name}"), false)?;
                properties.insert(name.clone(), node);
            }
        }
        let required = match obj.get("required") {
            None => Vec::new(),
            Some(list) => self.strings(list, at, "required")?,
        };
        match obj.get("additionalProperties") {
            None | Some(Value::Bool(true)) => Ok(Node::Object {
                properties,
                required,
                closed: false,
            }),
            Some(Value::Bool(false)) => Ok(Node::Object {
                properties,
                required,
                closed: true,
            }),
            Some(schema @ Value::Object(_)) => {
                if !properties.is_empty() || !required.is_empty() {
                    return Err(self.load(format!(
                        "{at}: `additionalProperties` as a schema cannot be combined with named members"
                    )));
                }
                Ok(Node::Map(Box::new(self.compile(
                    schema,
                    &format!("{at}/additionalProperties"),
                    false,
                )?)))
            }
            Some(_) => Err(self.load(format!(
                "{at}: `additionalProperties` must be a boolean or a schema"
            ))),
        }
    }

    fn reference(
        &mut self,
        obj: &Map<String, Value>,
        reference: &Value,
        at: &str,
    ) -> Result<Node, ProtocolError> {
        if let Some(extra) = obj
            .keys()
            .find(|k| *k != "$ref" && !ANNOTATIONS.contains(&k.as_str()))
        {
            return Err(self.load(format!(
                "{at}: `{extra}` beside `$ref` is not in the subset"
            )));
        }
        let name = reference
            .as_str()
            .and_then(|r| r.strip_prefix("#/$defs/"))
            .filter(|n| !n.is_empty() && !n.contains('/'))
            .ok_or_else(|| self.load(format!("{at}: `$ref` must be `#/$defs/<name>`")))?
            .to_string();
        let target = self
            .defs
            .get(&name)
            .ok_or_else(|| self.load(format!("{at}: `$ref` names missing `$defs/{name}`")))?;
        if self.stack.contains(&name) {
            return Err(self.load(format!("{at}: `$defs/{name}` refers to itself")));
        }
        self.stack.push(name.clone());
        let node = self.compile(target, &format!("#/$defs/{name}"), false);
        self.stack.pop();
        node
    }
}

fn compile_root(file: &str, root: &Value) -> Result<Node, ProtocolError> {
    let empty = Map::new();
    let defs = match root.get("$defs") {
        None => &empty,
        Some(d) => d.as_object().ok_or_else(|| ProtocolError::SchemaLoad {
            file: file.to_string(),
            detail: "`$defs` must be an object".to_string(),
        })?,
    };
    let mut compiler = Compiler {
        file,
        defs,
        stack: Vec::new(),
    };
    // Compile every definition, used or not, so an unsupported keyword cannot hide in one.
    for (name, def) in defs {
        compiler.stack.push(name.clone());
        let compiled = compiler.compile(def, &format!("#/$defs/{name}"), false);
        compiler.stack.pop();
        compiled?;
    }
    let node = compiler.compile(root, "#", true)?;
    if !matches!(node, Node::Object { .. }) {
        return Err(compiler.load("the root schema must be an object type"));
    }
    Ok(node)
}
