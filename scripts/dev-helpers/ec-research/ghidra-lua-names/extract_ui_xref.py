#!/usr/bin/env python3
"""Index the Enhanced Client UI scripts (Lua) and window definitions (XML) into ui_xref.tsv
for CrossReferenceUi.java.

Usage: python3 -I extract_ui_xref.py UI_ROOT [OUTPUT_TSV]

UI_ROOT is the unpacked UserInterface/Default folder. Only identifiers, counts and file:line
positions are written, never script text. Calls whose callee (or the root table of a dotted
callee) the UI defines itself are left out. The Lua side is a small tokenizer, not a parser:
calls are `Name(...)`, `A.B(...)`, `Name "s"` and `Name {...}` (method calls `a:b()` are
skipped), enclosing functions come from function/if/do/repeat ... end/until nesting, and the
number of results a caller uses is read from the tokens before the call:
  N   assigned to N variables (`local a, b = F()`)
  0   bare statement
  1   used inside an expression (condition, operand, non-last argument)
  *   all results forwarded (`return F()`, last argument or last table item)
The XML side is a tag scanner (elements, attributes, parent/child pairs, event handler
values and top-level window names) that tolerates the files' loose XML. `L"text"` is the
client's wide-string literal, not a call of `L`.
"""
import os
import re
import sys

DEFAULT_OUTPUT = "ui_xref.tsv"
LUA_SUFFIX = ".lua"
XML_SUFFIXES = (".xml", ".mod")
ENCODING = "latin-1"

# Roots of the global tables the engine fills (read by the UI, written by C).
ENGINE_ROOTS = ("SystemData", "WindowData")
# Components kept for SystemData/WindowData chains (root.table.field.subfield).
MAX_PATH_DEPTH = 4
INDEX_MARK = "[]"
# Lua 5.1 standard library globals; calls through them are not UO API calls.
LUA_STDLIB = {
    "assert", "collectgarbage", "dofile", "error", "getfenv", "getmetatable", "ipairs", "load",
    "loadfile", "loadstring", "module", "next", "pairs", "pcall", "print", "rawequal", "rawget",
    "rawset", "require", "select", "setfenv", "setmetatable", "tonumber", "tostring", "type",
    "unpack", "xpcall", "coroutine", "debug", "io", "math", "os", "package", "string", "table",
}
KEYWORDS = {
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
}
# Block structure: `while`/`for` always come with `do`, `elseif`/`else` share the `if` end.
BLOCK_OPENERS = {"function", "if", "do", "repeat"}
BLOCK_CLOSERS = {"end", "until"}
# Tokens after which a call starts a new statement (its results are discarded).
STATEMENT_KEYWORDS = {"then", "do", "else", "end", "repeat", "break"}
STATEMENT_ENDERS = {")", "]", "}", ";"}
LIST_OPENERS = {"(", "{"}
LIST_CLOSERS = {")", "}"}
PAIRS = {"(": ")", "[": "]", "{": "}"}
CLOSING = {v: k for k, v in PAIRS.items()}
# Handler registrations: callee -> (event argument, function argument), 1-based.
HANDLER_REGISTRARS = {"WindowRegisterEventHandler": (2, 3), "WindowRegisterCoreEventHandler": (2, 3)}
CHUNK = "<chunk>"
ANONYMOUS = "<anonymous>"
RESULT_ALL = "*"
VARARG = "..."
VARIADIC_MARK = "+"

LUA_TOKEN = re.compile(r'''
    (?P<comment>--(?:\[(?P<cl>=*)\[.*?\](?P=cl)\]|[^\n]*))
  | (?P<long>\[(?P<sl>=*)\[.*?\](?P=sl)\])
  | (?P<str>L?"(?:[^"\\\n]|\\.)*"|L?'(?:[^'\\\n]|\\.)*')
  | (?P<name>[A-Za-z_]\w*)
  | (?P<num>0[xX][0-9a-fA-F]+|\d+\.?\d*(?:[eE][-+]?\d+)?|\.\d+(?:[eE][-+]?\d+)?)
  | (?P<op>\.\.\.|\.\.|==|~=|<=|>=|[-+*/%^\#<>=(){}\[\];:,.])
  | (?P<nl>\n)
  | (?P<ws>[ \t\r\f\v]+)
  | (?P<other>.)
''', re.VERBOSE | re.DOTALL)

XML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)
XML_SPECIAL = re.compile(r"<[?!].*?>", re.DOTALL)
XML_TAG = re.compile(r"<(/?)([A-Za-z_][\w.:-]*)((?:[^>\"']|\"[^\"]*\"|'[^']*')*?)(/?)>", re.DOTALL)
XML_ATTRIBUTE = re.compile(r"([A-Za-z_][\w.:-]*)\s*=\s*(?:\"([^\"]*)\"|'([^']*)')")
# Attribute values indexed (element, attribute): the hooks C dispatches to.
VALUE_ATTRIBUTES = {("EventHandler", "event"), ("EventHandler", "function")}
WINDOW_LIST = "Windows"
TOP_LEVEL_PARENTS = ("Interface",)
PARENT_PLACEHOLDER = "$"


class Token:
    __slots__ = ("kind", "text", "line")

    def __init__(self, kind, text, line):
        self.kind = kind
        self.text = text
        self.line = line


def lua_tokens(code):
    tokens = []
    line = 1
    for match in LUA_TOKEN.finditer(code):
        kind = match.lastgroup
        text = match.group()
        if kind in ("comment", "long", "str", "name", "num", "op", "other"):
            if kind == "long":
                kind = "str"
            if kind == "name" and text in KEYWORDS:
                kind = "kw"
            if kind != "comment":
                tokens.append(Token(kind, text, line))
        line += text.count("\n")
    return tokens


def matching_forward(tokens, i):
    """Index of the bracket closing tokens[i]."""
    depth = 0
    for j in range(i, len(tokens)):
        if tokens[j].kind == "op" and tokens[j].text in PAIRS:
            depth += 1
        elif tokens[j].kind == "op" and tokens[j].text in CLOSING:
            depth -= 1
            if depth == 0:
                return j
    return len(tokens) - 1


def matching_backward(tokens, j):
    """Index of the bracket opening tokens[j]."""
    depth = 0
    for i in range(j, -1, -1):
        if tokens[i].kind == "op" and tokens[i].text in CLOSING:
            depth += 1
        elif tokens[i].kind == "op" and tokens[i].text in PAIRS:
            depth -= 1
            if depth == 0:
                return i
    return 0


def is_op(token, text):
    return token is not None and token.kind == "op" and token.text == text


def chain_at(tokens, i):
    """A dotted name chain starting at tokens[i]: (names, index after the chain)."""
    names = [tokens[i].text]
    j = i + 1
    while j + 1 < len(tokens) and is_op(tokens[j], ".") and tokens[j + 1].kind == "name":
        names.append(tokens[j + 1].text)
        j += 2
    return names, j


def split_arguments(tokens, open_index, close_index):
    """Top-level argument token ranges of the list between the brackets."""
    args = []
    start = open_index + 1
    depth = 0
    for j in range(open_index + 1, close_index):
        token = tokens[j]
        if token.kind == "op" and token.text in PAIRS:
            depth += 1
        elif token.kind == "op" and token.text in CLOSING:
            depth -= 1
        elif depth == 0 and is_op(token, ","):
            args.append((start, j))
            start = j + 1
    if start < close_index:
        args.append((start, close_index))
    return args


def ends_with_call(tokens, start, end):
    """Whether the expression tokens[start:end] ends with a call (can expand to many values)."""
    last = tokens[end - 1]
    if last.kind == "op" and last.text == VARARG:
        return end - start == 1
    if last.kind == "str" and end - start > 1 and tokens[end - 2].kind == "name":
        return True
    if not (last.kind == "op" and last.text in (")", "}")):
        return False
    opener = matching_backward(tokens, end - 1)
    if opener <= start:
        return False
    before = tokens[opener - 1]
    return before.kind in ("name", "str") or (before.kind == "op" and before.text in (")", "]"))


def skip_var_backward(tokens, j):
    """Index before the variable ending at tokens[j] (`a`, `a.b`, `a[x]`, `f().b`), or None."""
    while j >= 0:
        if is_op(tokens[j], "]"):
            # An index: a prefix (name or another index) must precede it.
            j = matching_backward(tokens, j) - 1
            continue
        if tokens[j].kind != "name":
            return None
        j -= 1
        if j >= 0 and is_op(tokens[j], "."):
            j -= 1
            continue
        return j
    return None


def results_used(tokens, start, close_index):
    """How many results of the call spanning tokens[start..close_index] the caller uses."""
    before = tokens[start - 1] if start > 0 else None
    after = tokens[close_index + 1] if close_index + 1 < len(tokens) else None
    if before is None:
        return 0
    if before.kind == "kw" and before.text == "return":
        return RESULT_ALL
    if is_op(before, "="):
        count = 0
        j = start - 2
        while True:
            k = skip_var_backward(tokens, j)
            if k is None:
                return 1 if count == 0 else count
            count += 1
            if k >= 0 and is_op(tokens[k], ","):
                j = k - 1
                continue
            if k >= 0 and (is_op(tokens[k], "{") or (is_op(tokens[k], ",") and count == 1)):
                break
            return count
        # A table constructor field `{ key = F() }`.
        return RESULT_ALL if after is not None and is_op(after, "}") else 1
    if before.kind == "op" and before.text in LIST_OPENERS | {","}:
        if after is not None and after.kind == "op" and after.text in LIST_CLOSERS:
            return RESULT_ALL
        return 1
    if before.kind == "kw" and before.text in STATEMENT_KEYWORDS:
        return 0
    if before.kind in ("name", "num", "str") or (before.kind == "op" and before.text in STATEMENT_ENDERS):
        return 0
    return 1


def literal_text(tokens, start, end):
    """An argument as text: a string's contents, otherwise its tokens joined."""
    if end - start == 1 and tokens[start].kind == "str":
        return tokens[start].text.lstrip("L")[1:-1]
    return "".join(token.text for token in tokens[start:end])


def function_name_after(tokens, i):
    """Name of `function a.b:c(` at tokens[i] (the `function` keyword), or None."""
    if i + 1 < len(tokens) and tokens[i + 1].kind == "name":
        names = [tokens[i + 1].text]
        j = i + 2
        while j + 1 < len(tokens) and tokens[j].kind == "op" and tokens[j].text in (".", ":") \
                and tokens[j + 1].kind == "name":
            names.append(tokens[j + 1].text)
            j += 2
        return ".".join(names)
    return None


def assigned_name_before(tokens, i):
    """Name assigned an anonymous function: `a.b = function` or `{ key = function`."""
    if i >= 2 and is_op(tokens[i - 1], "=") and tokens[i - 2].kind == "name":
        names = [tokens[i - 2].text]
        j = i - 3
        while j >= 1 and is_op(tokens[j], ".") and tokens[j - 1].kind == "name":
            names.insert(0, tokens[j - 1].text)
            j -= 2
        return ".".join(names)
    return None


def scan_lua(path, rel, records, definitions, paths):
    with open(path, encoding=ENCODING) as handle:
        tokens = lua_tokens(handle.read())
    stack = []
    locals_top = set()
    for i, token in enumerate(tokens):
        if token.kind == "kw":
            if token.text in BLOCK_OPENERS:
                name = None
                if token.text == "function":
                    name = function_name_after(tokens, i)
                    is_local = i > 0 and tokens[i - 1].kind == "kw" and tokens[i - 1].text == "local"
                    if name is None:
                        name = assigned_name_before(tokens, i)
                        if name is not None and not stack and "." not in name:
                            if name not in locals_top:
                                definitions.setdefault(name, ("function", rel, token.line))
                    elif not is_local:
                        definitions.setdefault(name, ("function", rel, token.line))
                    elif not stack:
                        locals_top.add(name)
                    name = name or ANONYMOUS
                stack.append((token.text, name))
            elif token.text in BLOCK_CLOSERS and stack:
                stack.pop()
            elif token.text == "local" and not stack and i + 1 < len(tokens) and tokens[i + 1].kind == "name":
                locals_top.add(tokens[i + 1].text)
            continue
        if token.kind != "name":
            continue
        previous = tokens[i - 1] if i > 0 else None
        if previous is not None and previous.kind == "op" and previous.text in (".", ":"):
            continue
        names, after = chain_at(tokens, i)
        if names[0] in ENGINE_ROOTS:
            record_path(tokens, i, rel, paths)
        # Global assignment at file level: `Name = ...` / `Name.Field = ...`.
        if not stack and len(names) == 1 and after < len(tokens) and is_op(tokens[after], "=") \
                and names[0] not in locals_top and (previous is None or previous.kind != "kw"
                                                    or previous.text != "local"):
            definitions.setdefault(names[0], ("value", rel, token.line))
        if after >= len(tokens):
            continue
        nxt = tokens[after]
        if is_op(nxt, ":") or (previous is not None and previous.kind == "kw" and previous.text == "function"):
            continue
        if is_op(nxt, "("):
            close = matching_forward(tokens, after)
            args = split_arguments(tokens, after, close)
            variadic = bool(args) and ends_with_call(tokens, *args[-1])
            arg_text = str(len(args)) + (VARIADIC_MARK if variadic else "")
        elif nxt.kind == "str" or is_op(nxt, "{"):
            close = matching_forward(tokens, after) if is_op(nxt, "{") else after
            arg_text = "1"
        else:
            continue
        if names[0] in LUA_STDLIB or names[0] in KEYWORDS:
            continue
        registrar = HANDLER_REGISTRARS.get(".".join(names))
        if registrar is not None and is_op(nxt, "(") and len(args) >= max(registrar):
            event, function = (literal_text(tokens, *args[position - 1]) for position in registrar)
            records.append(("HANDLER", ".".join(names), event, function, rel, str(token.line)))
        enclosing = CHUNK
        for kind, name in reversed(stack):
            if kind == "function" and name != ANONYMOUS:
                enclosing = name
                break
            if kind == "function" and enclosing == CHUNK:
                enclosing = ANONYMOUS
        records.append(("CALL", ".".join(names), rel, str(token.line), arg_text,
                        str(results_used(tokens, i, close)), enclosing))


def record_path(tokens, i, rel, paths):
    """A SystemData/WindowData chain (with [] for indexing), and whether the UI writes it."""
    names = [tokens[i].text]
    j = i + 1
    while len(names) < MAX_PATH_DEPTH and j < len(tokens):
        if is_op(tokens[j], ".") and j + 1 < len(tokens) and tokens[j + 1].kind == "name":
            names.append(tokens[j + 1].text)
            j += 2
        elif is_op(tokens[j], "["):
            names.append(INDEX_MARK)
            j = matching_forward(tokens, j) + 1
        else:
            break
    while names and names[-1] == INDEX_MARK:
        names.pop()
    write = j < len(tokens) and is_op(tokens[j], "=")
    for depth in range(2, len(names) + 1):
        key = ".".join(names[:depth]).replace("." + INDEX_MARK, INDEX_MARK)
        entry = paths.setdefault(key, [0, 0, rel, tokens[i].line])
        entry[1 if write and depth == len(names) else 0] += 1


def scan_xml(path, rel, elements, children, attributes, values, windows):
    with open(path, encoding=ENCODING) as handle:
        text = handle.read()
    blank = lambda match: re.sub(r"[^\n]", " ", match.group())
    text = XML_SPECIAL.sub(blank, XML_COMMENT.sub(blank, text))
    stack = []
    for match in XML_TAG.finditer(text):
        closing, name, body, empty = match.groups()
        if closing:
            while stack and stack.pop() != name:
                pass
            continue
        line = text.count("\n", 0, match.start()) + 1
        parent = stack[-1] if stack else ""
        entry = elements.setdefault(name, [0, rel, line])
        entry[0] += 1
        children[(parent, name)] = children.get((parent, name), 0) + 1
        for attribute in XML_ATTRIBUTE.finditer(body):
            key = attribute.group(1)
            value = attribute.group(2) if attribute.group(2) is not None else attribute.group(3)
            attributes[(name, key)] = attributes.get((name, key), 0) + 1
            if (name, key) in VALUE_ATTRIBUTES:
                values[(name, key, value)] = values.get((name, key, value), 0) + 1
            if key == "name" and parent == WINDOW_LIST and len(stack) >= 2 \
                    and stack[-2] in TOP_LEVEL_PARENTS and PARENT_PLACEHOLDER not in value:
                windows.setdefault(value, (name, rel, line))
        if not empty:
            stack.append(name)


def escape(value):
    return value.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n")


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    root = sys.argv[1]
    output = sys.argv[2] if len(sys.argv) == 3 else os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                                 DEFAULT_OUTPUT)
    calls, definitions, paths = [], {}, {}
    elements, children, attributes, values, windows = {}, {}, {}, {}, {}
    lua_files = xml_files = 0
    for directory, subdirs, files in os.walk(root):
        subdirs.sort()
        for name in sorted(files):
            path = os.path.join(directory, name)
            rel = os.path.relpath(path, root).replace(os.sep, "/")
            lower = name.lower()
            if lower.endswith(LUA_SUFFIX):
                lua_files += 1
                scan_lua(path, rel, calls, definitions, paths)
            elif lower.endswith(XML_SUFFIXES):
                xml_files += 1
                scan_xml(path, rel, elements, children, attributes, values, windows)
    rows = []
    # Calls into functions and tables the UI defines itself are UI-internal; keep the rest
    # (engine API calls, plus calls through locals and parameters).
    rows.extend(row for row in calls if row[0] != "CALL" or row[1].split(".")[0] not in definitions)
    for name, (kind, rel, line) in sorted(definitions.items()):
        rows.append(("LUADEF", name, kind, rel, str(line)))
    for key, (reads, writes, rel, line) in sorted(paths.items()):
        rows.append(("PATH", key, str(reads), str(writes), rel, str(line)))
    for name, (count, rel, line) in sorted(elements.items()):
        rows.append(("XMLEL", name, str(count), rel, str(line)))
    for (parent, child), count in sorted(children.items()):
        rows.append(("XMLCHILD", parent, child, str(count)))
    for (element, attribute), count in sorted(attributes.items()):
        rows.append(("XMLATTR", element, attribute, str(count)))
    for (element, attribute, value), count in sorted(values.items()):
        rows.append(("XMLVALUE", element, attribute, value, str(count)))
    for name, (element, rel, line) in sorted(windows.items()):
        rows.append(("XMLWINDOW", name, element, rel, str(line)))
    with open(output, "w", encoding="utf-8", newline="\n") as out:
        out.write("# Generated by extract_ui_xref.py from the Enhanced Client UserInterface/Default "
                  "(%d Lua, %d XML files); identifiers and positions only; do not edit.\n" % (lua_files, xml_files))
        out.write("# CALL\tname\tfile\tline\targs(+ = last argument may expand)\tresults(N, *)\tenclosing\n")
        out.write("# HANDLER\tregistrar\tevent\tfunction\tfile\tline\n")
        out.write("# LUADEF\tname\tfunction|value\tfile\tline\n")
        out.write("# PATH\tchain\treads\twrites\tfirst_file\tfirst_line\n")
        out.write("# XMLEL\telement\tcount\tfirst_file\tfirst_line\n")
        out.write("# XMLCHILD\tparent\tchild\tcount\n")
        out.write("# XMLATTR\telement\tattribute\tcount\n")
        out.write("# XMLVALUE\telement\tattribute\tvalue\tcount\n")
        out.write("# XMLWINDOW\tname\telement\tfile\tline\n")
        for row in rows:
            out.write("\t".join(escape(field) for field in row) + "\n")
    kept = sum(1 for row in rows if row[0] == "CALL")
    handlers = sum(1 for row in rows if row[0] == "HANDLER")
    print("Wrote %s: %d engine calls, %d handler registrations, %d definitions, %d paths, %d elements, %d attributes from %d Lua and %d XML files"
          % (output, kept, handlers, len(definitions), len(paths), len(elements), len(attributes), lua_files, xml_files))


if __name__ == "__main__":
    main()
