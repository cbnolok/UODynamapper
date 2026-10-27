#!/usr/bin/env python3
"""Extract luaL_Reg tables, luaL_register calls, luaL_openlibs entries and per-function
string literals from the LuaPlus Lua 5.1 sources into lua51_sources.tsv for
NameLuaFunctions.java.

Usage: python3 -I extract_lua_sources.py LUAPLUS_ROOT [OUTPUT_TSV]

LUAPLUS_ROOT is the LuaPlus checkout (the folder holding Src/LuaPlus). The parser is a
small tokenizer, not a C preprocessor: preprocessor lines are dropped (so both branches of
#if blocks are kept), simple string macros (#define NAME "text") and LUA_QL(x) are expanded,
and luaplus_assert(e) is stringified the way MSVC does (#e, whitespace runs collapsed).
"""
import os
import re
import sys

# Variants of the Lua 5.1 core parsed; the first is the LuaPlus-modified one in UOSA.
C_VARIANTS = ("lua51-luaplus", "lua51")
VARIANT_DIR = os.path.join("Src", "LuaPlus")
C_SUBDIR = "src"
# Standalone programs, not part of the library linked into UOSA.
SKIPPED_FILES = {"lua.c", "luac.c", "print.c", "lua.cpp"}
# LuaPlus C++ sources (namespace LuaPlus) parsed for literals and assert texts.
CPP_VARIANT = "luaplus"
CPP_SUFFIXES = (".cpp", ".inl", ".c")
HEADER_SUFFIX = ".h"
REG_TYPES = ("luaL_Reg", "luaL_reg")
REGISTER_CALLS = ("luaL_register", "luaL_openlib", "luaI_openlib")
OPENLIBS_TABLE = "lualibs"
OPENLIBS_FILE = "linit.c"
QUOTE_MACRO = "LUA_QL"
ASSERT_MACROS = ("luaplus_assert",)
TRANSPARENT_BLOCKS = ("namespace", "extern")
DEFAULT_OUTPUT = "lua51_sources.tsv"

IF_MARK = "@if"
ELSE_MARK = "@else"
ENDIF_MARK = "@endif"
CONDITIONALS = {"if": IF_MARK, "ifdef": IF_MARK, "ifndef": IF_MARK, "elif": ELSE_MARK,
                "else": ELSE_MARK, "endif": ENDIF_MARK}
TOKEN = re.compile(r'''
    (?P<mark>@(?:if|else|endif)\b)
  | (?P<str>"(?:[^"\\\n]|\\.)*")
  | (?P<chr>'(?:[^'\\\n]|\\.)*')
  | (?P<id>[A-Za-z_]\w*)
  | (?P<num>\.?\d[\w.]*)
  | (?P<scope>::)
  | (?P<punct>[^\s\w])
''', re.VERBOSE)
DEFINE = re.compile(r'#\s*define\s+(\w+)(\((\w+)\))?\s+(.*)')
ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b", "f": "\f", "v": "\v",
           "\\": "\\", "'": "'", '"': '"', "?": "?", "0": "\0"}
OCTAL_DIGITS = 3
OCTAL_BASE = 8
HEX_BASE = 16
PRINTABLE_FIRST = 0x20
PRINTABLE_LAST = 0x7e


def strip_comments(text):
    """Replace comments by a space, keeping string and char literals intact."""
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


def split_preprocessor(text):
    """Return (code with conditional directives as markers, list of other directives)."""
    code = []
    defines = []
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.lstrip().startswith("#"):
            directive = line
            while directive.endswith("\\") and i + 1 < len(lines):
                i += 1
                directive = directive[:-1] + " " + lines[i]
                code.append("")
            word = re.match(r"#\s*(\w*)", directive.strip()).group(1)
            code.append(CONDITIONALS.get(word, ""))
            if word not in CONDITIONALS:
                defines.append(directive.strip())
        else:
            code.append(line)
        i += 1
    return "\n".join(code), defines


def resolve_conditionals(tokens):
    """Keep every branch of #if groups; if the branches change the brace depth differently,
    keep only the first branch's braces so function bodies stay balanced."""
    def group(i):
        branches = [[]]
        i += 1
        while i < len(tokens):
            text = tokens[i][1]
            if text == IF_MARK:
                sub, i = group(i)
                branches[-1].extend(sub)
                continue
            i += 1
            if text == ELSE_MARK:
                branches.append([])
            elif text == ENDIF_MARK:
                break
            else:
                branches[-1].append(tokens[i - 1])
        deltas = {sum((t[1] == "{") - (t[1] == "}") for t in branch) for branch in branches}
        kept = list(branches[0])
        for branch in branches[1:]:
            kept.extend(t for t in branch if len(deltas) == 1 or t[1] not in "{}")
        return kept, i

    out = []
    i = 0
    while i < len(tokens):
        if tokens[i][1] == IF_MARK:
            kept, i = group(i)
            out.extend(kept)
        elif tokens[i][1] in (ELSE_MARK, ENDIF_MARK):
            i += 1
        else:
            out.append(tokens[i])
            i += 1
    return out


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
        e = body[i + 1]
        if e == "x":
            j = i + 2
            while j < len(body) and body[j] in "0123456789abcdefABCDEF":
                j += 1
            out.append(chr(int(body[i + 2:j], HEX_BASE)))
            i = j
        elif e.isdigit():
            j = i + 1
            while j < len(body) and j < i + 1 + OCTAL_DIGITS and body[j] in "01234567":
                j += 1
            out.append(chr(int(body[i + 1:j], OCTAL_BASE)))
            i = j
        else:
            out.append(ESCAPES.get(e, e))
            i += 2
    return "".join(out)


def escape_field(value):
    out = []
    for c in value:
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
            out.append("\\x%02x" % ord(c))
    return "".join(out)


def tokenize(code):
    return [(m.lastgroup, m.group(), m.start()) for m in TOKEN.finditer(code)]


class Macros:
    """String macros (NAME -> text) and one-parameter quoting macros such as LUA_QL."""

    def __init__(self):
        self.strings = {}
        self.raw = {}

    def add(self, directive):
        m = DEFINE.match(directive)
        if not m or m.group(2):
            return
        self.raw[m.group(1)] = m.group(4).strip()

    def resolve(self):
        changed = True
        while changed:
            changed = False
            for name, body in self.raw.items():
                if name in self.strings:
                    continue
                value = self.expand_tokens(tokenize(body), 0)
                if value is not None and value[1] == len(tokenize(body)):
                    self.strings[name] = value[0]
                    changed = True

    def expand_tokens(self, tokens, i):
        """Concatenate adjacent literals/macros from tokens[i]; return (text, next index)."""
        parts = []
        while i < len(tokens):
            kind, text, _ = tokens[i]
            if kind == "str":
                parts.append(unescape(text))
                i += 1
            elif kind == "id" and text in self.strings:
                parts.append(self.strings[text])
                i += 1
            elif (kind == "id" and text == QUOTE_MACRO and i + 3 < len(tokens)
                  and tokens[i + 1][1] == "(" and tokens[i + 2][0] == "str" and tokens[i + 3][1] == ")"):
                parts.append("'" + unescape(tokens[i + 2][1]) + "'")
                i += 4
            else:
                break
        return ("".join(parts), i) if parts else None


def matching(tokens, i, open_char, close_char):
    depth = 0
    for j in range(i, len(tokens)):
        if tokens[j][1] == open_char:
            depth += 1
        elif tokens[j][1] == close_char:
            depth -= 1
            if depth == 0:
                return j
    return len(tokens) - 1


def function_name(header):
    """Name of a function definition from the tokens before its body, or None."""
    texts = [t[1] for t in header]
    if "=" in texts or "(" not in texts:
        return None
    first = texts.index("(")
    # LUA_API int (lua_gettop) (lua_State *L)
    if first + 2 < len(texts) and header[first + 1][0] == "id" and texts[first + 2] == ")" \
            and first + 3 < len(texts) and texts[first + 3] == "(":
        return texts[first + 1]
    j = first - 1
    if j < 0 or header[j][0] != "id":
        return None
    parts = [texts[j]]
    while j >= 2 and texts[j - 1] == "::" and header[j - 2][0] == "id":
        j -= 2
        parts.insert(0, texts[j])
    if parts[-1] in ("if", "while", "for", "switch", "return", "sizeof"):
        return None
    return "::".join(parts)


def stringify(code, start, end):
    """MSVC #e: the argument text with whitespace runs collapsed to one space."""
    return re.sub(r"\s+", " ", code[start:end]).strip()


def parse_file(path, macros, records, strings, variant, namespace_prefix):
    with open(path, encoding="latin-1") as f:
        code, _ = split_preprocessor(strip_comments(f.read()))
    tokens = resolve_conditionals(tokenize(code))
    name = os.path.basename(path)
    i = 0
    statement_start = 0
    while i < len(tokens):
        kind, text, _ = tokens[i]
        if text == ";" or text == "}":
            statement_start = i + 1
            i += 1
            continue
        if text != "{":
            i += 1
            continue
        header = tokens[statement_start:i]
        head_texts = [t[1] for t in header]
        if head_texts and head_texts[0] in TRANSPARENT_BLOCKS:
            statement_start = i + 1
            i += 1
            continue
        end = matching(tokens, i, "{", "}")
        if "=" in head_texts and any(t in head_texts for t in REG_TYPES):
            table = head_texts[head_texts.index("[") - 1] if "[" in head_texts else None
            if table:
                parse_reg(tokens[i:end + 1], macros, records, variant, name, table)
        else:
            function = function_name(header)
            if function:
                qualified = namespace_prefix + function if "::" in function else function
                records.append(("FUNC", variant, name, qualified, "", ""))
                parse_body(code, tokens[i:end + 1], macros, records, strings, variant, name, qualified)
        statement_start = end + 1
        i = end + 1


def parse_reg(tokens, macros, records, variant, file_name, table):
    i = 1
    while i < len(tokens):
        if tokens[i][1] == "{":
            value = macros.expand_tokens(tokens, i + 1)
            if value is not None:
                entry, j = value
                if j + 1 < len(tokens) and tokens[j][1] == "," and tokens[j + 1][0] == "id":
                    records.append(("REG", variant, file_name, table, entry, tokens[j + 1][1]))
        i += 1


def parse_body(code, tokens, macros, records, strings, variant, file_name, function):
    i = 0
    while i < len(tokens):
        kind, text, position = tokens[i]
        if kind == "id" and text in REGISTER_CALLS and i + 1 < len(tokens) and tokens[i + 1][1] == "(":
            close = matching(tokens, i + 1, "(", ")")
            args = split_args(tokens[i + 2:close])
            if len(args) >= 3 and len(args[2]) == 1 and args[2][0][0] == "id":
                libname = macros.expand_tokens(args[1], 0)
                records.append(("REGISTER", variant, file_name, function, args[2][0][1],
                                libname[0] if libname else "", text))
        if kind == "id" and text in ASSERT_MACROS and i + 1 < len(tokens) and tokens[i + 1][1] == "(":
            close = matching(tokens, i + 1, "(", ")")
            strings.setdefault(stringify(code, tokens[i + 1][2] + 1, tokens[close][2]), set()).add(
                (variant, file_name, function))
        value = macros.expand_tokens(tokens, i) if kind in ("str", "id") else None
        if value is not None and (kind == "str" or value[1] > i + 1 or text in macros.strings):
            strings.setdefault(value[0], set()).add((variant, file_name, function))
            i = value[1]
            continue
        i += 1


def split_args(tokens):
    args = [[]]
    depth = 0
    for token in tokens:
        if token[1] in "([{":
            depth += 1
        elif token[1] in ")]}":
            depth -= 1
        if token[1] == "," and depth == 0:
            args.append([])
        else:
            args[-1].append(token)
    return args


def parse_openlibs(path, macros, records, variant):
    with open(path, encoding="latin-1") as f:
        code, _ = split_preprocessor(strip_comments(f.read()))
    tokens = resolve_conditionals(tokenize(code))
    for i, token in enumerate(tokens):
        if token[1] == OPENLIBS_TABLE and i + 3 < len(tokens) and tokens[i + 1][1] == "[":
            start = next(j for j in range(i, len(tokens)) if tokens[j][1] == "{")
            end = matching(tokens, start, "{", "}")
            j = start + 1
            while j < end:
                if tokens[j][1] == "{":
                    value = macros.expand_tokens(tokens, j + 1)
                    if value is not None and tokens[value[1] + 1][0] == "id":
                        records.append(("OPEN", variant, OPENLIBS_FILE, OPENLIBS_TABLE, value[0],
                                        tokens[value[1] + 1][1]))
                j += 1
            return


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    root = os.path.join(sys.argv[1], VARIANT_DIR)
    output = sys.argv[2] if len(sys.argv) == 3 else os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                                 DEFAULT_OUTPUT)
    records = []
    strings = {}
    sources = []
    for variant in C_VARIANTS:
        directory = os.path.join(root, variant, C_SUBDIR)
        macros = Macros()
        names = sorted(os.listdir(directory))
        for file_name in names:
            if file_name.endswith(HEADER_SUFFIX):
                with open(os.path.join(directory, file_name), encoding="latin-1") as f:
                    for directive in split_preprocessor(strip_comments(f.read()))[1]:
                        macros.add(directive)
        macros.resolve()
        for file_name in names:
            if file_name.endswith(".c") and file_name not in SKIPPED_FILES:
                parse_file(os.path.join(directory, file_name), macros, records, strings, variant, "")
                sources.append(variant + "/" + file_name)
        parse_openlibs(os.path.join(directory, OPENLIBS_FILE), macros, records, variant)
    macros = Macros()
    for file_name in sorted(os.listdir(root)):
        if file_name.endswith(HEADER_SUFFIX):
            with open(os.path.join(root, file_name), encoding="latin-1") as f:
                for directive in split_preprocessor(strip_comments(f.read()))[1]:
                    macros.add(directive)
    macros.resolve()
    for file_name in sorted(os.listdir(root)):
        if file_name.endswith(CPP_SUFFIXES):
            parse_file(os.path.join(root, file_name), macros, records, strings, CPP_VARIANT, "LuaPlus::")
            sources.append(CPP_VARIANT + "/" + file_name)
    tables = {(r[1], r[2], r[3]) for r in records if r[0] == "REG"}
    records = [r for r in records if r[0] != "REGISTER" or (r[1], r[2], r[4]) in tables]
    # A literal is evidence only if exactly one source function name uses it.
    for text, users in sorted(strings.items()):
        functions = {user[2] for user in users}
        if len(functions) == 1 and text:
            variants = sorted({user[0] for user in users})
            files = sorted({user[1] for user in users})
            records.append(("STRING", ",".join(variants), ",".join(files), functions.pop(), "", text))
    unique = sorted(set(records))
    with open(output, "w", encoding="ascii") as out:
        out.write("# Generated by extract_lua_sources.py from LuaPlus (MIT); do not edit.\n")
        out.write("# Sources: " + " ".join(sources) + "\n")
        out.write("# kind\tvariant\tfile\tfunction_or_table\tkey\tvalue[\tcallee]\n")
        for record in unique:
            out.write("\t".join(escape_field(field) for field in record) + "\n")
    counts = {}
    for record in unique:
        counts[record[0]] = counts.get(record[0], 0) + 1
    print(output, counts)


if __name__ == "__main__":
    main()
