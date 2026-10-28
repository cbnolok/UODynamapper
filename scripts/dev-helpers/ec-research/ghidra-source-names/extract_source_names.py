#!/usr/bin/env python3
"""Extract naming evidence from an open-source library's sources into a map for
NameSourceFunctions.java: function definitions, string literals used by exactly one
function, direct call edges (C), statically initialized tables of function pointers
(flattened to byte offsets for 32-bit MSVC) and MSVC primary vtable layouts (C++).

Usage: python3 -I extract_source_names.py --lib NAME [--version VERSION] --out MAP.tsv SRC_DIR...

NAME selects a profile (file selection, debug-only macros, structural macros, fixed
preprocessor conditions); unknown names use the generic profile. The parser is a tokenizer,
not a preprocessor: #if branches are kept as conditions. Conditions fixed by the profile
are applied; the others become variant atoms for tables and vtables (each variant is
emitted, the Ghidra script keeps the one that matches the program) and are ignored for
literals and calls (both branches count).
"""
import argparse
import fnmatch
import itertools
import os
import re
import sys

POINTER_SIZE = 4
MAX_PACK = 8
MAX_VARIANT_ATOMS = 5
MAX_LAYOUT_DEPTH = 40
MAX_MACRO_DEPTH = 8
HEX_BASE = 16
OCTAL_BASE = 8
OCTAL_DIGITS = 3
PRINTABLE_FIRST = 0x20
PRINTABLE_LAST = 0x7e

# Conditions fixed for every library: a 32-bit MSVC release build on Windows.
COMMON_PINNED = {
    "_MSC_VER": True, "_WIN32": True, "WIN32": True, "_M_IX86": True, "__cplusplus": None,
    "__GNUC__": False, "__BORLANDC__": False, "__MWERKS__": False, "__INTEL_COMPILER": False,
    "__SUNPRO_CC": False, "__MINGW32__": False, "__CYGWIN__": False, "macintosh": False,
    "__APPLE__": False, "_WIN64": False, "_M_X64": False, "__x86_64__": False,
    "DEBUG": False, "_DEBUG": False, "NDEBUG": True,
}

PROFILES = {
    "zlib": {
        "files": ["*.c", "*.h"],
        "exclude": ["example.c", "minigzip.c", "contrib/*", "old/*", "amiga/*", "as400/*", "msdos/*",
                    "projects/*", "qnx/*", "win32/*"],
        "drop_macros": ["Trace", "Tracev", "Tracevv", "Tracec", "Tracecv", "Assert"],
        "pinned": {"ZLIB_DEBUG": False, "DYNAMIC_CRC_TABLE": False, "MAKEFIXED": False, "BUILDFIXED": False},
        "calls": True,
    },
    "freetype": {
        "files": ["src/*.c", "src/*.h", "include/*.h"],
        "exclude": ["src/tools/*", "src/base/ftmac.c", "src/gxvalid/gxvfgen.c", "include/freetype/config/ftmodule.h"],
        "drop_macros": ["FT_TRACE", "FT_TRACE0", "FT_TRACE1", "FT_TRACE2", "FT_TRACE3", "FT_TRACE4",
                        "FT_TRACE5", "FT_TRACE6", "FT_TRACE7", "FT_ERROR", "FT_ASSERT", "FT_THROW",
                        "GXV_TRACE", "GXV_TRACE_HEXDUMP", "GXV_TRACE_HEXDUMP_C", "GXV_TRACE_HEXDUMP_SFNTNAME",
                        "OTV_TRACE", "AF_LOG", "Trace", "Tracev", "Tracevv", "Tracec", "Tracecv", "Assert"],
        "pinned": {"FT_DEBUG_LEVEL_TRACE": False, "FT_DEBUG_LEVEL_ERROR": False, "FT_DEBUG_MEMORY": False,
                   "FT_MACINTOSH": False, "FT_CONFIG_OPTION_PIC": False, "FT_CONFIG_OPTION_SYSTEM_ZLIB": False,
                   "FT_LONG64": True, "FT_SIZEOF_INT == 4": True, "FT_SIZEOF_LONG == 4": True,
                   "FT_SIZEOF_LONG == 8": False,
                   # ftoption.h defaults; the cmap formats would exceed MAX_VARIANT_ATOMS.
                   "TT_CONFIG_CMAP_FORMAT_0": True, "TT_CONFIG_CMAP_FORMAT_2": True,
                   "TT_CONFIG_CMAP_FORMAT_4": True, "TT_CONFIG_CMAP_FORMAT_6": True,
                   "TT_CONFIG_CMAP_FORMAT_8": True, "TT_CONFIG_CMAP_FORMAT_10": True,
                   "TT_CONFIG_CMAP_FORMAT_12": True},
        "replace": [(r"FT_DEFINE_SERVICE\(\s*(\w+)\s*\)",
                     r"typedef struct FT_Service_\1Rec_ FT_Service_\1Rec; "
                     r"typedef struct FT_Service_\1Rec_ const * FT_Service_\1; struct FT_Service_\1Rec_"),
                    (r"\bFT_BEGIN_HEADER\b", ""), (r"\bFT_END_HEADER\b", "")],
        # FreeType's bundled zlib 1.1.4 (src/gzip) keeps the zlib names; keep them apart.
        "path_namespaces": {"src/gzip/": "ft_zlib"},
        "calls": True,
    },
    "tinyxml": {
        "files": ["*.cpp", "*.h"],
        "exclude": ["xmltest.cpp"],
        "drop_macros": ["TIXML_LOG", "assert"],
        "pinned": {"TIXML_SAFE": True, "DEBUG_PARSER": False, "TIXML_NEEDS_SNPRINTF": False},
        "calls": False,
    },
    "cryptopp": {
        "files": ["*.cpp", "*.h"],
        "exclude": ["test.cpp", "bench.cpp", "bench2.cpp", "validat*.cpp", "datatest.cpp", "regtest.cpp",
                    "fipstest.cpp", "fipsalgt.cpp", "dlltest.cpp", "adhoc.cpp*", "bench.h", "validate.h",
                    "factory.h", "TestScripts/*", "TestVectors/*", "TestData/*"],
        "drop_macros": ["assert", "CRYPTOPP_ASSERT"],
        "pinned": {"CRYPTOPP_DEBUG": False, "CRYPTOPP_IMPORTS": False, "CRYPTOPP_EXPORTS": False,
                   "CRYPTOPP_MANUALLY_INSTANTIATE_TEMPLATES": False, "CRYPTOPP_DOXYGEN_PROCESSING": False,
                   "CRYPTOPP_IS_DLL": False},
        "replace": [(r"\bNAMESPACE_BEGIN\(\s*(\w+)\s*\)", r"namespace \1 {"), (r"\bNAMESPACE_END\b", "}"),
                    (r"\bANONYMOUS_NAMESPACE_BEGIN\b", "namespace {"), (r"\bANONYMOUS_NAMESPACE_END\b", "}"),
                    (r"\bDOCUMENTED_NAMESPACE_BEGIN\(\s*(\w+)\s*\)", r"namespace \1 {"),
                    (r"\bDOCUMENTED_NAMESPACE_END\b", "}"),
                    (r"\bCRYPTOPP_CONSTANT\(([^()]*)\)", r"enum {\1};"),
                    (r"\bUSING_NAMESPACE\(\s*\w+\s*\)", "")],
        "calls": False,
    },
}
GENERIC_PROFILE = {"files": ["*.c", "*.cpp", "*.h"], "exclude": [], "drop_macros": ["assert"], "pinned": {},
                   "calls": True}

C_KEYWORDS = {"if", "while", "for", "switch", "return", "sizeof", "do", "else", "case", "goto", "throw",
              "catch", "new", "delete", "defined", "typeof", "__typeof__", "_alloca", "alloca"}
TYPE_WORDS = {"void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "bool",
              "wchar_t", "__int64", "__int32", "__int16", "__int8", "const", "volatile", "struct", "enum",
              "union", "class", "typename"}
DECL_NOISE = {"static", "extern", "inline", "__inline", "__forceinline", "register", "auto", "mutable",
              "explicit", "local", "__cdecl", "__stdcall", "__fastcall", "WINAPI", "CALLBACK"}
ACCESS = {"public", "private", "protected"}
CLASS_KEYS = {"class", "struct", "union"}
BUILTIN_SIZES = {
    ("char",): 1, ("signed", "char"): 1, ("unsigned", "char"): 1, ("bool",): 1,
    ("short",): 2, ("short", "int"): 2, ("signed", "short"): 2, ("unsigned", "short"): 2,
    ("unsigned", "short", "int"): 2, ("wchar_t",): 2, ("__int16",): 2, ("__int8",): 1,
    ("int",): 4, ("signed",): 4, ("unsigned",): 4, ("signed", "int"): 4, ("unsigned", "int"): 4,
    ("long",): 4, ("long", "int"): 4, ("signed", "long"): 4, ("unsigned", "long"): 4,
    ("unsigned", "long", "int"): 4, ("signed", "long", "int"): 4, ("__int32",): 4, ("size_t",): 4,
    ("ptrdiff_t",): 4, ("float",): 4,
    ("double",): 8, ("long", "long"): 8, ("unsigned", "long", "long"): 8, ("__int64",): 8,
    ("unsigned", "__int64",): 8, ("signed", "__int64"): 8,
}
FLOAT_TYPES = {("float",), ("double",)}
MACRO_LIKE = re.compile(r"^[A-Z][A-Z0-9_]*$")

TOKEN = re.compile(r'''
    (?P<mark>@(?:if|elif)\d+|@else\b|@endif\b)
  | (?P<str>"(?:[^"\\\n]|\\.)*")
  | (?P<chr>'(?:[^'\\\n]|\\.)*')
  | (?P<id>[A-Za-z_]\w*)
  | (?P<num>\.?\d(?:[\w.]|[eE][+-])*)
  | (?P<op>::|->|<<|>>|&&|\|\||[<>=!]=|\#\#)
  | (?P<punct>[^\s\w])
''', re.VERBOSE)
DEFINE = re.compile(r"(\w+)(\(([^)]*)\))?\s*(.*)", re.S)
ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b", "f": "\f", "v": "\v",
           "\\": "\\", "'": "'", '"': '"', "?": "?"}


class Token:
    __slots__ = ("kind", "text", "pos", "cons")

    def __init__(self, kind, text, pos=0, cons=frozenset()):
        self.kind = kind
        self.text = text
        self.pos = pos
        self.cons = cons

    def __repr__(self):
        return self.text


# ---- Text ----

def strip_comments(text):
    """Replace comments by a space, keeping string and char literals and line breaks."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c in "\"'":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(" " + "\n" * text.count("\n", i, j))
            i = j
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            out.append(" ")
        else:
            out.append(c)
            i += 1
    return "".join(out)


def preprocess(text, replacements):
    """Return (code with conditional markers, #define bodies, conditions). Directive lines are
    blanked; #if/#ifdef/#ifndef/#elif become @ifN/@elifN referring to conditions[N]."""
    lines = strip_comments(text).split("\n")
    out = []
    defines = []
    conditions = []
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line.lstrip().startswith("#"):
            for pattern, repl in replacements:
                line = re.sub(pattern, repl, line)
            out.append(line)
            i += 1
            continue
        directive = line
        continuation = 0
        while directive.endswith("\\") and i + 1 < len(lines):
            i += 1
            directive = directive[:-1] + " " + lines[i]
            continuation += 1
        m = re.match(r"\s*#\s*(\w*)\s*(.*)", directive, re.S)
        word, rest = m.group(1), m.group(2).strip()
        if word in ("if", "ifdef", "ifndef", "elif"):
            if word == "ifndef" and include_guard(lines, i + 1, rest):
                word, rest = "if", "1"
            conditions.append((word, rest))
            out.append("@%s%d" % ("elif" if word == "elif" else "if", len(conditions) - 1))
        elif word == "else":
            out.append("@else")
        elif word == "endif":
            out.append("@endif")
        else:
            out.append("")
            if word == "define":
                defines.append(rest)
        out.extend([""] * continuation)
        i += 1
    return "\n".join(out), defines, conditions


def include_guard(lines, start, rest):
    """True when `#ifndef NAME` is followed by `#define NAME` (an include guard)."""
    for line in lines[start:]:
        if line.strip():
            return re.fullmatch(r"\s*#\s*define\s+%s\s*" % re.escape(rest.strip()), line) is not None
    return False


def unescape(literal):
    body = literal[1:-1]
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        e = body[i + 1] if i + 1 < len(body) else "\\"
        if e == "x":
            j = i + 2
            while j < len(body) and body[j] in "0123456789abcdefABCDEF":
                j += 1
            out.append(chr(int(body[i + 2:j] or "0", HEX_BASE) & 0xff))
            i = j
        elif e in "01234567":
            j = i + 1
            while j < len(body) and j < i + 1 + OCTAL_DIGITS and body[j] in "01234567":
                j += 1
            out.append(chr(int(body[i + 1:j], OCTAL_BASE) & 0xff))
            i = j
        else:
            out.append(ESCAPES.get(e, e))
            i += 2
    return "".join(out)


def escape_field(value):
    out = []
    for c in str(value):
        if c == "\\":
            out.append("\\\\")
        elif c == "\t":
            out.append("\\t")
        elif c == "\n":
            out.append("\\n")
        elif c == "\r":
            out.append("\\r")
        elif PRINTABLE_FIRST <= ord(c) <= PRINTABLE_LAST:
            out.append(c)
        else:
            out.append("\\x%02x" % (ord(c) & 0xff))
    return "".join(out)


def tokenize(code):
    tokens = []
    for m in TOKEN.finditer(code):
        kind = m.lastgroup
        if kind == "str" and tokens and tokens[-1].kind == "id" and tokens[-1].text == "L" \
                and tokens[-1].pos + 1 == m.start():
            tokens[-1] = Token("wstr", m.group(), tokens[-1].pos)
            continue
        tokens.append(Token(kind, m.group(), m.start()))
    return tokens


# ---- Conditions ----

def condition_literal(word, rest):
    """A condition as (atom, polarity), True or False."""
    rest = re.sub(r"\s+", " ", rest).strip()
    if word in ("ifdef", "ifndef"):
        name = rest.split(" ")[0] if rest else ""
        return (name, word == "ifdef")
    if rest in ("0", "(0)"):
        return False
    if rest in ("1", "(1)"):
        return True
    m = re.fullmatch(r"(!\s*)?defined\s*\(?\s*(\w+)\s*\)?", rest)
    if m:
        return (m.group(2), not m.group(1))
    m = re.fullmatch(r"!\s*\(?\s*(\w+)\s*\)?", rest)
    if m:
        return (m.group(1), False)
    return (rest, True)


def negate(literal):
    if literal is True or literal is False:
        return not literal
    return (literal[0], not literal[1])


def pin(literal, pinned):
    if literal is True or literal is False:
        return literal
    value = pinned.get(literal[0], None)
    return literal if value is None else value == literal[1]


def resolve_conditionals(tokens, conditions, pinned):
    """Drop tokens of dead branches, annotate the others with their condition set (frozenset of
    (atom, polarity)). Where branches of one group change the brace depth differently, only the
    first live branch keeps its braces, so blocks stay balanced."""
    out = []
    frames = []  # [group id, prior literals, current constraint or None (dead), branch index]
    group_ids = itertools.count()
    paths = []
    for token in tokens:
        if token.kind == "mark":
            text = token.text
            if text.startswith("@if"):
                literal = pin(condition_literal(*conditions[int(text[3:])]), pinned)
                frames.append([next(group_ids), [literal], branch_constraint([], literal), 0])
            elif text.startswith("@elif") and frames:
                frame = frames[-1]
                literal = pin(condition_literal("if", conditions[int(text[5:])][1]), pinned)
                frame[2] = branch_constraint(frame[1], literal)
                frame[1].append(literal)
                frame[3] += 1
            elif text == "@else" and frames:
                frame = frames[-1]
                frame[2] = branch_constraint(frame[1], True)
                frame[1].append(True)
                frame[3] += 1
            elif text == "@endif" and frames:
                frames.pop()
            continue
        cons = set()
        dead = False
        for frame in frames:
            if frame[2] is None:
                dead = True
                break
            cons |= frame[2]
        if dead or contradictory(cons):
            continue
        token.cons = frozenset(cons)
        out.append(token)
        paths.append(tuple((frame[0], frame[3]) for frame in frames))
    # Brace deltas per (group, branch).
    deltas = {}
    branches = {}
    for token, path in zip(out, paths):
        for group, branch in path:
            branches.setdefault(group, set()).add(branch)
            if token.text in "{}" and token.kind == "punct":
                key = (group, branch)
                deltas[key] = deltas.get(key, 0) + (1 if token.text == "{" else -1)
    unbalanced = set()
    for group, seen in branches.items():
        values = {deltas.get((group, branch), 0) for branch in seen}
        if len(values) > 1 or values != {0}:
            first = min(seen)
            if len(values) > 1 or deltas.get((group, first), 0) != 0:
                unbalanced.add((group, first))
    result = []
    for token, path in zip(out, paths):
        if token.kind == "punct" and token.text in "{}":
            drop = False
            for group, branch in path:
                if any(g == group for g, _ in unbalanced) and (group, branch) not in unbalanced:
                    drop = True
            if drop:
                continue
        result.append(token)
    return result


def branch_constraint(prior, literal):
    cons = set()
    for previous in prior:
        negated = negate(previous)
        if negated is False:
            return None
        if negated is not True:
            cons.add(negated)
    if literal is False:
        return None
    if literal is not True:
        cons.add(literal)
    return None if contradictory(cons) else frozenset(cons)


def contradictory(cons):
    atoms = {}
    for atom, polarity in cons:
        if atoms.setdefault(atom, polarity) != polarity:
            return True
    return False


def satisfied(cons, assignment):
    return all(assignment.get(atom, False) == polarity for atom, polarity in cons)


# ---- Macros ----

class Macros:
    """String macros (NAME -> literal text), numeric macros and function-like macros."""

    def __init__(self):
        self.object = {}
        self.function = {}
        self.strings = {}

    def add(self, directive):
        m = DEFINE.match(directive)
        if not m:
            return
        name, params, body = m.group(1), m.group(3), m.group(4).strip()
        if m.group(2) is not None and directive[len(name)] == "(":
            self.function[name] = ([p.strip() for p in params.split(",") if p.strip()], tokenize(body))
        else:
            self.object.setdefault(name, tokenize(body))

    def resolve(self):
        changed = True
        while changed:
            changed = False
            for name, body in self.object.items():
                if name in self.strings:
                    continue
                value = self.expand_strings(body, 0)
                if value is not None and value[1] == len(body):
                    self.strings[name] = value[0]
                    changed = True

    def expand_strings(self, tokens, i):
        """Concatenate adjacent literals and string macros from tokens[i]; (text, next) or None."""
        parts = []
        while i < len(tokens):
            token = tokens[i]
            if token.kind == "str":
                parts.append(unescape(token.text))
                i += 1
            elif token.kind == "id" and token.text in self.strings:
                parts.append(self.strings[token.text])
                i += 1
            elif token.text == "(" and i + 2 < len(tokens) and tokens[i + 2].text == ")" \
                    and (tokens[i + 1].kind == "str" or tokens[i + 1].text in self.strings):
                inner = self.expand_strings(tokens, i + 1)
                parts.append(inner[0])
                i += 3
            else:
                break
        return ("".join(parts), i) if parts else None

    def number(self, tokens, depth=0):
        """Integer value of an expression of literals and numeric macros, or None."""
        expanded = []
        for token in tokens:
            if token.kind == "id" and token.text in self.object and depth < MAX_MACRO_DEPTH:
                value = self.number(self.object[token.text], depth + 1)
                if value is None:
                    return None
                expanded.append(Token("num", str(value), token.pos))
            elif token.kind in ("num", "punct", "op", "chr"):
                expanded.append(token)
            else:
                return None
        return evaluate(expanded)

    def literals_in(self, name, depth=0):
        """Literals produced by expanding the body of a function-like macro."""
        found = []
        if depth > MAX_MACRO_DEPTH or name not in self.function:
            return found
        body = self.function[name][1]
        i = 0
        while i < len(body):
            token = body[i]
            if token.text == "#" or token.text == "##":
                i += 2
                continue
            value = self.expand_strings(body, i) if token.kind in ("str", "id") else None
            if value is not None and (token.kind == "str" or token.text in self.strings):
                found.append(value[0])
                i = value[1]
                continue
            if token.kind == "id" and token.text in self.function:
                found.extend(self.literals_in(token.text, depth + 1))
            i += 1
        return found

    def calls_in(self, name, depth=0):
        found = set()
        if depth > MAX_MACRO_DEPTH or name not in self.function:
            return found
        body = self.function[name][1]
        for i, token in enumerate(body[:-1]):
            if token.kind == "id" and body[i + 1].text == "(":
                if token.text in self.function:
                    found |= self.calls_in(token.text, depth + 1)
                else:
                    found.add(token.text)
        return found


def evaluate(tokens):
    """Integer constant expression: literals, unary - ~ !, binary * / % + - << >> & ^ |."""
    position = [0]

    def peek():
        return tokens[position[0]].text if position[0] < len(tokens) else None

    def take():
        position[0] += 1
        return tokens[position[0] - 1]

    def primary():
        token = take() if peek() is not None else None
        if token is None:
            raise ValueError
        if token.text == "(":
            value = binary(0)
            if peek() != ")":
                raise ValueError
            take()
            return value
        if token.text in ("-", "~", "!", "+"):
            value = primary()
            return {"-": -value, "~": ~value, "!": int(not value), "+": value}[token.text]
        if token.kind == "chr":
            text = unescape(token.text)
            value = 0
            for c in text:
                value = (value << 8) | ord(c)
            return value
        if token.kind != "num":
            raise ValueError
        text = token.text.rstrip("uUlL")
        return int(text, 0) if not re.fullmatch(r"0\d+", text) else int(text, OCTAL_BASE)

    levels = [("|",), ("^",), ("&",), ("<<", ">>"), ("+", "-"), ("*", "/", "%")]

    def binary(level):
        if level == len(levels):
            return primary()
        value = binary(level + 1)
        while peek() in levels[level]:
            op = take().text
            right = binary(level + 1)
            value = {"|": value | right, "^": value ^ right, "&": value & right, "<<": value << right,
                     ">>": value >> right, "+": value + right, "-": value - right, "*": value * right,
                     "/": value // right if right else 0, "%": value % right if right else 0}[op]
        return value

    try:
        value = binary(0)
        return value if position[0] == len(tokens) else None
    except (ValueError, IndexError):
        return None


# ---- Model ----

class Member:
    def __init__(self, name, sig, virtual, pure, dtor, cons):
        self.name = name
        self.sig = sig
        self.virtual = virtual
        self.pure = pure
        self.dtor = dtor
        self.cons = cons


class ClassInfo:
    def __init__(self, qualified, scope, file_name):
        self.qualified = qualified
        self.scope = scope
        self.file = file_name
        self.params = []
        self.defaults = []
        self.bases = []  # (text, virtual, cons)
        self.members = []
        self.fields = []  # (type tokens, declarator tokens, cons) for structs
        self.union = False
        self.specialization = False
        self.unsure = False


class Variable:
    def __init__(self, file_name, name, type_tokens, array, init, cons):
        self.file = file_name
        self.name = name
        self.type_tokens = type_tokens
        self.array = array
        self.init = init
        self.cons = cons


class Library:
    def __init__(self, profile):
        self.profile = profile
        self.macros = Macros()
        self.functions = {}  # (file, name) -> flags
        self.strings = {}  # text -> set of (file, function, inline)
        self.calls = {}  # (file, function) -> set of callees
        self.classes = {}
        self.typedefs = {}  # name -> list of (base tokens, declarator tokens, cons)
        self.variables = []
        self.instantiations = set()  # (template-id text, scope tuple)
        self.sources = []


# ---- Parsing ----

def matching(tokens, i, open_text, close_text, end=None):
    depth = 0
    end = len(tokens) if end is None else end
    for j in range(i, end):
        text = tokens[j].text
        if text == open_text and tokens[j].kind == "punct":
            depth += 1
        elif text == close_text and tokens[j].kind == "punct":
            depth -= 1
            if depth == 0:
                return j
    return end - 1


def matching_angle(tokens, i):
    depth = 0
    for j in range(i, len(tokens)):
        text = tokens[j].text
        if text == "<":
            depth += 1
        elif text == ">":
            depth -= 1
        elif text == ">>":
            depth -= 2
        elif text in (";", "{", "}"):
            return -1
        if depth <= 0 and j > i:
            return j
    return -1


def split_top(tokens, separator=","):
    """Split on separator outside (), [], {} and template brackets."""
    parts = [[]]
    depth = 0
    angle = 0
    for token in tokens:
        text = token.text
        if text in "([{" and token.kind == "punct":
            depth += 1
        elif text in ")]}" and token.kind == "punct":
            depth -= 1
        elif text == "<" and depth == 0:
            angle += 1
        elif text == ">" and depth == 0 and angle > 0:
            angle -= 1
        elif text == ">>" and depth == 0 and angle > 1:
            angle -= 2
        if text == separator and depth == 0 and angle == 0:
            parts.append([])
        else:
            parts[-1].append(token)
    return parts


def join_type(tokens):
    out = ""
    previous = None
    for token in tokens:
        text = ">>" if token.text == ">>" else token.text
        if previous is not None and (previous.kind in ("id", "num") and token.kind in ("id", "num")):
            out += " "
        out += "> >" if text == ">>" else text
        previous = token
    return out


def strip_template_prefix(tokens):
    """Remove a leading template <...> (possibly several); return (rest, parameter lists)."""
    params = []
    while len(tokens) > 1 and tokens[0].text == "template" and tokens[1].text == "<":
        close = matching_angle(tokens, 1)
        if close < 0:
            break
        params.append(tokens[2:close])
        tokens = tokens[close + 1:]
    return tokens, params


def strip_noise(tokens):
    """Drop leading storage keywords and macro-like words (FT_CALLBACK_TABLE_DEF, ZEXTERN)."""
    i = 0
    while i < len(tokens):
        token = tokens[i]
        nxt = tokens[i + 1].text if i + 1 < len(tokens) else ""
        if token.kind == "id" and (token.text in DECL_NOISE or MACRO_LIKE.match(token.text) and nxt != "("
                                   and nxt != "::" and nxt != "<"):
            i += 1
        else:
            break
    return tokens[i:]


def function_name(header):
    """(qualified name parts, index of the parameter '(') of a function header, or None.
    Skips prefix macro calls such as FT_EXPORT_DEF( FT_Error )."""
    i = 0
    n = len(header)
    while i < n:
        token = header[i]
        if token.text == "(" and token.kind == "punct":
            close = matching(header, i, "(", ")")
            previous = header[i - 1] if i > 0 else None
            after = header[close + 1] if close + 1 < n else None
            if previous is not None and previous.kind == "id" and MACRO_LIKE.match(previous.text) \
                    and after is not None and (after.kind == "id" or after.text in ("*", "&")):
                i = close + 1
                continue
            # LUA_API int (lua_gettop) (lua_State *L)
            if close == i + 2 and header[i + 1].kind == "id" and after is not None and after.text == "(":
                return [header[i + 1].text], close + 1
            if previous is None:
                return None
            parts = name_before(header, i)
            return (parts, i) if parts else None
        if token.text == ";" or token.text == "=" and not (i > 0 and header[i - 1].text == "operator"):
            return None
        i += 1
    return None


def name_before(header, i):
    """Qualified name ending just before index i: A::B<T>::~C, operator==, ..."""
    j = i - 1
    # operator symbols
    k = j
    while k >= 0 and header[k].kind in ("punct", "op") and header[k].text not in (")", ">", "]"):
        k -= 1
    if k >= 0 and header[k].text == "operator":
        if k == j:
            return None
        name = "operator" + "".join(t.text for t in header[k + 1:i])
        j = k - 1
        parts = [name]
    elif k >= 0 and header[k].text == "operator" and header[j].text in (")", "]"):
        return None
    elif header[j].kind == "id":
        if header[j].text in C_KEYWORDS:
            return None
        parts = [header[j].text]
        if j > 0 and header[j - 1].text == "~":
            parts = ["~" + header[j].text]
            j -= 1
        j -= 1
    else:
        return None
    while j >= 1 and header[j].text == "::":
        j -= 1
        if header[j].text == ">":
            depth = 0
            while j >= 0:
                if header[j].text == ">":
                    depth += 1
                elif header[j].text == "<":
                    depth -= 1
                    if depth == 0:
                        break
                j -= 1
            j -= 1
        if j < 0 or header[j].kind != "id":
            break
        parts.insert(0, header[j].text)
        j -= 1
    return parts


def normalize_params(tokens):
    params = []
    for part in split_top(tokens):
        cut = []
        depth = 0
        for token in part:
            if token.text in "([" and token.kind == "punct":
                depth += 1
            elif token.text in ")]" and token.kind == "punct":
                depth -= 1
            if token.text == "=" and depth == 0:
                break
            cut.append(token)
        cut = [t for t in cut if t.text not in ("class", "struct", "typename", "register")]
        if len(cut) > 1 and cut[-1].kind == "id" and cut[-1].text not in TYPE_WORDS:
            cut = cut[:-1]
        params.append(join_type(cut))
    if params == ["void"] or params == [""]:
        return ""
    return ",".join(params)


class Parser:
    def __init__(self, library, file_name, tokens, namespace_prefix):
        self.lib = library
        self.file = file_name
        self.tokens = tokens
        self.prefix = namespace_prefix
        self.is_c = file_name.endswith(".c")

    def run(self):
        self.parse_scope(0, len(self.tokens), [], None)

    def qualified(self, scope, parts):
        names = list(scope) + list(parts)
        if self.prefix:
            names.insert(0, self.prefix)
        return "::".join(names)

    def parse_scope(self, i, end, scope, owner):
        tokens = self.tokens
        start = i
        pending = []
        depth = 0
        while i < end:
            token = tokens[i]
            text = token.text
            if token.kind == "punct" and text in "([":
                depth += 1
            elif token.kind == "punct" and text in ")]":
                depth -= 1
            elif depth == 0 and text == ":" and owner is not None and i - start == 1 \
                    and tokens[start].text in ACCESS:
                start = i + 1
            elif depth == 0 and text == ";" and token.kind == "punct":
                statement = tokens[start:i]
                self.statement(statement, scope, owner)
                pending.append(statement)
                start = i + 1
            elif depth == 0 and text == "}" and token.kind == "punct":
                start = i + 1
            elif depth == 0 and text == "{" and token.kind == "punct":
                close = matching(tokens, i, "{", "}", end)
                next_start = self.block(tokens[start:i], i, close, scope, owner, pending)
                pending = []
                i = next_start
                start = i
                depth = 0
                continue
            i += 1

    def block(self, header, open_index, close, scope, owner, pending):
        """Handle a block; return the index where the next statement starts."""
        tokens = self.tokens
        header = strip_noise(header)
        texts = [t.text for t in header]
        if texts[:1] == ["namespace"]:
            names = [t.text for t in header[1:] if t.kind == "id"]
            self.parse_scope(open_index + 1, close, scope + names[:1], None)
            return close + 1
        if texts[:1] == ["extern"] and len(header) == 2 and header[1].kind == "str":
            self.parse_scope(open_index + 1, close, scope, owner)
            return close + 1
        if has_top_assign(header):
            after = close + 1
            while after < len(tokens) and tokens[after].text != ";":
                after += 1
            if owner is None:
                self.variable(header, open_index, close, scope)
            return after + 1
        rest, template_params = strip_template_prefix(header)
        typedef = bool(rest) and rest[0].text == "typedef"
        if typedef:
            rest = rest[1:]
        rest = strip_noise(rest)
        if rest and rest[0].text in CLASS_KEYS | {"enum"} and "(" not in [t.text for t in rest]:
            info = None
            if rest[0].text != "enum":
                info = self.class_block(rest, template_params, open_index, close, scope, owner)
            after = close + 1
            tail_start = after
            while after < len(tokens) and tokens[after].text != ";":
                after += 1
            if typedef:
                self.typedef_names(tokens[tail_start:after], info, rest)
            return after + 1
        if "(" in texts:
            self.function(header, open_index, close, scope, owner)
            return close + 1
        if not header and pending and owner is None:
            # K&R definition: int f(a, b) int a; int b; { -- the parameter declarations
            # follow a header whose parentheses hold identifiers only.
            for candidate in reversed(pending):
                found = kr_header(candidate)
                if found:
                    self.function(strip_noise(found), open_index, close, scope, owner)
                    break
                if not candidate or any(t.text in ("=", "{") for t in candidate):
                    break
        return close + 1

    def class_block(self, rest, template_params, open_index, close, scope, owner):
        keyword = rest[0].text
        colon = None
        angle = 0
        for k, token in enumerate(rest):
            if token.text == "<":
                angle += 1
            elif token.text == ">":
                angle -= 1
            elif token.text == ":" and angle == 0 and colon is None:
                colon = k
        name_part = rest[1:colon] if colon is not None else rest[1:]
        specialization = any(t.text == "<" for t in name_part)
        if specialization:
            name_part = name_part[:[t.text for t in name_part].index("<")]
        ids = [t for t in name_part if t.kind == "id"]
        name = ids[-1].text if ids else None
        qualified = self.qualified(scope, [name]) if name else None
        info = ClassInfo(qualified, list(scope) + ([name] if name else []), self.file)
        info.union = keyword == "union"
        info.specialization = specialization or bool(template_params and not template_params[-1])
        if template_params and template_params[-1]:
            for part in split_top(template_params[-1]):
                texts = [t.text for t in part]
                cut = texts.index("=") if "=" in texts else len(part)
                ids = [t.text for t in part[:cut] if t.kind == "id"]
                info.params.append(ids[-1] if ids else "")
                info.defaults.append(join_type(part[cut + 1:]) if cut < len(part) else None)
        if colon is not None:
            for part in split_top(rest[colon + 1:]):
                virtual = any(t.text == "virtual" for t in part)
                base = [t for t in part if t.text not in ACCESS and t.text != "virtual"]
                if base:
                    info.bases.append((join_type(base), virtual, base[0].cons))
                    self.note_instantiation(base, info.scope if name else scope)
        if name and (qualified not in self.lib.classes or self.lib.classes[qualified].specialization):
            if not info.specialization or qualified not in self.lib.classes:
                self.lib.classes[qualified] = info if not info.specialization else self.lib.classes.get(
                    qualified, info)
        self.parse_scope(open_index + 1, close, info.scope if name else scope, info)
        return info

    def note_instantiation(self, tokens, scope):
        if any(t.text == "<" for t in tokens) and not any(t.text in ("(", "=") for t in tokens):
            self.lib.instantiations.add((join_type(tokens), tuple(scope)))

    def typedef_names(self, tail, info, rest):
        for part in split_top(tail):
            ids = [t for t in part if t.kind == "id"]
            if not ids:
                continue
            pointer = any(t.text == "*" for t in part)
            if info is not None and info.qualified:
                base = [Token("id", "struct"), Token("id", info.qualified.split("::")[-1])]
            else:
                base = rest[:1]
            self.lib.typedefs.setdefault(ids[-1].text, []).append(
                (base, [Token("punct", "*")] if pointer else [], ids[-1].cons))

    def statement(self, statement, scope, owner):
        statement = strip_noise(statement)
        if not statement:
            return
        texts = [t.text for t in statement]
        if texts[0] == "typedef":
            self.typedef(statement[1:], scope)
            return
        if owner is not None:
            if owner.qualified and not owner.union and "(" in texts:
                self.member(statement, owner, False)
            elif "(" not in texts or self.function_pointer_field(statement):
                owner.fields.append(statement)
            elif owner.qualified:
                owner.unsure = True

    @staticmethod
    def function_pointer_field(statement):
        texts = [t.text for t in statement]
        for k in range(len(texts) - 2):
            if texts[k] == "(" and texts[k + 1] == "*" and statement[k + 2].kind == "id":
                return True
        return False

    def typedef(self, statement, scope):
        texts = [t.text for t in statement]
        for k in range(len(texts) - 3):
            if texts[k] == "(" and texts[k + 1] == "*" and statement[k + 2].kind == "id" and texts[k + 3] == ")":
                self.lib.typedefs.setdefault(texts[k + 2], []).append(
                    ([Token("id", "void")], [Token("punct", "*")], statement[k + 2].cons))
                return
        if "(" in texts:
            return
        if any(t.text == "<" for t in statement):
            self.note_instantiation(statement[:-1], scope)
        parts = split_top(statement)
        first = parts[0]
        ids = [k for k, t in enumerate(first) if t.kind == "id"]
        if not ids:
            return
        name_index = ids[-1]
        base = [t for t in first[:name_index] if t.text != "*"]
        declarators = [first[name_index - 1:name_index + 1]] + parts[1:]
        for declarator in declarators:
            names = [t for t in declarator if t.kind == "id"]
            if not names:
                continue
            pointer = [Token("punct", "*")] if any(t.text == "*" for t in declarator) else []
            array = [t for t in declarator if t.text in "[]" or t.kind == "num"]
            self.lib.typedefs.setdefault(names[-1].text, []).append((base, pointer + array, names[-1].cons))

    def member(self, header, owner, inline_definition):
        rest, template_params = strip_template_prefix(header)
        texts = [t.text for t in rest]
        if any(t in texts for t in ("friend", "typedef", "using")):
            return None
        found = function_name(rest)
        if not found:
            return None
        parts, open_index = found
        close = matching(rest, open_index, "(", ")")
        name = parts[-1]
        tail = [t.text for t in rest[close + 1:]]
        const = tail[:1] == ["const"]
        pure = tail[-2:] == ["=", "0"]
        virtual = "virtual" in texts[:open_index]
        simple = name.split("<")[0]
        dtor = simple.startswith("~")
        owner_name = owner.qualified.split("::")[-1] if owner.qualified else ""
        if simple == owner_name or len(parts) > 1 or template_params:
            return None
        sig = normalize_params(rest[open_index + 1:close]) + ("|const" if const else "")
        owner.members.append(Member(simple, sig, virtual, pure, dtor, rest[open_index - 1].cons))
        return simple

    def function(self, header, open_index, close, scope, owner):
        rest, _ = strip_template_prefix(header)
        found = function_name(rest)
        if not found:
            return
        parts, _ = found
        parts = [re.sub(r"<.*", "", p) if not p.startswith("operator") else p for p in parts]
        inline = owner is not None or self.file.endswith((".h", ".hpp", ".inl")) \
            or any(t.text in ("inline", "__inline", "__forceinline") for t in header)
        if owner is not None:
            self.member(header, owner, True)
        if owner is not None and not owner.qualified:
            return
        name = self.qualified(scope, parts)
        flags = "i" if inline else ""
        key = (self.file, name)
        self.lib.functions[key] = flags
        self.body(open_index, close, key, inline)

    def body(self, start, end, key, inline):
        tokens = self.tokens
        macros = self.lib.macros
        drop = self.lib.profile["drop_macros"]
        calls = self.lib.calls.setdefault(key, set())
        i = start + 1
        while i < end:
            token = tokens[i]
            if token.kind == "id" and token.text in drop and i + 1 < end and tokens[i + 1].text == "(":
                i = matching(tokens, i + 1, "(", ")", end) + 1
                continue
            if token.kind == "id" and i + 1 < end and tokens[i + 1].text == "(" and token.text not in C_KEYWORDS:
                previous = tokens[i - 1].text
                if token.text in macros.function:
                    for text in macros.literals_in(token.text):
                        self.use(text, key, inline)
                    if self.is_c:
                        calls.update(macros.calls_in(token.text))
                elif self.is_c and previous not in (".", "->", "::"):
                    calls.add(token.text)
            value = macros.expand_strings(tokens, i) if token.kind in ("str", "id") else None
            if value is not None and (token.kind == "str" or token.text in macros.strings):
                self.use(value[0], key, inline)
                i = value[1]
                continue
            i += 1

    def use(self, text, key, inline):
        if text:
            self.lib.strings.setdefault(text, set()).add((key[0], key[1], inline))

    def variable(self, header, open_index, close, scope):
        texts = [t.text for t in header]
        assign = top_assign_index(header)
        before = header[:assign]
        array = "[" in [t.text for t in before]
        if array:
            before = before[:[t.text for t in before].index("[")]
        ids = [t for t in before if t.kind == "id"]
        if len(ids) < 2:
            return
        name = ids[-1].text
        type_tokens = [t for t in before[:-1] if t.text not in DECL_NOISE and t.text not in ("const", "volatile")
                       and not (t.kind == "id" and MACRO_LIKE.match(t.text))]
        if "::" in texts[:assign] or self.prefix and False:
            return
        self.lib.variables.append(Variable(self.file, name, type_tokens, array,
                                           self.tokens[open_index:close + 1], ids[-1].cons))


def kr_header(statement):
    """The `type name(a, b)` part of a K&R definition's first statement (header plus the first
    parameter declaration, `int f(a, b) int a`), or None."""
    for close in range(len(statement) - 1, 1, -1):
        if statement[close].text != ")":
            continue
        open_index = close - 1
        while open_index >= 0 and statement[open_index].text != "(":
            open_index -= 1
        inner = statement[open_index + 1:close]
        if open_index >= 1 and statement[open_index - 1].kind == "id" and inner \
                and all(t.kind == "id" or t.text == "," for t in inner) \
                and all(t.text != "(" for t in statement[:open_index]):
            return statement[:close + 1]
    return None


def has_top_assign(header):
    return top_assign_index(header) >= 0


def top_assign_index(header):
    depth = 0
    for k, token in enumerate(header):
        if token.kind == "punct" and token.text in "([":
            depth += 1
        elif token.kind == "punct" and token.text in ")]":
            depth -= 1
        elif token.text == "=" and token.kind == "punct" and depth == 0 \
                and not (k > 0 and header[k - 1].text == "operator"):
            return k
    return -1


# ---- Vtables ----

class Slot:
    __slots__ = ("name", "sig", "owner", "pure", "dtor")

    def __init__(self, name, sig, owner, pure, dtor):
        self.name = name
        self.sig = sig
        self.owner = owner
        self.pure = pure
        self.dtor = dtor

    def key(self):
        return (self.name, self.sig, self.owner, self.pure, self.dtor)


class Layouts:
    def __init__(self, library):
        self.lib = library
        self.cache = {}

    def lookup(self, name, scope):
        parts = name.split("::")
        if parts[0] == "":
            parts = parts[1:]
        for k in range(len(scope), -1, -1):
            candidate = "::".join(list(scope[:k]) + parts)
            if candidate in self.lib.classes:
                return candidate
        return None

    def resolve(self, text, scope, subst, assignment, depth):
        """(class qualified name, args list of (text, scope)) for a type text, or None."""
        tokens = tokenize(text)
        if len(tokens) == 1 and tokens[0].text in subst:
            arg_text, arg_scope = subst[tokens[0].text]
            return self.resolve(arg_text, arg_scope, {}, assignment, depth + 1) if depth < MAX_LAYOUT_DEPTH else None
        tokens = [t for t in tokens if t.text not in ("class", "struct", "typename")]
        tokens = [part for t in tokens for part in ([Token("op", ">"), Token("op", ">")] if t.text == ">>" else [t])]
        angle = [t.text for t in tokens].index("<") if "<" in [t.text for t in tokens] else -1
        if angle >= 0:
            close = matching_angle(tokens, angle)
            if close != len(tokens) - 1:
                return None
            name = join_type(tokens[:angle])
            args = [(substitute(join_type(part), subst), scope) for part in split_top(tokens[angle + 1:close])]
        else:
            name = join_type(tokens)
            args = []
        if name in subst:
            return None
        qualified = self.lookup(name, scope)
        if qualified is None:
            return None
        return qualified, args

    def layout(self, qualified, args, assignment, depth=0):
        key = (qualified, tuple(a[0] for a in args), tuple(sorted(assignment.items())))
        if key in self.cache:
            return self.cache[key]
        self.cache[key] = None
        result = self.compute(qualified, args, assignment, depth) if depth < MAX_LAYOUT_DEPTH else None
        self.cache[key] = result
        return result

    def display(self, qualified, args):
        info = self.lib.classes[qualified]
        if not info.params:
            return qualified
        return qualified + "<" + ",".join(a[0] for a in args) + ">"

    def compute(self, qualified, args, assignment, depth):
        info = self.lib.classes[qualified]
        if info.specialization or info.union:
            return None
        subst = {}
        args = list(args)
        for k, param in enumerate(info.params):
            if k < len(args):
                subst[param] = args[k]
            elif info.defaults[k] is not None:
                value = (substitute(info.defaults[k], subst), info.scope[:-1])
                args.append(value)
                subst[param] = value
            else:
                return None
        if len(args) > len(info.params):
            return None
        display = self.display(qualified, args)
        bases = []
        for text, virtual, cons in info.bases:
            if not satisfied(cons, assignment):
                continue
            if virtual:
                return None
            resolved = self.resolve(text, info.scope, subst, assignment, depth)
            if resolved is None:
                return None
            layout = self.layout(resolved[0], resolved[1], assignment, depth + 1)
            if layout is None:
                return None
            bases.append(layout)
        slots = []
        primary = next((b for b in bases if b[0]), None)
        secondary = set()
        secondary_dtor = False
        for base in bases:
            if base is primary:
                continue
            for slot in base[0]:
                secondary.add((slot.name, slot.sig))
                secondary_dtor |= slot.dtor
        if primary is not None:
            slots = [Slot(*s.key()) for s in primary[0]]
        new = []
        declared_dtor = False
        for member in info.members:
            if not satisfied(member.cons, assignment):
                continue
            if member.dtor:
                declared_dtor = True
                index = next((k for k, s in enumerate(slots) if s.dtor), -1)
                if index >= 0:
                    slots[index] = Slot("~" + qualified.split("::")[-1], "", display, False, True)
                elif member.virtual and not secondary_dtor:
                    new.append(Slot("~" + qualified.split("::")[-1], "", display, member.pure, True))
                continue
            index = next((k for k, s in enumerate(slots) if s.name == member.name and s.sig == member.sig), -1)
            if index >= 0:
                slots[index] = Slot(member.name, member.sig, display, member.pure, False)
            elif (member.name, member.sig) in secondary:
                continue
            elif member.virtual:
                new.append(Slot(member.name, member.sig, display, member.pure, False))
        if not declared_dtor:
            # The compiler-generated destructor overrides a virtual one in every class.
            index = next((k for k, s in enumerate(slots) if s.dtor), -1)
            if index >= 0:
                slots[index] = Slot("~" + qualified.split("::")[-1], "", display, False, True)
        groups = []
        by_name = {}
        for slot in new:
            if slot.name not in by_name:
                by_name[slot.name] = []
                groups.append(by_name[slot.name])
            by_name[slot.name].append(slot)
        # MSVC groups the new overloads of one name at the first one, in reverse declaration order.
        for group in groups:
            slots.extend(reversed(group))
        return (slots, display)

    def atoms(self, qualified, seen=None):
        """Unpinned condition atoms of a class and its bases (by name, not instantiated)."""
        seen = set() if seen is None else seen
        if qualified in seen or qualified not in self.lib.classes:
            return set()
        seen.add(qualified)
        info = self.lib.classes[qualified]
        found = set()
        for member in info.members:
            found |= {atom for atom, _ in member.cons}
        for text, _, cons in info.bases:
            found |= {atom for atom, _ in cons}
            for token in tokenize(text):
                if token.kind == "id":
                    other = self.lookup(token.text, info.scope)
                    if other:
                        found |= self.atoms(other, seen)
        return found


def substitute(text, subst):
    if not subst:
        return text
    tokens = tokenize(text)
    out = []
    for token in tokens:
        if token.kind == "id" and token.text in subst:
            out.extend(tokenize(subst[token.text][0]))
        else:
            out.append(token)
    return join_type(out)


def vtable_records(library):
    layouts = Layouts(library)
    targets = []
    for qualified, info in sorted(library.classes.items()):
        if not info.params and not info.specialization:
            targets.append((qualified, [], qualified))
    for text, scope in sorted(library.instantiations):
        resolved = layouts.resolve(text, list(scope), {}, {}, 0)
        if resolved and library.classes[resolved[0]].params and resolved[1]:
            if not any(re.search(r"\b" + re.escape(p) + r"\b", a[0]) for a in resolved[1]
                       for p in enclosing_params(library, scope)):
                targets.append((resolved[0], resolved[1], text))
    records = []
    emitted = set()
    for qualified, args, _ in targets:
        atoms = sorted(layouts.atoms(qualified))
        if len(atoms) > MAX_VARIANT_ATOMS:
            atoms = atoms[:MAX_VARIANT_ATOMS]
        variants = []
        for values in itertools.product((False, True), repeat=len(atoms)):
            assignment = dict(zip(atoms, values))
            result = layouts.layout(qualified, args, assignment)
            if result is None or not result[0]:
                continue
            signature = tuple(s.key() for s in result[0])
            variants.append((signature, result[1], assignment))
        for index, (signature, display, conditions) in enumerate(group_variants(variants)):
            if (display, signature) in emitted:
                continue
            emitted.add((display, signature))
            records.append(("VTABLE", display, str(index), str(len(signature)), conditions))
            for slot_index, (name, sig, owner, pure, dtor) in enumerate(signature):
                flags = ("p" if pure else "") + ("d" if dtor else "")
                records.append(("SLOT", display, str(index), str(slot_index), owner + "::" + name, flags, sig))
    return records


def group_variants(variants):
    """Merge (key, extra, assignment) entries with equal keys: [(key, extra, conditions)], the
    conditions naming only the atoms with one value over all assignments of the key."""
    groups = []
    for key, extra, assignment in variants:
        group = next((g for g in groups if g[0] == key), None)
        if group is None:
            groups.append((key, extra, [assignment]))
        else:
            group[2].append(assignment)
    result = []
    for key, extra, assignments in groups:
        fixed = []
        for atom in sorted(assignments[0]):
            values = {a[atom] for a in assignments}
            if len(values) == 1:
                fixed.append(("" if values.pop() else "!") + atom)
        result.append((key, extra, ",".join(fixed)))
    return result


def enclosing_params(library, scope):
    params = set()
    for k in range(1, len(scope) + 1):
        info = library.classes.get("::".join(scope[:k]))
        if info:
            params.update(info.params)
    return params


# ---- Tables ----

class Scalar:
    def __init__(self, size, kind):
        self.size = size
        self.align = min(size, MAX_PACK)
        self.kind = kind  # ptr, int, float


class StructType:
    def __init__(self, fields, size, align):
        self.fields = fields  # [(offset, type)]
        self.size = size
        self.align = align


class ArrayType:
    def __init__(self, element, count):
        self.element = element
        self.count = count
        self.size = element.size * count if count is not None else None
        self.align = element.align


class Tables:
    def __init__(self, library):
        self.lib = library
        self.structs = {}
        for qualified, info in library.classes.items():
            self.structs.setdefault(qualified.split("::")[-1], info)
        self.function_names = {}
        for file_name, name in library.functions:
            self.function_names.setdefault(name.split("::")[-1], []).append((file_name, name))
        self.variable_names = {}
        for variable in library.variables:
            self.variable_names.setdefault(variable.name, []).append(variable)

    def type_of(self, base, declarator, assignment, depth=0):
        """Type of base tokens plus declarator tokens (* and [N]), or None if unsure."""
        if depth > MAX_LAYOUT_DEPTH:
            return None
        texts = [t.text for t in declarator]
        dims = []
        k = 0
        while k < len(declarator):
            if texts[k] == "[":
                close = matching(declarator, k, "[", "]")
                value = self.lib.macros.number(declarator[k + 1:close])
                if value is None:
                    return None
                dims.append(value)
                k = close + 1
            else:
                k += 1
        if "*" in texts:
            element = Scalar(POINTER_SIZE, "ptr")
        else:
            element = self.base_type(base, assignment, depth)
        if element is None:
            return None
        for count in reversed(dims):
            element = ArrayType(element, count)
        return element

    def base_type(self, base, assignment, depth):
        words = tuple(t.text for t in base if t.text not in ("const", "volatile", "register", "FAR", "far"))
        if not words:
            return None
        if words in BUILTIN_SIZES:
            size = BUILTIN_SIZES[words]
            return Scalar(size, "float" if words in FLOAT_TYPES else "int")
        if words[0] == "enum":
            return Scalar(POINTER_SIZE, "int")
        if words[0] in ("struct", "union") and len(words) == 2:
            return self.struct_type(words[1], assignment, depth)
        if len(words) == 1:
            name = words[0]
            if name in self.lib.typedefs:
                options = [d for d in self.lib.typedefs[name] if satisfied(d[2], assignment)]
                results = []
                for target, declarator, _ in options:
                    result = self.type_of(target, declarator, assignment, depth + 1)
                    results.append(None if result is None else layout_key(result))
                    chosen = result
                if not results or None in results or len(set(results)) != 1:
                    return None
                return chosen
            if name in self.structs:
                return self.struct_type(name, assignment, depth)
        return None

    def struct_type(self, name, assignment, depth):
        info = self.structs.get(name)
        if info is None or info.union or info.unsure or info.members:
            return None
        fields = []
        offset = 0
        align = 1
        for statement in info.fields:
            if not all(satisfied(t.cons, assignment) for t in statement[:1]):
                continue
            parsed = self.field_declarators(statement, assignment, depth)
            if parsed is None:
                return None
            for field_type in parsed:
                offset = (offset + field_type.align - 1) // field_type.align * field_type.align
                fields.append((offset, field_type))
                offset += field_type.size
                align = max(align, field_type.align)
        if not fields:
            return None
        size = (offset + align - 1) // align * align
        return StructType(fields, size, align)

    def field_declarators(self, statement, assignment, depth):
        statement = [t for t in statement if satisfied(t.cons, assignment)]
        if not statement:
            return []
        texts = [t.text for t in statement]
        if ":" in texts:
            return None  # bit fields
        if Parser.function_pointer_field(statement):
            return [Scalar(POINTER_SIZE, "ptr")]
        parts = split_top(statement)
        first = parts[0]
        ids = [k for k, t in enumerate(first) if t.kind == "id"]
        if len(ids) < 2 and not (ids and "*" in texts):
            return None
        name_index = ids[-1]
        base = [t for t in first[:name_index] if t.text != "*"]
        result = []
        for declarator in [first[name_index - 1:]] + parts[1:]:
            declarator = [t for t in declarator if t.kind != "id" or t is declarator[-1] and False]
            field_type = self.type_of(base, [t for t in declarator if t.text in "*[]" or t.kind == "num"
                                              or t.kind == "id" and False], assignment, depth + 1)
            if field_type is None:
                return None
            result.append(field_type)
        return result

    def flatten(self, variable, assignment):
        """[(offset, size, kind, value)] for a variable's initializer, or None if unsure."""
        variable_type = self.type_of(variable.type_tokens, [], assignment)
        if variable_type is None:
            return None
        tree = self.initializer_tree([t for t in variable.init[1:-1] if satisfied(t.cons, assignment)])
        if tree is None:
            return None
        leaves = []
        if variable.array:
            elements = tree
            index = 0
            offset = 0
            while index < len(elements):
                index = self.assign(variable_type, elements, index, offset, leaves)
                if index is None:
                    return None
                offset += variable_type.size
            total = offset
        else:
            if self.assign(variable_type, tree, 0, 0, leaves, whole=True) is None:
                return None
            total = variable_type.size
        return leaves, total

    def initializer_tree(self, tokens):
        elements = []
        for part in split_top(tokens):
            if not part:
                continue
            if part[0].text == "{" and part[-1].text == "}" and matching(part, 0, "{", "}") == len(part) - 1:
                inner = self.initializer_tree(part[1:-1])
                if inner is None:
                    return None
                elements.append(inner)
            else:
                elements.append(part)
        return elements

    def assign(self, field_type, elements, index, offset, leaves, whole=False):
        """Assign elements[index...] to field_type at offset; return the next index."""
        if whole:
            return self.assign_aggregate(field_type, elements, offset, leaves) and 1
        if isinstance(field_type, Scalar):
            if index >= len(elements):
                leaves.append((offset, field_type.size, field_type.kind, None, "zero"))
                return index
            element = elements[index]
            if isinstance(element, list) and element and isinstance(element[0], list):
                if len(element) != 1:
                    return None
                element = element[0]
            leaves.append((offset, field_type.size, field_type.kind, element, "expr"))
            return index + 1
        if index < len(elements) and is_group(elements[index]):
            if not self.assign_aggregate(field_type, elements[index], offset, leaves):
                return None
            return index + 1
        if isinstance(field_type, StructType):
            for field_offset, sub in field_type.fields:
                index = self.assign(sub, elements, index, offset + field_offset, leaves)
                if index is None:
                    return None
            return index
        for k in range(field_type.count or 0):
            index = self.assign(field_type.element, elements, index, offset + k * field_type.element.size, leaves)
            if index is None:
                return None
        return index

    def assign_aggregate(self, field_type, elements, offset, leaves):
        if isinstance(field_type, Scalar):
            return self.assign(field_type, elements, 0, offset, leaves) is not None
        index = 0
        if isinstance(field_type, StructType):
            for field_offset, sub in field_type.fields:
                index = self.assign(sub, elements, index, offset + field_offset, leaves)
                if index is None:
                    return False
        else:
            for k in range(field_type.count or 0):
                index = self.assign(field_type.element, elements, index, offset + k * field_type.element.size,
                                    leaves)
                if index is None:
                    return False
        return index == len(elements)

    def leaf(self, tokens, kind, variable):
        """(kind, value) of an initializer expression: str, func, ref, null, int; None = unknown;
        False = inconsistent with the field type."""
        tokens = strip_casts(tokens)
        if not tokens:
            return None
        macros = self.lib.macros
        value = macros.expand_strings(tokens, 0)
        if value is not None and value[1] == len(tokens) and (tokens[0].kind == "str" or tokens[0].text in macros.strings):
            return ("str", value[0]) if kind == "ptr" else False
        texts = [t.text for t in tokens]
        if texts in (["NULL"], ["0"], ["0L"], ["0U"], ["0UL"]):
            return ("null", 0) if kind == "ptr" else ("int", 0)
        if len(tokens) == 1 and tokens[0].kind == "id":
            name = tokens[0].text
            if name in self.function_names:
                return ("func", self.function_ref(name, variable)) if kind == "ptr" else False
            if name in self.variable_names and any(v.array for v in self.variable_names[name]):
                return ("ref", self.variable_ref(name, variable)) if kind == "ptr" else False
        if len(tokens) == 2 and texts[0] == "&" and tokens[1].kind == "id":
            name = tokens[1].text
            if name in self.variable_names:
                return ("ref", self.variable_ref(name, variable)) if kind == "ptr" else False
            if name in self.function_names:
                return ("func", self.function_ref(name, variable)) if kind == "ptr" else False
            return None
        if kind == "int":
            number = macros.number(tokens)
            return ("int", number) if number is not None else None
        if kind == "ptr":
            number = macros.number(tokens)
            if number == 0:
                return ("null", 0)
        return None

    def function_ref(self, name, variable):
        """Qualified name of the function a table entry names: the definition in the table's
        file, else the first one."""
        candidates = self.function_names[name]
        same = [qualified for file_name, qualified in candidates if file_name == variable.file]
        return same[0] if same else candidates[0][1]

    def variable_ref(self, name, variable):
        candidates = self.variable_names[name]
        same = [v for v in candidates if v.file == variable.file]
        target = same[0] if same else candidates[0]
        return target.file + ":" + target.name


def is_group(element):
    return isinstance(element, list) and (not element or isinstance(element[0], list))


def strip_casts(tokens):
    while len(tokens) > 2 and tokens[0].text == "(":
        close = matching(tokens, 0, "(", ")")
        inner = tokens[1:close]
        if close >= len(tokens) - 1 or not inner or not all(
                t.kind == "id" or t.text in ("*", "const", "::") for t in inner):
            break
        tokens = tokens[close + 1:]
    if len(tokens) > 2 and tokens[0].text == "(" and matching(tokens, 0, "(", ")") == len(tokens) - 1:
        return strip_casts(tokens[1:-1])
    return tokens


def layout_key(field_type):
    if isinstance(field_type, Scalar):
        return ("s", field_type.size, field_type.kind)
    if isinstance(field_type, ArrayType):
        return ("a", layout_key(field_type.element), field_type.count)
    return ("t", tuple((o, layout_key(t)) for o, t in field_type.fields), field_type.size)


def table_records(library):
    tables = Tables(library)
    records = []
    next_variant = {}
    for variable in library.variables:
        atoms = set()
        for token in variable.init:
            atoms |= {atom for atom, _ in token.cons}
        atoms |= struct_atoms(tables, variable.type_tokens, set())
        atoms = sorted(atoms)
        if len(atoms) > MAX_VARIANT_ATOMS:
            print("table %s:%s skipped: %d condition atoms (%s)" % (variable.file, variable.name, len(atoms),
                                                                  " ".join(atoms)), file=sys.stderr)
            continue
        variants = []
        for values in itertools.product((False, True), repeat=len(atoms)):
            assignment = dict(zip(atoms, values))
            flattened = tables.flatten(variable, assignment)
            if flattened is None:
                continue
            leaves, total = flattened
            fields = []
            consistent = True
            for offset, size, kind, element, origin in leaves:
                if origin == "zero":
                    fields.append((offset, size, "null" if kind == "ptr" else "int", 0))
                    continue
                if kind == "float":
                    continue
                value = tables.leaf(element, kind, variable)
                if value is False:
                    consistent = False
                    break
                if value is not None:
                    fields.append((offset, size, value[0], value[1]))
            if not consistent or not any(f[2] in ("func", "ref") for f in fields):
                continue
            variants.append(((total, tuple(fields)), None, assignment))
        for (total, fields), _, conditions in group_variants(variants):
            name = variable.file + ":" + variable.name
            index = next_variant.get(name, 0)
            next_variant[name] = index + 1
            records.append(("TABLE", name, str(index), str(total), join_type(variable.type_tokens)
                            + ("[]" if variable.array else ""), conditions))
            for offset, size, kind, value in fields:
                records.append(("FIELD", name, str(index), str(offset), str(size), kind, str(value)))
    return records


def struct_atoms(tables, type_tokens, seen):
    found = set()
    names = [t.text for t in type_tokens if t.kind == "id" and t.text not in TYPE_WORDS]
    for name in names:
        if name in seen:
            continue
        seen.add(name)
        # Typedef conditions are not variant atoms: base_type() requires all live options
        # to agree on the layout.
        for target, _, _ in tables.lib.typedefs.get(name, []):
            found |= struct_atoms(tables, target, seen)
        info = tables.structs.get(name)
        if info is not None:
            for statement in info.fields:
                for token in statement:
                    found |= {atom for atom, _ in token.cons}
                found |= struct_atoms(tables, statement, seen)
    return found


# ---- Main ----

def select_files(root, profile):
    selected = []
    for directory, subdirs, files in os.walk(root):
        subdirs.sort()
        for file_name in sorted(files):
            path = os.path.join(directory, file_name)
            relative = os.path.relpath(path, root).replace(os.sep, "/")
            if not any(fnmatch.fnmatch(relative, p) for p in profile["files"]):
                continue
            if any(fnmatch.fnmatch(relative, p) or fnmatch.fnmatch(file_name, p) for p in profile["exclude"]):
                continue
            selected.append((relative, path))
    return selected


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--lib", required=True)
    parser.add_argument("--version", default="")
    parser.add_argument("--out", required=True)
    parser.add_argument("sources", nargs="+")
    args = parser.parse_args()
    profile = dict(GENERIC_PROFILE)
    profile.update(PROFILES.get(args.lib, {}))
    profile.setdefault("replace", [])
    profile.setdefault("path_namespaces", {})
    pinned = dict(COMMON_PINNED)
    pinned.update(profile["pinned"])
    pinned = {k: v for k, v in pinned.items() if v is not None}
    library = Library(profile)
    files = []
    for root in args.sources:
        files.extend(select_files(root, profile))
    prepared = []
    for relative, path in files:
        with open(path, encoding="latin-1") as f:
            code, defines, conditions = preprocess(f.read(), profile["replace"])
        for directive in defines:
            library.macros.add(directive)
        prepared.append((relative, code, conditions))
    library.macros.resolve()
    for relative, code, conditions in prepared:
        tokens = resolve_conditionals(tokenize(code), conditions, pinned)
        prefix = ""
        for path_prefix, namespace in profile["path_namespaces"].items():
            if relative.startswith(path_prefix):
                prefix = namespace
        Parser(library, relative, tokens, prefix).run()
        library.sources.append(relative)

    records = []
    for (file_name, name), flags in sorted(library.functions.items()):
        records.append(("FUNC", file_name, name, flags))
    defined = {name for _, name in library.functions}
    short = {}
    for _, name in library.functions:
        short.setdefault(name.split("::")[-1], set()).add(name)
    if profile["calls"]:
        for (file_name, name), callees in sorted(library.calls.items()):
            for callee in sorted(callees):
                qualified = callee if callee in defined else None
                if qualified is None and len(short.get(callee, ())) == 1:
                    qualified = next(iter(short[callee]))
                if qualified and qualified != name:
                    records.append(("CALL", file_name, name, qualified))
    unique = 0
    for text, users in sorted(library.strings.items()):
        keys = {(u[0], u[1]) for u in users}
        if len(keys) == 1 and not any(u[2] for u in users):
            file_name, function = keys.pop()
            records.append(("STRING", file_name, function, text))
            unique += 1
    records.extend(table_records(library))
    records.extend(vtable_records(library))
    with open(args.out, "w", encoding="ascii") as out:
        out.write("# Generated by extract_source_names.py; do not edit.\n")
        out.write("# Library: %s %s\n" % (args.lib, args.version))
        out.write("# Files: %d (%s)\n" % (len(library.sources), " ".join(library.sources)))
        out.write("# FUNC file name flags(i = inline or header) | STRING file function text"
                  " | CALL file caller callee\n")
        out.write("# TABLE name variant size type conditions | FIELD name variant offset size"
                  " kind(str,func,ref,null,int) value\n")
        out.write("# VTABLE class variant slots conditions | SLOT class variant index owner::method"
                  " flags(p = pure, d = destructor) signature\n")
        for record in records:
            out.write("\t".join(escape_field(field) for field in record) + "\n")
    counts = {}
    for record in records:
        counts[record[0]] = counts.get(record[0], 0) + 1
    print(args.out, counts)


if __name__ == "__main__":
    main()
