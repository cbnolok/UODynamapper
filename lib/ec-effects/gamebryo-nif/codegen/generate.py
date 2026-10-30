#!/usr/bin/env python3
"""Generates the gamebryo-nif Rust types from the schema in schema/ for one NIF version.

usage (from anywhere; Python 3.9+ standard library only):
    python3 -I lib/ec-effects/gamebryo-nif/codegen/generate.py [--version 20.0.0.4] [--user-version 0]
        [--schema DIR] [--allowlist PATH] [--out DIR]

Defaults generate the checked-in code: version 20.0.0.4, user version 0, schema/*.schema,
allowlist.txt, and ../src/generated.

Schema format
-------------
The schema describes the stream layout of each class as the Gamebryo SDK's LoadBinary reads it.
`#` starts a comment. A declaration starts at column 0; its members are the indented lines below.

    const NAME = 5                        integer or float constant for expressions and defaults
    enum Name : u32 from "path"           members: `NAME` or `NAME = value` (C-style numbering)
    bitflags Name : u8 from "path"        members: `NAME = mask`
    bitfield Name : u16 from "path"       members: `name: bool|uint|EnumName @pos[:width]`
    struct Name[<T>] [: Parent] from "path" ["path" ...]
    object Name [: Parent] [abstract] from "path" ["path" ...]
    extern object Name : Parent from "path"
                                          a link target whose layout is not described (links to
                                          it compile, blocks of it cannot be read)

`from` cites the SDK source the layout was taken from. A struct or object member is a field:

    name: Type[dims] [if COND] [arg EXPR] [since V] [before V] [= DEFAULT]

* Type: u8 i8 u16 i16 u32 i32 u64 i64 f32, NiBool (one byte), string (inline length-prefixed),
  Ref<Object> (an owning link), Ptr<Object> (a non-owning link), an enum/bitflags/bitfield, a
  struct (generic ones take one type argument), or T inside a generic struct.
* dims: `[EXPR]` is an array of that length, `[EXPR][EXPR]` an array of rows; `[u32]` (or u8,
  u16) is an array stored with its own element count of that type in front.
* `if` reads the field only when COND is non-zero; `since V` / `before V` keep it only for file
  versions >= V / < V (resolved here, at generation time).
* `arg` passes a value to the field's type, read there as `arg` (key arrays pass the key type).
* Expressions use C operators, field names (own or inherited), `arg`, constants and
  `Enum.MEMBER`. A field that is not stored (version-gated out or condition false) evaluates to
  its default (0 if it has none).
* DEFAULT: a number, constant or `Enum.MEMBER`; `true`/`false` for NiBool; `{1.0, 0.5}` for a
  struct of floats; `{member: value, ...}` for a bitfield.

Design
------
* Version gates are resolved for the requested version, so the generated structs contain exactly
  the fields that version stores. Supporting another version means generating it into another
  module.
* Generated types are the allowlisted objects, their ancestors, and the structs and enums their
  fields use. Ref/Ptr targets outside that set become uninhabited marker types.
* Objects (and structs with a parent) embed their parent as a `base` field with Deref to it, so
  `node.name` works and an abstract ancestor can be borrowed from any subtype.
* Conditional fields are plain fields: they are read/written only when their condition holds and
  keep their default otherwise.
* Fields declared several times under one name (alternative positions or conditions) share one
  Rust field; each declaration still reads/writes at its own position.
"""

import argparse
import re
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent

# NiStream: the oldest file version the SDK still loads (NiStream::ms_uiNifMinVersion).
NIF_MIN_VERSION = "10.1.0.114"
# From this version on, strings are indices into the header's string table
# (NiStream::LoadFixedString); before it they are stored inline (LoadCStringAsFixedString),
# which is the only string form this generator and crate support.
STRING_TABLE_VERSION = "20.1.0.1"
# Integer types an array can store its element count as.
COUNT_TYPES = ("u8", "u16", "u32")

# ---------------------------------------------------------------------------- versions


def vnum(text):
    parts = [int(p) for p in text.split(".")]
    if len(parts) != 4:
        raise ValueError(f"version {text!r} is not a.b.c.d")
    return (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3]


# ---------------------------------------------------------------------------- expressions

OPS = ["||", "&&", "==", "!=", "<=", ">=", "<<", ">>", "<", ">", "|", "&", "+", "-", "*", "/",
       "!", "(", ")"]
PREC = {"||": 1, "&&": 2, "|": 3, "&": 5, "==": 6, "!=": 6, "<": 7, ">": 7, "<=": 7, ">=": 7,
        "<<": 8, ">>": 8, "+": 9, "-": 9, "*": 10, "/": 10}
OP_RE = re.compile("|".join(re.escape(o) for o in OPS))
NUM_RE = re.compile(r"^(0x[0-9a-fA-F]+|\d+\.\d+(e[+-]?\d+)?|\d+)$")
BOOL_OPS = {"||", "&&", "==", "!=", "<", ">", "<=", ">="}

# AST: ("const", value) | ("field", Field) | ("arg",) | ("not", node) | ("neg", node)
#      | ("bin", op, a, b)


def tokenize(text):
    raw = []
    pos = 0
    for m in OP_RE.finditer(text):
        chunk = text[pos:m.start()].strip()
        if chunk:
            raw.append(("id", chunk))
        raw.append(("op", m.group(0)))
        pos = m.end()
    chunk = text[pos:].strip()
    if chunk:
        raw.append(("id", chunk))
    out = []
    for kind, val in raw:
        if kind == "id" and NUM_RE.match(val):
            if val.startswith("0x"):
                out.append(("num", int(val, 16)))
            elif "." in val:
                out.append(("num", float(val)))
            else:
                out.append(("num", int(val)))
        else:
            out.append((kind, val))
    return out


class ExprParser:
    def __init__(self, text, resolve):
        self.text = text
        self.toks = tokenize(text)
        self.i = 0
        self.resolve = resolve

    def parse(self):
        if not self.toks:
            raise ValueError("empty expression")
        node = self.binary(0)
        if self.i != len(self.toks):
            raise ValueError(f"trailing tokens in {self.text!r}")
        return node

    def atom(self):
        if self.i >= len(self.toks):
            raise ValueError(f"unexpected end of {self.text!r}")
        kind, val = self.toks[self.i]
        self.i += 1
        if kind == "num":
            return ("const", val)
        if kind == "id":
            return self.resolve(val)
        if val == "(":
            node = self.binary(0)
            if self.i >= len(self.toks) or self.toks[self.i] != ("op", ")"):
                raise ValueError(f"missing ')' in {self.text!r}")
            self.i += 1
            return node
        if val == "!":
            return fold(("not", self.atom()))
        if val == "-":
            return fold(("neg", self.atom()))
        raise ValueError(f"bad token {val!r} in {self.text!r}")

    def binary(self, min_prec):
        lhs = self.atom()
        while self.i < len(self.toks):
            kind, op = self.toks[self.i]
            if kind != "op" or op not in PREC or PREC[op] < min_prec:
                break
            self.i += 1
            rhs = self.binary(PREC[op] + 1)
            lhs = fold(("bin", op, lhs, rhs))
        return lhs


def apply_op(op, a, b):
    return {
        "||": lambda: int(bool(a) or bool(b)), "&&": lambda: int(bool(a) and bool(b)),
        "|": lambda: int(a) | int(b), "&": lambda: int(a) & int(b),
        "==": lambda: int(a == b), "!=": lambda: int(a != b), "<": lambda: int(a < b),
        ">": lambda: int(a > b), "<=": lambda: int(a <= b), ">=": lambda: int(a >= b),
        "<<": lambda: int(a) << int(b), ">>": lambda: int(a) >> int(b), "+": lambda: a + b,
        "-": lambda: a - b, "*": lambda: a * b,
        "/": lambda: a // b if isinstance(a, int) and isinstance(b, int) else a / b,
    }[op]()


def fold(node):
    """Constant folding, including short-circuit of && and || with a constant side."""
    kind = node[0]
    if kind == "not" and node[1][0] == "const":
        return ("const", int(not node[1][1]))
    if kind == "neg" and node[1][0] == "const":
        return ("const", -node[1][1])
    if kind == "bin":
        _, op, a, b = node
        if a[0] == "const" and b[0] == "const":
            return ("const", apply_op(op, a[1], b[1]))
        for c, other in ((a, b), (b, a)):
            if c[0] != "const":
                continue
            if op == "&&":
                return other if c[1] else ("const", 0)
            if op == "||":
                return ("const", 1) if c[1] else other
            if op in ("|", "+") and c[1] == 0:
                return other
    return node


def or_nodes(nodes):
    out = None
    for n in nodes:
        out = n if out is None else fold(("bin", "||", out, n))
    return out


# ---------------------------------------------------------------------------- naming

RUST_KEYWORDS = {
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
}
SNAKE_RE = re.compile(r"^[a-z][a-z0-9_]*$")


def rust_ident(name):
    """A schema field name (already snake_case) as a Rust identifier."""
    return f"r#{name}" if name in RUST_KEYWORDS else name


def bare(ident):
    return ident.removeprefix("r#")


def upper_snake(name):
    out = re.sub(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", "_", name)
    out = re.sub(r"[^A-Za-z0-9]+", "_", out).strip("_").upper()
    if out[0].isdigit():
        out = "N_" + out
    return out


def rust_str(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def float_lit(value):
    text = repr(float(value))
    if text in ("inf", "-inf"):
        return "f32::INFINITY" if text == "inf" else "f32::NEG_INFINITY"
    return text if ("." in text or "e" in text) else text + ".0"


# ---------------------------------------------------------------------------- schema parsing

BASIC_RUST = {
    "u8": "u8", "i8": "i8", "u16": "u16", "i16": "i16", "u32": "u32", "i32": "i32",
    "u64": "u64", "i64": "i64", "f32": "f32", "NiBool": "NiBool",
}
ENUM_STORAGE = ("u8", "u16", "u32", "i32")
FIELD_KEYWORDS = ("if", "arg", "since", "before", "=")
OPENERS, CLOSERS = "([{<", ")]}>"


class SchemaError(Exception):
    pass


class EnumDecl:
    def __init__(self, kind, name, storage, sources, where):
        self.kind = kind  # "enum" | "bitflags" | "bitfield"
        self.name = name
        self.storage = storage
        self.sources = sources
        self.where = where
        self.options = []  # (name, value) for enum/bitflags
        self.members = []  # (name, type, pos, width) for bitfield


class FieldDecl:
    def __init__(self, name, type_name, template, dims, clauses, where):
        self.name = name
        self.type_name = type_name
        self.template = template
        self.dims = dims  # list of dimension texts
        self.cond = clauses.get("if")
        self.arg = clauses.get("arg")
        self.since = clauses.get("since")
        self.before = clauses.get("before")
        self.default = clauses.get("=")
        self.where = where

    @property
    def counted(self):
        return len(self.dims) == 1 and self.dims[0] in COUNT_TYPES


class TypeDecl:
    def __init__(self, kind, name, generic, parent, abstract, sources, where, extern=False):
        self.kind = kind  # "struct" | "object"
        self.name = name
        self.generic = generic
        self.parent = parent
        self.abstract = abstract
        self.sources = sources
        self.where = where
        self.extern = extern
        self.fields = []


def strip_comment(line):
    in_quote = False
    for i, ch in enumerate(line):
        if ch == '"':
            in_quote = not in_quote
        elif ch == "#" and not in_quote:
            return line[:i].rstrip()
    return line.rstrip()


def parse_sources(text, where):
    sources = re.findall(r'"([^"]+)"', text)
    if not sources or re.sub(r'"[^"]*"', "", text).strip():
        raise SchemaError(f"{where}: expected `from \"path\" [\"path\" ...]`")
    return sources


def parse_number(text):
    text = text.strip()
    neg = text.startswith("-")
    body = text[1:].strip() if neg else text
    if re.fullmatch(r"0x[0-9a-fA-F]+|\d+", body):
        value = int(body, 0)
    elif re.fullmatch(r"\d+\.\d*(e[+-]?\d+)?|\d+e[+-]?\d+", body):
        value = float(body)
    else:
        return None
    return -value if neg else value


def split_clauses(text, where):
    """Splits `if X arg Y since V before V = D` at top-level keywords."""
    clauses = {}
    current = None
    depth = 0
    for tok in text.split():
        # `arg` is also the name of the passed value inside expressions: it is that name when it
        # starts a clause value or follows an operator.
        operand_expected = current is not None and (
            not clauses[current] or clauses[current][-1][-1] in "=!<>&|+-*/(")
        if depth == 0 and tok in FIELD_KEYWORDS and not (tok == "arg" and operand_expected):
            if tok in clauses:
                raise SchemaError(f"{where}: `{tok}` given twice")
            current = tok
            clauses[current] = []
            continue
        if current is None:
            raise SchemaError(f"{where}: unexpected {tok!r}")
        clauses[current].append(tok)
        depth += sum(tok.count(c) for c in "([{") - sum(tok.count(c) for c in ")]}")
    out = {}
    for key, toks in clauses.items():
        if not toks:
            raise SchemaError(f"{where}: `{key}` without a value")
        out[key] = " ".join(toks)
    return out


def parse_field(text, where):
    m = re.match(r"(\w+)\s*:\s*(\w+)(?:<(\w+)>)?", text)
    if not m:
        raise SchemaError(f"{where}: expected `name: Type ...`")
    name, type_name, template = m.groups()
    if not SNAKE_RE.match(name):
        raise SchemaError(f"{where}: field name {name!r} is not snake_case")
    if name == "base":
        raise SchemaError(f"{where}: field name 'base' clashes with the parent field")
    rest = text[m.end():]
    dims = []
    while rest.startswith("["):
        depth = 0
        for i, ch in enumerate(rest):
            depth += (ch == "[") - (ch == "]")
            if depth == 0:
                break
        else:
            raise SchemaError(f"{where}: unbalanced '['")
        dims.append(rest[1:i].strip())
        rest = rest[i + 1:]
    if len(dims) > 2:
        raise SchemaError(f"{where}: at most two array dimensions are supported")
    if any(d in COUNT_TYPES for d in dims) and len(dims) != 1:
        raise SchemaError(f"{where}: a counted array must be one-dimensional")
    return FieldDecl(name, type_name, template, dims, split_clauses(rest, where), where)


HEADER_TYPE_RE = re.compile(
    r"^(extern\s+)?(struct|object)\s+(\w+)(<T>)?(?:\s*:\s*(\w+))?(\s+abstract)?\s+from\s+(.+)$")
HEADER_ENUM_RE = re.compile(r"^(enum|bitflags|bitfield)\s+(\w+)\s*:\s*(\w+)\s+from\s+(.+)$")
HEADER_CONST_RE = re.compile(r"^const\s+(\w+)\s*=\s*(.+)$")
BITFIELD_MEMBER_RE = re.compile(r"^(\w+)\s*:\s*(\w+)\s*@\s*(\d+)(?:\s*:\s*(\d+))?$")
ENUM_OPTION_RE = re.compile(r"^(\w+)(?:\s*=\s*(.+))?$")


class Schema:
    """All declarations of the schema files, before version resolution."""

    def __init__(self, paths):
        self.consts = {}
        self.enums = {}
        self.types = {}  # name -> TypeDecl (structs and objects)
        for path in paths:
            self.parse_file(path)
        self.check()

    def declare(self, table, name, value, where):
        if name in self.consts or name in self.enums or name in self.types:
            raise SchemaError(f"{where}: {name} is declared twice")
        table[name] = value

    def parse_file(self, path):
        current = None
        for number, raw in enumerate(path.read_text().splitlines(), 1):
            where = f"{path.name}:{number}"
            line = strip_comment(raw)
            if not line.strip():
                continue
            if line[0].isspace():
                if current is None:
                    raise SchemaError(f"{where}: indented line outside a declaration")
                self.parse_member(current, line.strip(), where)
                continue
            current = self.parse_header(line, where)

    def parse_header(self, line, where):
        m = HEADER_CONST_RE.match(line)
        if m:
            value = parse_number(m.group(2))
            if value is None:
                raise SchemaError(f"{where}: constant {m.group(1)} is not a number")
            self.declare(self.consts, m.group(1), value, where)
            return None
        m = HEADER_ENUM_RE.match(line)
        if m:
            kind, name, storage, rest = m.groups()
            if storage not in ENUM_STORAGE:
                raise SchemaError(f"{where}: storage {storage} is not one of {ENUM_STORAGE}")
            decl = EnumDecl(kind, name, storage, parse_sources(rest, where), where)
            self.declare(self.enums, name, decl, where)
            return decl
        m = HEADER_TYPE_RE.match(line)
        if m:
            extern, kind, name, generic, parent, abstract, rest = m.groups()
            if extern and kind != "object":
                raise SchemaError(f"{where}: only objects can be extern")
            if generic and (kind != "struct" or parent):
                raise SchemaError(f"{where}: only parentless structs can be generic")
            if kind == "object" and name != "NiObject" and not parent:
                raise SchemaError(f"{where}: object {name} needs a parent")
            decl = TypeDecl(kind, name, bool(generic), parent, bool(abstract),
                            parse_sources(rest, where), where, extern=bool(extern))
            self.declare(self.types, name, decl, where)
            return decl
        raise SchemaError(f"{where}: unknown declaration {line!r}")

    def parse_member(self, decl, text, where):
        if isinstance(decl, TypeDecl):
            if decl.extern:
                raise SchemaError(f"{where}: extern object {decl.name} cannot have fields")
            decl.fields.append(parse_field(text, where))
        elif decl.kind == "bitfield":
            m = BITFIELD_MEMBER_RE.match(text)
            if not m:
                raise SchemaError(f"{where}: expected `name: type @pos[:width]`")
            name, mtype, pos, width = m.groups()
            decl.members.append((name, mtype, int(pos), int(width or 1)))
        else:
            m = ENUM_OPTION_RE.match(text)
            if not m:
                raise SchemaError(f"{where}: expected `NAME [= value]`")
            name, value_text = m.groups()
            if value_text is None:
                if decl.kind == "bitflags":
                    raise SchemaError(f"{where}: bitflags members need a value")
                value = decl.options[-1][1] + 1 if decl.options else 0
            else:
                value = parse_number(value_text)
                if not isinstance(value, int):
                    raise SchemaError(f"{where}: {value_text!r} is not an integer")
            decl.options.append((name, value))

    def check(self):
        for decl in self.types.values():
            if decl.parent is None:
                continue
            parent = self.types.get(decl.parent)
            if parent is None or parent.kind != decl.kind:
                raise SchemaError(f"{decl.where}: parent {decl.parent} is not a {decl.kind}")
            if parent.extern and not decl.extern:
                raise SchemaError(f"{decl.where}: parent {decl.parent} is extern")

    def ancestors(self, name):
        out = []
        while name:
            out.append(name)
            name = self.types[name].parent
        return out

    def object_names(self):
        return {n for n, d in self.types.items() if d.kind == "object"}


# ---------------------------------------------------------------------------- model building


class Field:
    """One Rust field: one or more schema declarations ("slots") sharing a name."""

    def __init__(self, owner, name, rust_type, nif_type):
        self.owner = owner
        self.name = name
        self.rust_name = rust_ident(name)
        self.rust_type = rust_type
        self.nif_type = nif_type
        self.slots = []  # Slot objects in declaration order
        self.default = None  # schema default text of the first declaration
        self.absent_value = 0  # value in expressions while the field is not stored
        self.presence = ("const", 1)

    @property
    def scalar_kind(self):
        slot = self.slots[0]
        return "array" if slot.length is not None or slot.count else slot.elem_kind


class Slot:
    def __init__(self, field, cond, length, width, count, arg, elem_type, elem_kind):
        self.field = field
        self.cond = cond  # AST
        self.length = length  # AST or None
        self.width = width  # AST or None
        self.count = count  # count prefix type (counted array) or None
        self.arg = arg  # AST or None
        self.elem_type = elem_type  # Rust type of one element
        self.elem_kind = elem_kind  # "basic" | "enum" | "struct" | "link" | "string" | "generic"


class TypeDef:
    """A generated struct (schema struct or object)."""

    def __init__(self, decl):
        self.decl = decl
        self.name = decl.name
        self.is_object = decl.kind == "object"
        self.generic = decl.generic
        self.parent = decl.parent
        self.abstract = decl.abstract
        self.fields = []  # own Field objects in order
        self.slots = []  # own Slot objects in file order
        self.absent = {}  # names not stored in this version -> value in expressions


class Model:
    def __init__(self, schema, version, user_version, allowlist):
        self.s = schema
        self.ver = version
        self.user = user_version
        self.objects = {}  # name -> TypeDef (generated objects)
        self.compounds = {}  # name -> TypeDef
        self.enums = set()
        self.markers = set()
        objects = schema.object_names()
        for name in allowlist:
            decl = schema.types.get(name)
            if decl is None or decl.kind != "object" or decl.extern:
                sys.exit(f"allowlisted type {name} is not an object in the schema")
            if decl.abstract:
                sys.exit(f"allowlisted type {name} is abstract")
        names = set()
        for name in allowlist:
            names.update(schema.ancestors(name))
        # Block types are exactly the allowlist; other ancestors are generated as structs only,
        # and a block of such a type is reported as unknown.
        self.concrete = sorted(set(allowlist))
        self.descendants = {n: {c for c in self.concrete if n in schema.ancestors(c)}
                            for n in names}
        # Parents before children, so inherited fields are known when a child is built.
        self.object_order = sorted(names, key=lambda n: (len(schema.ancestors(n)), n))
        self.link_targets = objects
        for name in self.object_order:
            self.objects[name] = TypeDef(schema.types[name])
        for name in self.object_order:
            self.build(self.objects[name])

    # ---- versions

    def in_version(self, f):
        if f.since and self.ver < vnum(f.since):
            return False
        if f.before and self.ver >= vnum(f.before):
            return False
        return True

    # ---- types

    def need_compound(self, name):
        if name not in self.compounds:
            decl = self.s.types[name]
            if decl.parent:
                self.need_compound(decl.parent)
            td = TypeDef(decl)
            self.compounds[name] = td
            self.build(td)
        return self.compounds[name]

    def type_ref(self, td, f, name, template, allow_generic):
        """Rust type, schema type text and kind of a type reference."""
        where = f.where
        if name == "T":
            if not td.generic:
                sys.exit(f"{where}: T outside a generic struct")
            return "T", "T", "generic"
        if name in ("Ref", "Ptr"):
            if template not in self.link_targets:
                sys.exit(f"{where}: link target {template} is not an object")
            if template not in self.object_order:
                self.markers.add(template)
            return f"{name}<{template}>", f"{name}<{template}>", "link"
        if template and name not in self.s.types:
            sys.exit(f"{where}: {name} takes no type argument")
        if name in BASIC_RUST:
            return BASIC_RUST[name], name, "basic"
        if name == "string":
            return "NifString", "string", "string"
        if name in self.s.enums:
            self.need_enum(name)
            return name, name, "enum"
        decl = self.s.types.get(name)
        if decl is None or decl.kind != "struct":
            sys.exit(f"{where}: unknown type {name}")
        sub = self.need_compound(name)
        if not sub.generic:
            if template:
                sys.exit(f"{where}: {name} is not generic")
            return name, name, "struct"
        if not template:
            sys.exit(f"{where}: generic {name} without a type argument")
        if not allow_generic:
            sys.exit(f"{where}: nested generic type argument {name}")
        targ, ttext, _ = self.type_ref(td, f, template, None, False)
        return f"{name}<{targ}>", f"{name}<{ttext}>", "struct"

    def need_enum(self, name):
        if name in self.enums:
            return
        self.enums.add(name)
        decl = self.s.enums[name]
        for _, mtype, _, _ in decl.members:
            if mtype in self.s.enums:
                self.need_enum(mtype)
            elif mtype not in ("bool", "uint"):
                sys.exit(f"{decl.where}: bitfield member type {mtype} is not bool, uint or an enum")

    # ---- expressions

    def chain(self, td):
        out = [td]
        name = td.parent
        while name:
            out.append(self.objects[name] if td.is_object else self.compounds[name])
            name = out[-1].parent
        return out

    def scope_lookup(self, td, name):
        """A field visible from `td` (own, then ancestors), ("absent", value) or None."""
        for t in self.chain(td):
            for fld in t.fields:
                if fld.name == name:
                    return fld
            if name in t.absent:
                return ("absent", t.absent[name])
        return None

    def resolver(self, td, where):
        def resolve(name):
            if name == "arg":
                return ("arg",)
            if "." in name:
                enum_name, option = name.split(".", 1)
                decl = self.s.enums.get(enum_name)
                values = dict(decl.options) if decl else {}
                if option not in values:
                    sys.exit(f"{where}: unknown enum value {name}")
                return ("const", values[option])
            fld = self.scope_lookup(td, name) if td is not None else None
            if isinstance(fld, tuple):
                return ("const", fld[1])
            if fld is not None:
                return ("field", fld)
            if name in self.s.consts:
                return ("const", self.s.consts[name])
            sys.exit(f"{where}: unknown name {name!r} in an expression")
        return resolve

    def parse(self, td, text, where):
        try:
            return ExprParser(text, self.resolver(td, where)).parse()
        except ValueError as e:
            sys.exit(f"{where}: {e}")

    def const_int(self, text, where):
        """A default or absent value that must fold to an integer constant."""
        node = self.parse(None, text, where)
        if node[0] != "const" or not isinstance(node[1], int):
            sys.exit(f"{where}: {text!r} is not an integer constant")
        return node[1]

    def absent_value(self, f):
        """What a field evaluates to while it is not stored: its default if numeric, else 0."""
        text = f.default
        if text is None or text.startswith("{") or f.dims:
            return 0
        if text in ("true", "false"):
            return int(text == "true")
        node = self.parse(None, text, f.where)
        return node[1] if node[0] == "const" and isinstance(node[1], int) else 0

    # ---- building

    def build(self, td):
        for f in td.decl.fields:
            if not self.in_version(f):
                td.absent.setdefault(f.name, self.absent_value(f))
                continue
            cond = self.parse(td, f.cond, f.where) if f.cond else ("const", 1)
            if cond == ("const", 0):
                td.absent.setdefault(f.name, self.absent_value(f))
                continue
            elem, nif_type, kind = self.type_ref(td, f, f.type_name, f.template, True)
            length = width = count = None
            if f.counted:
                count = f.dims[0]
            elif f.dims:
                length = self.parse(td, f.dims[0], f.where)
                if len(f.dims) == 2:
                    width = self.parse(td, f.dims[1], f.where)
            for dim in (length, width):
                if dim is not None and dim[0] == "field" and dim[1].scalar_kind == "array":
                    sys.exit(f"{f.where}: jagged arrays are not supported")
            arg = self.parse(td, f.arg, f.where) if f.arg else None
            if not f.dims:
                rust_type = elem
            elif width is None:
                rust_type = f"Vec<{elem}>"
            else:
                rust_type = f"Vec<Vec<{elem}>>"
            nif_type += "".join(f"[{d}]" if d in COUNT_TYPES else "[]" for d in f.dims)
            existing = next((x for x in td.fields if x.name == f.name), None)
            if existing is None:
                existing = Field(td, f.name, rust_type, nif_type)
                existing.default = f.default
                existing.default_where = f.where
                existing.absent_value = self.absent_value(f)
                td.fields.append(existing)
            elif existing.rust_type != rust_type:
                sys.exit(f"{f.where}: {f.name} redeclared with type {rust_type} "
                         f"(was {existing.rust_type})")
            slot = Slot(existing, cond, length, width, count, arg, elem, kind)
            existing.slots.append(slot)
            td.slots.append(slot)
        for fld in td.fields:
            fld.presence = or_nodes([s.cond for s in fld.slots])


# ---------------------------------------------------------------------------- Rust emission


class Emitter:
    """Turns expression ASTs into Rust. `recv` is the struct value (`s` or `self`)."""

    def __init__(self, recv):
        self.recv = recv

    def value(self, node):
        code, kind = self.emit(node)
        return code if kind == "i" else f"i64::from({code})"

    def cond(self, node):
        code, kind = self.emit(node)
        return code if kind == "b" else f"({code} != 0)"

    def emit(self, node):
        kind = node[0]
        if kind == "const":
            v = node[1]
            if isinstance(v, float):
                sys.exit(f"float constant {v} in an integer expression")
            return str(v), "i"
        if kind == "arg":
            return "arg", "i"
        if kind == "field":
            fld = node[1]
            if fld.scalar_kind in ("array", "struct", "string", "link", "generic"):
                sys.exit(f"{fld.owner.name}.{fld.name}: used as a number in an expression")
            access = f"{self.recv}.{fld.rust_name}.ev()"
            if fld.presence == ("const", 1):
                return access, "i"
            return (f"(if {self.cond(fld.presence)} {{ {access} }} "
                    f"else {{ {fld.absent_value} }})"), "i"
        if kind == "not":
            return f"!{self.cond(node[1])}", "b"
        if kind == "neg":
            return f"-{self.value(node[1])}", "i"
        _, op, a, b = node
        if op in ("&&", "||"):
            return f"({self.cond(a)} {op} {self.cond(b)})", "b"
        if op in BOOL_OPS:
            return f"({self.value(a)} {op} {self.value(b)})", "b"
        return f"({self.value(a)} {op} {self.value(b)})", "i"


def expr_text(node):
    """Readable form of an AST, for FieldInfo.condition and .length."""
    kind = node[0]
    if kind == "const":
        return str(node[1])
    if kind == "arg":
        return "arg"
    if kind == "field":
        return node[1].name
    if kind == "not":
        return f"!{expr_text(node[1])}"
    if kind == "neg":
        return f"-{expr_text(node[1])}"
    _, op, a, b = node
    return f"({expr_text(a)} {op} {expr_text(b)})"


HEADER = """// @generated by lib/ec-effects/gamebryo-nif/codegen/generate.py -- DO NOT EDIT.
//
// Source: lib/ec-effects/gamebryo-nif/codegen/schema/*.schema (written from the Gamebryo SDK
// stream loading code), for NIF version {version}, user version {user}.
// Regenerate: python3 -I lib/ec-effects/gamebryo-nif/codegen/generate.py
"""

FILE_ALLOWS = """#![allow(clippy::all)]
#![allow(unused_variables, unused_parens, unused_imports, non_camel_case_types, non_snake_case)]
"""


def source_text(decl):
    return ", ".join(f"`{s}`" for s in decl.sources)


class Generator:
    def __init__(self, model, version_text):
        self.m = model
        self.s = model.s
        self.version_text = version_text
        self.infos = []  # rust FieldInfo statics

    def header(self):
        return HEADER.format(version=self.version_text, user=self.m.user)

    # ---- enums

    def gen_enums(self):
        out = [self.header(), FILE_ALLOWS,
               "use crate::reflect::{BitfieldMember, EnumKind, EnumOption};\n"]
        for name in sorted(self.m.enums):
            decl = self.s.enums[name]
            storage = decl.storage
            const = upper_snake(name)
            out.append(f"/// {decl.kind} `{name}` (storage `{storage}`), from {source_text(decl)}.")
            out.append("#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]")
            out.append(f"pub struct {name}(pub {storage});\n")
            if decl.kind in ("enum", "bitflags"):
                out.append(f"impl {name} {{")
                for oname, value in decl.options:
                    out.append(f"    pub const {upper_snake(oname)}: Self = Self({value});")
                out.append("}\n")
                out.append(f"const {const}_OPTIONS: &[EnumOption] = &[")
                for oname, value in decl.options:
                    raw = str(value) if value >= 0 else f"({value}{storage}) as u64"
                    out.append(f"    EnumOption {{ name: {rust_str(oname)}, value: {raw} }},")
                out.append("];\n")
                kind = "Enum" if decl.kind == "enum" else "Bitflags"
                out.append(f"crate::nif_enum!({name}, {storage}, "
                           f"EnumKind::{kind}({const}_OPTIONS));\n")
            else:
                out.append(f"impl {name} {{")
                for mname, mtype, pos, width in decl.members:
                    mask = ((1 << width) - 1) << pos
                    getter = rust_ident(mname)
                    setter = "set_" + mname
                    if mtype == "bool" and width == 1:
                        out.append(f"    pub fn {getter}(&self) -> bool {{ self.0 & {mask:#x} != 0 }}")
                        out.append(f"    pub fn {setter}(&mut self, value: bool) {{ "
                                   f"if value {{ self.0 |= {mask:#x} }} else {{ self.0 &= !{mask:#x} }} }}")
                    else:
                        out.append(f"    pub fn {getter}(&self) -> {storage} {{ "
                                   f"(self.0 & {mask:#x}) >> {pos} }}")
                        out.append(f"    pub fn {setter}(&mut self, value: {storage}) {{ "
                                   f"self.0 = (self.0 & !{mask:#x}) | ((value << {pos}) & {mask:#x}) }}")
                out.append("}\n")
                out.append(f"const {const}_MEMBERS: &[BitfieldMember] = &[")
                for mname, mtype, pos, width in decl.members:
                    out.append(f"    BitfieldMember {{ name: {rust_str(mname)}, "
                               f"pos: {pos}, width: {width}, "
                               f"nif_type: {rust_str(mtype)} }},")
                out.append("];\n")
                out.append(f"crate::nif_enum!({name}, {storage}, EnumKind::Bitfield({const}_MEMBERS));\n")
        return "\n".join(out)

    # ---- defaults

    def default_expr(self, fld):
        slot = fld.slots[0]
        if slot.length is not None or slot.count:
            if fld.default is not None:
                sys.exit(f"{fld.default_where}: arrays cannot have a default")
            return "Vec::new()"
        text = fld.default
        if text is None:
            return "Default::default()"
        where = fld.default_where
        kind, ty = slot.elem_kind, slot.elem_type
        if kind == "basic":
            if ty == "NiBool":
                if text in ("true", "false"):
                    return f"NiBool({int(text == 'true')})"
                return f"NiBool({self.m.const_int(text, where)})"
            if ty == "f32":
                return float_lit(self.const_float(text, where))
            return str(self.m.const_int(text, where))
        if kind == "enum":
            decl = self.s.enums[ty]
            if decl.kind == "bitfield":
                return f"{ty}({self.bitfield_value(decl, text, where)})"
            if re.fullmatch(rf"{ty}\.\w+", text) and text.split(".")[1] in dict(decl.options):
                return f"{ty}::{upper_snake(text.split('.')[1])}"
            return f"{ty}({self.m.const_int(text, where)})"
        if kind == "struct" and "<" not in ty and text.startswith("{") and text.endswith("}"):
            parts = [p.strip() for p in text[1:-1].split(",")]
            sub = self.m.compounds[ty]
            if (not sub.parent and len(parts) == len(sub.fields)
                    and all(f.slots[0].elem_type == "f32" for f in sub.fields)):
                inits = ", ".join(f"{f.rust_name}: {float_lit(self.const_float(p, where))}"
                                  for f, p in zip(sub.fields, parts))
                return f"{ty} {{ {inits} }}"
        sys.exit(f"{where}: unsupported default {text!r} for {ty}")

    def const_float(self, text, where):
        node = self.m.parse(None, text, where)
        if node[0] != "const":
            sys.exit(f"{where}: {text!r} is not a constant")
        return float(node[1])

    def bitfield_value(self, decl, text, where):
        if not (text.startswith("{") and text.endswith("}")):
            return self.m.const_int(text, where)
        members = {m[0]: m for m in decl.members}
        value = 0
        for item in filter(None, (p.strip() for p in text[1:-1].split(","))):
            mname, _, mtext = (s.strip() for s in item.partition(":"))
            if mname not in members:
                sys.exit(f"{where}: {decl.name} has no member {mname}")
            _, _, pos, width = members[mname]
            mvalue = self.m.const_int(mtext, where)
            if not 0 <= mvalue < (1 << width):
                sys.exit(f"{where}: {mname} value {mvalue} does not fit {width} bits")
            value |= mvalue << pos
        return value

    # ---- field info

    def info_const(self, td, fld):
        const = f"FI_{upper_snake(td.name)}_{bare(fld.rust_name).upper()}"
        slot = fld.slots[0]
        length_text = None if slot.length is None else expr_text(slot.length)
        cond_text = None if fld.presence == ("const", 1) else expr_text(fld.presence)
        opt = lambda v: "None" if v is None else f"Some({rust_str(v)})"
        self.infos.append(
            f"static {const}: FieldInfo = FieldInfo {{ name: {rust_str(fld.name)}, "
            f"nif_type: {rust_str(fld.nif_type)}, declared_in: {rust_str(td.name)}, "
            f"length: {opt(length_text)}, condition: {opt(cond_text)} }};")
        return const

    # ---- struct bodies

    def generics(self, td):
        return ("<T: FieldType>", "<T>") if td.generic else ("", "")

    def struct_def(self, td):
        out = []
        extra = " (abstract)" if td.abstract else ""
        parent = f", inherits `{td.parent}`" if td.parent else ""
        out.append(f"/// {td.decl.kind} `{td.name}`{extra}{parent}.")
        out.append("///")
        out.append(f"/// Layout from {source_text(td.decl)}.")
        out.append("#[derive(Debug, Clone, PartialEq)]")
        bound, _ = self.generics(td)
        out.append(f"pub struct {td.name}{bound} {{")
        if td.parent:
            out.append(f"    pub base: {td.parent},")
        for fld in td.fields:
            notes = [f"`{fld.nif_type}`"]
            if fld.slots[0].length is not None:
                notes.append(f"length `{expr_text(fld.slots[0].length)}`")
            if fld.slots[0].width is not None:
                notes.append(f"width `{expr_text(fld.slots[0].width)}`")
            if fld.slots[0].count:
                notes.append(f"with a `{fld.slots[0].count}` count in front")
            if fld.presence != ("const", 1):
                notes.append(f"if `{expr_text(fld.presence)}`")
            out.append(f"    /// {', '.join(notes)}")
            out.append(f"    pub {fld.rust_name}: {fld.rust_type},")
        out.append("}\n")
        return out

    def default_impl(self, td):
        bound, args = self.generics(td)
        out = [f"impl{bound} Default for {td.name}{args} {{", "    fn default() -> Self {",
               "        Self {"]
        if td.parent:
            out.append("            base: Default::default(),")
        for fld in td.fields:
            out.append(f"            {fld.rust_name}: {self.default_expr(fld)},")
        out += ["        }", "    }", "}\n"]
        return out

    def deref_impls(self, td):
        out = []
        for trait, ref, method in (("Deref", "&", "deref"), ("DerefMut", "&mut ", "deref_mut")):
            target = "" if trait == "DerefMut" else f"    type Target = {td.parent};\n"
            self_ref = "&self" if trait == "Deref" else "&mut self"
            out.append(f"impl std::ops::{trait} for {td.name} {{")
            if target:
                out.append(target)
            out.append(f"    fn {method}({self_ref}) -> {ref}{td.parent} {{ {ref}self.base }}")
            out.append("}\n")
        return out

    def guarded(self, em, slot, stmt, indent):
        if slot.cond == ("const", 1):
            return [indent + stmt]
        return [f"{indent}if {em.cond(slot.cond)} {{", f"{indent}    {stmt}", f"{indent}}}"]

    def read_body(self, td, indent):
        em = Emitter("s")
        out = []
        for slot in td.slots:
            fld = slot.field
            arg = em.value(slot.arg) if slot.arg is not None else "0"
            name = rust_str(fld.name)
            if slot.count:
                call = f"rd_counted::<{slot.count}, _>(r, {name}, {arg})?"
            elif slot.length is None:
                call = f"rd(r, {name}, {arg})?"
            elif slot.width is None:
                call = f"rd_vec(r, {name}, {em.value(slot.length)}, {arg})?"
            else:
                call = (f"rd_vec2(r, {name}, {em.value(slot.length)}, "
                        f"{em.value(slot.width)}, {arg})?")
            out += self.guarded(em, slot, f"s.{fld.rust_name} = {call};", indent)
        return out

    def write_body(self, td, indent):
        em = Emitter("self")
        out = []
        for slot in td.slots:
            fld = slot.field
            arg = em.value(slot.arg) if slot.arg is not None else "0"
            name = rust_str(fld.name)
            value = f"&self.{fld.rust_name}"
            if slot.count:
                stmt = f"wr_counted::<{slot.count}, _>(w, {name}, {value}, {arg})?;"
            elif slot.length is None:
                stmt = f"wr(w, {name}, {value}, {arg})?;"
            elif slot.width is None:
                stmt = f"wr_vec(w, {name}, {value}, {em.value(slot.length)}, {arg})?;"
            else:
                stmt = (f"wr_vec2(w, {name}, {value}, {em.value(slot.length)}, "
                        f"{em.value(slot.width)}, {arg})?;")
            out += self.guarded(em, slot, stmt, indent)
        return out

    def field_arg(self, em, fld):
        """The arg a field passes to its value (reflection uses the first declaration's)."""
        slot = fld.slots[0]
        return em.value(slot.arg) if slot.arg is not None else "0"

    def reflect_body(self, td, mutable, indent):
        """Field list pushes. Mutable views borrow fields of `self` for the result lifetime, so
        presence and args (which read `self`) are bound first; the returned `lets` must be
        emitted before any mutable borrow."""
        em = Emitter("self")
        lets, pushes = [], []
        for i, fld in enumerate(td.fields):
            info = self.info_by_field[id(fld)]
            present = "true" if fld.presence == ("const", 1) else em.cond(fld.presence)
            arg = self.field_arg(em, fld)
            if mutable and present != "true":
                lets.append(f"{indent}let p{i} = {present};")
                present = f"p{i}"
            if mutable and not arg.lstrip("-").isdigit():
                lets.append(f"{indent}let a{i} = {arg};")
                arg = f"a{i}"
            if mutable:
                pushes.append(f"{indent}out.push(field_mut(&{info}, {present}, "
                              f"self.{fld.rust_name}.as_value_mut({arg})));")
            else:
                pushes.append(f"{indent}out.push(field(&{info}, {present}, "
                              f"self.{fld.rust_name}.as_value({arg})));")
        return lets, pushes

    def min_size(self, td):
        terms = [f"<{td.parent} as NifIo>::MIN_SIZE"] if td.parent else []
        for fld in td.fields:
            slots = fld.slots
            if len(slots) != 1 or slots[0].cond != ("const", 1):
                continue
            slot = slots[0]
            elem = f"<{slot.elem_type} as NifIo>::MIN_SIZE"
            if slot.count:
                terms.append(f"<{slot.count} as NifIo>::MIN_SIZE")
            elif slot.length is None:
                terms.append(elem)
            elif slot.length[0] == "const" and slot.width is None:
                terms.append(f"{slot.length[1]} * {elem}")
        return " + ".join(terms) if terms else "0"

    # ---- compounds

    IO_USE = ("use crate::io::{rd, rd_counted, rd_vec, rd_vec2, wr, wr_counted, wr_vec, wr_vec2, "
              "Ev, NifIo, Reader, Writer};")

    def gen_compounds(self):
        out = [self.header(), FILE_ALLOWS, "use super::*;",
               "use crate::error::Result;",
               self.IO_USE,
               "use crate::reflect::{field, field_mut, AsValue, Field, FieldInfo, FieldMut, FieldType,",
               "    Reflect, StructMut, StructRef, Value, ValueMut};",
               "use crate::types::{NiBool, NifString, Ptr, Ref};\n"]
        body = []
        for name in sorted(self.m.compounds):
            td = self.m.compounds[name]
            bound, args = self.generics(td)
            body += self.struct_def(td)
            body += self.default_impl(td)
            if td.parent:
                body += self.deref_impls(td)
            body.append(f"impl{bound} NifIo for {name}{args} {{")
            body.append(f"    const MIN_SIZE: usize = {self.min_size(td)};\n")
            body.append("    fn read(r: &mut Reader<'_>, arg: i64) -> Result<Self> {")
            mut = "mut " if td.slots else ""
            if td.parent:
                body.append(f"        let {mut}s = Self {{ base: {td.parent}::read(r, arg)?, "
                            "..Default::default() };")
            else:
                body.append(f"        let {mut}s = Self::default();")
            body += self.read_body(td, "        ")
            body.append("        Ok(s)")
            body.append("    }\n")
            body.append("    fn write(&self, w: &mut Writer, arg: i64) -> Result<()> {")
            if td.parent:
                body.append("        self.base.write(w, arg)?;")
            body += self.write_body(td, "        ")
            body.append("        Ok(())")
            body.append("    }")
            body.append("}\n")
            body.append(f"impl{bound} Reflect for {name}{args} {{")
            body.append(f"    fn type_name(&self) -> &'static str {{ {rust_str(name)} }}\n")
            for mutable in (False, True):
                if mutable:
                    body.append("    fn fields_mut(&mut self, arg: i64) -> Vec<FieldMut<'_>> {")
                else:
                    body.append("    fn fields(&self, arg: i64) -> Vec<Field<'_>> {")
                body.append(f"        let mut out = Vec::with_capacity({len(td.fields)});")
                lets, pushes = self.reflect_body(td, mutable, "        ")
                body += lets
                if td.parent:
                    method = "fields_mut" if mutable else "fields"
                    body.append(f"        out.extend(self.base.{method}(arg));")
                body += pushes
                body.append("        out")
                body.append("    }\n")
            body.append("}\n")
            body.append(f"impl{bound} AsValue for {name}{args} {{")
            body.append("    fn as_value(&self, arg: i64) -> Value<'_> {")
            body.append("        Value::Struct(StructRef { value: self, arg })")
            body.append("    }\n")
            body.append("    fn as_value_mut(&mut self, arg: i64) -> ValueMut<'_> {")
            body.append("        ValueMut::Struct(StructMut { value: self, arg })")
            body.append("    }")
            body.append("}\n")
        return "\n".join(out + self.compound_infos + [""] + body)

    def flush_infos(self):
        infos, self.infos = self.infos, []
        return infos

    def prepare_infos(self):
        """Names every field's FieldInfo static; reflection code refers to them."""
        self.info_by_field = {}
        for name in sorted(self.m.compounds):
            td = self.m.compounds[name]
            for fld in td.fields:
                self.info_by_field[id(fld)] = self.info_const(td, fld)
        self.compound_infos = self.flush_infos()
        for name in self.m.object_order:
            td = self.m.objects[name]
            for fld in td.fields:
                self.info_by_field[id(fld)] = self.info_const(td, fld)
        self.object_infos = self.flush_infos()

    # ---- objects

    def gen_objects(self):
        out = [self.header(), FILE_ALLOWS, "use super::*;",
               "use crate::error::Result;",
               self.IO_USE,
               "use crate::reflect::{field, field_mut, Field, FieldInfo, FieldMut, AsValue};",
               "use crate::types::{NiBool, NiObjectType, NifString, Ptr, Ref};\n"]
        body = []
        for name in self.m.object_order:
            td = self.m.objects[name]
            body += self.struct_def(td)
            body += self.default_impl(td)
            if td.parent:
                body += self.deref_impls(td)
            body.append(f"impl ObjectIo for {name} {{")
            body.append("    fn read_fields(r: &mut Reader<'_>) -> Result<Self> {")
            mut = "mut " if td.slots else ""
            if td.parent:
                body.append(f"        let {mut}s = Self {{ base: {td.parent}::read_fields(r)?, "
                            "..Default::default() };")
            else:
                body.append(f"        let {mut}s = Self::default();")
            body += self.read_body(td, "        ")
            body.append("        Ok(s)")
            body.append("    }\n")
            body.append("    fn write_fields(&self, w: &mut Writer) -> Result<()> {")
            if td.parent:
                body.append("        self.base.write_fields(w)?;")
            body += self.write_body(td, "        ")
            body.append("        Ok(())")
            body.append("    }\n")
            body.append("    fn collect_fields<'a>(&'a self, out: &mut Vec<Field<'a>>) {")
            if td.parent:
                body.append("        self.base.collect_fields(out);")
            body += self.reflect_body(td, False, "        ")[1]
            body.append("    }\n")
            body.append("    fn collect_fields_mut<'a>(&'a mut self, out: &mut Vec<FieldMut<'a>>) {")
            lets, pushes = self.reflect_body(td, True, "        ")
            body += lets
            if td.parent:
                body.append("        self.base.collect_fields_mut(out);")
            body += pushes
            body.append("    }")
            body.append("}\n")
            body += self.object_type_impl(td)
        for name in sorted(self.m.markers):
            decl = self.s.types[name]
            body.append(f"/// object `{name}` ({source_text(decl)}): a link target whose layout")
            body.append("/// this crate does not describe (no block of this type can be read).")
            body.append("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
            body.append(f"pub enum {name} {{}}\n")
            body.append(f"impl NiObjectType for {name} {{")
            body.append(f"    const NAME: &'static str = {rust_str(name)};\n")
            body.append("    fn from_block(_block: &Block) -> Option<&Self> { None }\n")
            body.append("    fn from_block_mut(_block: &mut Block) -> Option<&mut Self> { None }")
            body.append("}\n")
        return "\n".join(out + self.object_infos + [""] + body)

    def path_to(self, concrete, ancestor):
        depth = self.s.ancestors(concrete).index(ancestor)
        return "x" + ".base" * depth

    def object_type_impl(self, td):
        out = [f"impl NiObjectType for {td.name} {{",
               f"    const NAME: &'static str = {rust_str(td.name)};\n"]
        for mutable in (False, True):
            sig = ("fn from_block_mut(block: &mut Block) -> Option<&mut Self>" if mutable
                   else "fn from_block(block: &Block) -> Option<&Self>")
            ref = "&mut " if mutable else "&"
            out.append(f"    {sig} {{")
            out.append("        match block {")
            for c in sorted(self.m.descendants[td.name]):
                path = self.path_to(c, td.name)
                value = path if path == "x" else ref + path
                out.append(f"            Block::{c}(x) => Some({value}),")
            if len(self.m.descendants[td.name]) < len(self.m.concrete):
                out.append("            _ => None,")
            out.append("        }")
            out.append("    }\n")
        out.append("}\n")
        return out

    # ---- blocks

    def gen_blocks(self):
        concrete = self.m.concrete
        out = [self.header(), FILE_ALLOWS, "use super::*;",
               "use crate::error::Result;",
               "use crate::io::{Reader, Writer};",
               "use crate::reflect::{Field, FieldMut, Reflect};",
               "use crate::types::NiObjectType;\n"]
        out.append(f"/// NIF version this module was generated for ({self.version_text}).")
        out.append(f"pub const NIF_VERSION: u32 = {self.m.ver:#010x};")
        out.append("/// User version this module was generated for.")
        out.append(f"pub const NIF_USER_VERSION: u32 = {self.m.user};\n")
        out.append("/// The concrete block types this crate reads and writes.")
        out.append("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]")
        out.append("pub enum BlockType {")
        out += [f"    {c}," for c in concrete]
        out.append("}\n")
        out.append("impl BlockType {")
        out.append("    pub const ALL: &'static [BlockType] = &[")
        out += [f"        BlockType::{c}," for c in concrete]
        out.append("    ];\n")
        out.append("    /// The class name, as the file's block type table stores it.")
        out.append("    pub fn name(self) -> &'static str {")
        out.append("        match self {")
        out += [f"            BlockType::{c} => {rust_str(c)}," for c in concrete]
        out.append("        }")
        out.append("    }\n")
        out.append("    pub fn from_name(name: &[u8]) -> Option<Self> {")
        out.append("        Some(match name {")
        out += [f"            b{rust_str(c)} => BlockType::{c}," for c in concrete]
        out.append("            _ => return None,")
        out.append("        })")
        out.append("    }\n")
        out.append("    /// The type and its ancestors, most derived first (ends with `NiObject`).")
        out.append("    pub fn ancestry(self) -> &'static [&'static str] {")
        out.append("        match self {")
        for c in concrete:
            chain = ", ".join(rust_str(a) for a in self.s.ancestors(c))
            out.append(f"            BlockType::{c} => &[{chain}],")
        out.append("        }")
        out.append("    }\n")
        out.append("    /// Whether the type is `type_name` or derives from it.")
        out.append("    pub fn is_a(self, type_name: &str) -> bool {")
        out.append("        self.ancestry().contains(&type_name)")
        out.append("    }")
        out.append("}\n")
        out.append("/// A block of any supported type.")
        out.append("#[derive(Debug, Clone, PartialEq)]")
        out.append("pub enum Block {")
        out += [f"    {c}({c})," for c in concrete]
        out.append("}\n")
        out.append("impl Block {")
        out.append("    pub fn block_type(&self) -> BlockType {")
        out.append("        match self {")
        out += [f"            Block::{c}(_) => BlockType::{c}," for c in concrete]
        out.append("        }")
        out.append("    }\n")
        out.append("    pub fn type_name(&self) -> &'static str {")
        out.append("        self.block_type().name()")
        out.append("    }\n")
        out.append("    /// Reads a block of type `ty` at the reader's position.")
        out.append("    pub fn read(ty: BlockType, r: &mut Reader<'_>) -> Result<Self> {")
        out.append("        Ok(match ty {")
        out += [f"            BlockType::{c} => Block::{c}({c}::read_fields(r)?)," for c in concrete]
        out.append("        })")
        out.append("    }\n")
        out.append("    pub fn write(&self, w: &mut Writer) -> Result<()> {")
        out.append("        match self {")
        out += [f"            Block::{c}(x) => x.write_fields(w)," for c in concrete]
        out.append("        }")
        out.append("    }\n")
        out.append("    /// The fields, inherited ones first, in file order.")
        out.append("    pub fn fields(&self) -> Vec<Field<'_>> {")
        out.append("        let mut out = Vec::new();")
        out.append("        match self {")
        out += [f"            Block::{c}(x) => x.collect_fields(&mut out)," for c in concrete]
        out.append("        }")
        out.append("        out")
        out.append("    }\n")
        out.append("    pub fn fields_mut(&mut self) -> Vec<FieldMut<'_>> {")
        out.append("        let mut out = Vec::new();")
        out.append("        match self {")
        out += [f"            Block::{c}(x) => x.collect_fields_mut(&mut out)," for c in concrete]
        out.append("        }")
        out.append("        out")
        out.append("    }\n")
        out.append("    /// The block viewed as `T` (a concrete type or an ancestor).")
        out.append("    pub fn get<T: NiObjectType>(&self) -> Option<&T> {")
        out.append("        T::from_block(self)")
        out.append("    }\n")
        out.append("    pub fn get_mut<T: NiObjectType>(&mut self) -> Option<&mut T> {")
        out.append("        T::from_block_mut(self)")
        out.append("    }")
        out.append("}\n")
        out.append("impl Reflect for Block {")
        out.append("    fn type_name(&self) -> &'static str {")
        out.append("        Block::type_name(self)")
        out.append("    }\n")
        out.append("    fn fields(&self, _arg: i64) -> Vec<Field<'_>> {")
        out.append("        Block::fields(self)")
        out.append("    }\n")
        out.append("    fn fields_mut(&mut self, _arg: i64) -> Vec<FieldMut<'_>> {")
        out.append("        Block::fields_mut(self)")
        out.append("    }")
        out.append("}\n")
        for c in concrete:
            out.append(f"impl From<{c}> for Block {{")
            out.append(f"    fn from(value: {c}) -> Self {{")
            out.append(f"        Block::{c}(value)")
            out.append("    }")
            out.append("}\n")
            out.append(f"impl {c} {{")
            out.append(f"    pub const BLOCK_TYPE: BlockType = BlockType::{c};")
            out.append("}\n")
            out.append(f"impl Reflect for {c} {{")
            out.append(f"    fn type_name(&self) -> &'static str {{ {rust_str(c)} }}\n")
            out.append("    fn fields(&self, _arg: i64) -> Vec<Field<'_>> {")
            out.append("        let mut out = Vec::new();")
            out.append("        self.collect_fields(&mut out);")
            out.append("        out")
            out.append("    }\n")
            out.append("    fn fields_mut(&mut self, _arg: i64) -> Vec<FieldMut<'_>> {")
            out.append("        let mut out = Vec::new();")
            out.append("        self.collect_fields_mut(&mut out);")
            out.append("        out")
            out.append("    }")
            out.append("}\n")
        return "\n".join(out)

    def gen_mod(self):
        return "\n".join([
            self.header(),
            "mod blocks;",
            "mod compounds;",
            "mod enums;",
            "mod objects;\n",
            "pub use blocks::*;",
            "pub use compounds::*;",
            "pub use enums::*;",
            "pub use objects::*;\n",
            "use crate::error::Result;",
            "use crate::io::{Reader, Writer};",
            "use crate::reflect::{Field, FieldMut};\n",
            "/// Per-object serialization and reflection of the object's own and inherited fields.",
            "pub(crate) trait ObjectIo: Sized {",
            "    fn read_fields(r: &mut Reader<'_>) -> Result<Self>;",
            "    fn write_fields(&self, w: &mut Writer) -> Result<()>;",
            "    fn collect_fields<'a>(&'a self, out: &mut Vec<Field<'a>>);",
            "    fn collect_fields_mut<'a>(&'a mut self, out: &mut Vec<FieldMut<'a>>);",
            "}",
            "",
        ])


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--version", default="20.0.0.4")
    ap.add_argument("--user-version", type=lambda v: int(v, 0), default=0)
    ap.add_argument("--schema", type=Path, default=SCRIPT_DIR / "schema")
    ap.add_argument("--allowlist", type=Path, default=SCRIPT_DIR / "allowlist.txt")
    ap.add_argument("--out", type=Path, default=SCRIPT_DIR.parent / "src" / "generated")
    args = ap.parse_args()

    version = vnum(args.version)
    if not vnum(NIF_MIN_VERSION) <= version < vnum(STRING_TABLE_VERSION):
        sys.exit(f"version {args.version} is outside the supported range "
                 f"{NIF_MIN_VERSION} <= version < {STRING_TABLE_VERSION}")
    allowlist = [line.split("#")[0].strip() for line in args.allowlist.read_text().splitlines()]
    allowlist = [a for a in allowlist if a]
    paths = sorted(args.schema.glob("*.schema"))
    if not paths:
        sys.exit(f"no *.schema files in {args.schema}")
    try:
        schema = Schema(paths)
    except SchemaError as e:
        sys.exit(f"schema error: {e}")
    model = Model(schema, version, args.user_version, allowlist)
    gen = Generator(model, args.version)
    gen.prepare_infos()
    files = {
        "enums.rs": gen.gen_enums(),
        "compounds.rs": gen.gen_compounds(),
        "objects.rs": gen.gen_objects(),
        "blocks.rs": gen.gen_blocks(),
        "mod.rs": gen.gen_mod(),
    }
    args.out.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (args.out / name).write_text(text.rstrip("\n") + "\n")
    print(f"generated {len(model.concrete)} block types, {len(model.objects)} objects, "
          f"{len(model.compounds)} structs, {len(model.enums)} enums, "
          f"{len(model.markers)} link-only markers into {args.out}")


if __name__ == "__main__":
    main()
