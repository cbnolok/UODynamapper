#!/usr/bin/env python3
"""Compile Chaos particle scripts (.ems) the way UOSA.exe does and report what the data uses.

Reference implementation of results/uosa_chaos.md: the lexer (ChaosScript::Tokenize), the classifier
(ChaosScript::ClassifyToken) and the compiler (Emitter::CompileScript and the sequence block), including
their quirks. Read-only; standard library only.

    python3 -I verify_chaos.py DIR_OR_FILE... [--dump]

Every regular file under the given paths is tried; files that do not start with PARTICLESYSTEM are
skipped. Nothing from the data is written anywhere: the report goes to stdout (aggregate counts, plus
per-file results with --dump). Exit code 1 if any script fails to compile.
"""

import argparse
import collections
import os
import re
import sys

# ---- token types (ClassifyToken) ----
T_NUMBER, T_EQUALS, T_COMMA, T_OPEN, T_CLOSE, T_LBRACE, T_RBRACE, T_QUOTED = 1, 2, 3, 4, 5, 6, 7, 8
T_PARTICLESYSTEM, T_EVENTSEQUENCE, T_RANDOM, T_XYZ, T_CELSETUP, T_RGBA = 9, 0xA, 0xB, 0xC, 0xD, 0xE
T_TEXTURE, T_INHERITED, T_TYPE, T_EMITTERNAME, T_MESHNAME, T_CELANIMATE = 0xF, 0x10, 0x11, 0x13, 0x14, 0x15
T_BURST, T_EMISSIONBOOST, T_COLLISIONCHECK, T_TYPE_VALUE, T_BLEND_VALUE = 0x16, 0x17, 0x18, 0x19, 0x1A
T_MINEMITTIME, T_LOCATIONONLY, T_SCALAR_EVENT, T_VECTOR_EVENT, T_COLOR_EVENT = 0x1B, 0x1D, 0x1E, 0x1F, 0x20
T_FADE, T_SO, T_AT, T_INITIAL, T_FINAL = 0x21, 0x22, 0x23, 0x24, 0x25
T_BOOL_PROP, T_NUMERIC_PROP, T_VECTOR_PROP, T_BLEND_PROP, T_OTHER = 0x26, 0x27, 0x28, 0x29, 0x2A

SINGLE_CHAR_TYPES = {"=": T_EQUALS, ",": T_COMMA, "(": T_OPEN, ")": T_CLOSE, "{": T_LBRACE, "}": T_RBRACE,
                     '"': T_QUOTED}

# Ordered substring checks; the first match wins. "SO" and "AT" must match the whole token.
CLASSIFY_ORDER = [
    (("STAYATEMITTER", "CENTERED", "SORT", "LOCAL"), T_BOOL_PROP),
    (("PARTICLESYSTEM",), T_PARTICLESYSTEM), (("EVENTSEQUENCE",), T_EVENTSEQUENCE), (("RANDOM",), T_RANDOM),
    (("XYZ",), T_XYZ), (("RGBA",), T_RGBA), (("FADE",), T_FADE), (("CELSETUP",), T_CELSETUP),
    (("INITIAL",), T_INITIAL), (("FINAL",), T_FINAL), (("TEXTURE",), T_TEXTURE), (("TYPE",), T_TYPE),
    (("INHERITED",), T_INHERITED), (("EMITTERNAME",), T_EMITTERNAME), (("MESHNAME",), T_MESHNAME),
    (("CELANIMATE",), T_CELANIMATE), (("BURST",), T_BURST), (("EMISSIONBOOST",), T_EMISSIONBOOST),
    (("COLLISIONCHECK",), T_COLLISIONCHECK),
    (("POINT", "MESH", "BILLBOARD", "EMITTER", "LIGHT", "TEST", "AXIAL"), T_TYPE_VALUE),
    ("=SO", T_SO), ("=AT", T_AT),
    (("ALPHA_",), T_BLEND_VALUE), (("SOURCEBLENDMODE", "DESTBLENDMODE"), T_BLEND_PROP),
    (("GRAVITY", "EMITRADIUS", "CHAOSLOCATION", "CHAOSVELOCITY"), T_VECTOR_PROP),
    (("LIFETIME", "EMITRATE", "NUMPARTICLES", "SCALEMULTIPLIER", "VELOCITYMULTIPLIER", "LOOPS"), T_NUMERIC_PROP),
    (("MINEMITTIME",), T_MINEMITTIME), (("LOCATIONONLY",), T_LOCATIONONLY),
    (("SCALE", "EVENTTIMER", "THRUST", "LENGTH"), T_SCALAR_EVENT), (("VELOCITY", "ROTATION"), T_VECTOR_EVENT),
    (("COLOR",), T_COLOR_EVENT),
]

DELIMITERS = '(),"{}'
C_SPACE = " \t\n\v\f\r"
HIGH_BIT = 0x80

# ---- values ----
PARTICLE_TYPES = [("POINT", 1), ("MESH", 2), ("BILLBOARD", 3), ("EMITTER", 4), ("AXIAL", 5), ("LIGHT", 6),
                  ("TEST", 8)]
RENDERED_TYPES = {1, 2, 4, 5}
BLEND_MODES = {"ALPHA_ONE": 0, "ALPHA_ZERO": 1, "ALPHA_SRCCOLOR": 2, "ALPHA_INVSRCCOLOR": 3,
               "ALPHA_DESTCOLOR": 4, "ALPHA_INVDESTCOLOR": 5, "ALPHA_SRCALPHA": 6, "ALPHA_INVSRCALPHA": 7,
               "ALPHA_DESTALPHA": 8, "ALPHA_INVDESTALPHA": 9, "ALPHA_SRCALPHASAT": 10}
# CreateEvent: substring order -> (event name, value kind, fade allowed, forbidden with StayAtEmitter)
EVENT_ORDER = [("SCALE", "scalar"), ("EVENTTIMER", "scalar"), ("REDCOLOR", "scalar"), ("GREENCOLOR", "scalar"),
               ("BLUECOLOR", "scalar"), ("ALPHA", "scalar"), ("COLOR", "rgba"), ("LENGTH", "scalar"),
               ("VELOCITYX", "scalar"), ("VELOCITYY", "scalar"), ("VELOCITYZ", "scalar"), ("VELOCITY", "xyz"),
               ("THRUST", "scalar"), ("ROTATION", "xyz")]
STAY_FORBIDDEN_EVENTS = {"VELOCITYX", "VELOCITYY", "VELOCITYZ", "VELOCITY", "THRUST"}
NUMERIC_PROPS = ["LIFETIME", "EMITRATE", "NUMPARTICLES", "SCALEMULTIPLIER", "VELOCITYMULTIPLIER", "LOOPS"]
VECTOR_PROPS = ["GRAVITY", "CHAOSLOCATION", "CHAOSVELOCITY"]  # EMITRADIUS has no handler
BOOL_PROPS = ["STAYATEMITTER", "CENTERED", "SORT", "LOCAL"]
BOOL_TRUE_CHARS, BOOL_FALSE_CHARS = "1TY", "0FN"
CEL_ARG_COUNT = 7
INT_RANGE_MASK = 0xFFFF
DEFAULT_LIFETIME = (0.0, 0.0)
FLOAT_PREFIX = re.compile(r"\s*[+-]?(\d+\.?\d*([eE][+-]?\d+)?|\.\d+([eE][+-]?\d+)?)")
INT_PREFIX = re.compile(r"\s*[+-]?\d+")


class ScriptError(Exception):
    pass


def c_atof(text):
    m = FLOAT_PREFIX.match(text)
    return float(m.group(0)) if m else 0.0


def c_atol(text):
    m = INT_PREFIX.match(text)
    return int(m.group(0)) if m else 0


def classify(raw):
    if raw[0].isdigit() or raw[0] == "-":
        return raw, T_NUMBER
    if raw[0] in SINGLE_CHAR_TYPES:  # by the first character: "(1" is an open parenthesis
        return raw, SINGLE_CHAR_TYPES[raw[0]]
    up = raw.upper()
    for keys, ttype in CLASSIFY_ORDER:
        if isinstance(keys, str):
            if up == keys[1:]:
                return up, ttype
        elif any(k in up for k in keys):
            return up, ttype
    return up, T_OTHER


def tokenize(text, quirks):
    """ChaosScript::Tokenize. Returns [(text, type)]; records quirk hits in `quirks`."""
    tokens = []
    state, tok = "space", ""

    def push(t):
        if t:
            tokens.append(classify(t))

    i, n = 0, len(text)
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if state == "space":
            if c in C_SPACE or ord(c) >= HIGH_BIT:
                pass
            elif c == "/" and nxt == "/":
                state, tok = "comment", ""
            elif c == '"':
                state, tok = "quoted", c
            else:
                state, tok = "word", c
                if c in DELIMITERS and nxt and nxt not in C_SPACE and nxt not in DELIMITERS:
                    quirks["delimiter merged with the next word"] += 1
        elif state == "word":
            if c in C_SPACE:
                push(tok)
                state, tok = "space", ""
            elif c in DELIMITERS:
                push(tok)
                if c == '"':
                    quirks["quote directly after a word"] += 1
                    state, tok = "quoted", tok + c
                else:
                    push(c)
                    tok = ""
            elif c == "/" and nxt == "/":
                if tok:
                    quirks["word dropped before //"] += 1
                state, tok = "comment", ""
            else:
                tok += c
        elif state == "quoted":
            tok += c
            if c == '"' and len(tok) > 1:
                push(tok)
                state, tok = "space", ""
        elif state == "comment":
            if c == "\n":
                state, tok = "space", ""
        i += 1
    if state in ("word", "quoted"):
        push(tok)
    return tokens


class Parser:
    def __init__(self, tokens, stats):
        self.t, self.i, self.stats = tokens, 0, stats

    def peek(self):
        if self.i >= len(self.t):
            raise ScriptError("unexpected end of script")
        return self.t[self.i]

    def take(self, ttype=None, what=""):
        tok = self.peek()
        if ttype is not None and tok[1] != ttype:
            raise ScriptError("expected %s, got %r" % (what or ttype, tok[0]))
        self.i += 1
        return tok

    def number(self):
        return c_atof(self.take(T_NUMBER, "number")[0])

    def range_(self):
        """ParseRange: number | RANDOM(a, b). Returns (min, max) as written."""
        if self.peek()[1] == T_RANDOM:
            self.take()
            self.take(T_OPEN, "(")
            lo = self.number()
            self.take(T_COMMA, ",")
            hi = self.number()
            self.take(T_CLOSE, ")")
            self.stats["values"]["random"] += 1
            if lo > hi:
                self.stats["quirks"]["Random(min > max)"] += 1
            return lo, hi
        v = self.number()
        return v, v

    def int_range(self):
        if self.peek()[1] == T_RANDOM:
            self.take()
            self.take(T_OPEN, "(")
            lo = c_atol(self.take(T_NUMBER, "number")[0]) & INT_RANGE_MASK
            self.take(T_COMMA, ",")
            hi = c_atol(self.take(T_NUMBER, "number")[0]) & INT_RANGE_MASK
            self.take(T_CLOSE, ")")
            return lo, hi
        v = c_atol(self.take(T_NUMBER, "number")[0]) & INT_RANGE_MASK
        return v, v

    def tuple_(self, head, count, kind):
        self.take(head, kind)
        self.take(T_OPEN, "(")
        out = []
        for k in range(count):
            if k:
                self.take(T_COMMA, ",")
            out.append(self.range_())
        self.take(T_CLOSE, ")")
        self.stats["values"][kind] += 1
        return out

    def equals(self):
        self.take(T_EQUALS, "=")

    def bool_(self):
        text = self.take()[0].upper()
        if any(ch in text for ch in BOOL_TRUE_CHARS):
            return True
        if any(ch in text for ch in BOOL_FALSE_CHARS):
            return False
        raise ScriptError("bad bool %r" % text)

    def script(self):
        self.take(T_PARTICLESYSTEM, "PARTICLESYSTEM")
        self.take(T_QUOTED, "system name")
        version = self.take(T_NUMBER, "version")[0]
        self.stats["versions"][version] += 1
        self.take(T_LBRACE, "{")
        system = {"min_emit_time": None, "sequences": []}
        while True:
            tok, ttype = self.peek()
            if ttype == T_RBRACE:
                self.take()
                break
            if ttype == T_EVENTSEQUENCE:
                self.take()
                system["sequences"].append(self.sequence())
            elif ttype == T_MINEMITTIME:
                self.take()
                self.equals()
                system["min_emit_time"] = self.range_()
                self.stats["keys"]["MINEMITTIME"] += 1
            elif ttype == T_LOCATIONONLY:
                self.take()
                self.equals()
                self.bool_()
                self.stats["keys"]["LOCATIONONLY"] += 1
            else:
                raise ScriptError('Unexpected "%s" in Particle System Block!' % tok)
        return system

    def sequence(self):
        self.take(T_QUOTED, "sequence name")
        self.take(T_LBRACE, "{")
        seq = {"flags": set(), "type": 0, "lifetime": DEFAULT_LIFETIME, "events": [], "blend": [0, 0],
               "scale_mult": 1.0, "vel_mult": 1.0}
        st = self.stats
        while True:
            tok, ttype = self.peek()
            if ttype == T_RBRACE:
                self.take()
                break
            is_event = ttype in (T_FADE, T_INITIAL, T_FINAL, T_AT)
            if not is_event and seq["events"]:
                raise ScriptError("Cannot specify any sequence properties after specifying events.")
            if is_event:
                self.event(seq)
                continue
            self.take()
            st["keys"][tok] += 1
            if ttype in (T_TEXTURE, T_MESHNAME, T_EMITTERNAME):
                self.equals()
                seq["resource"] = self.take(T_QUOTED, "file name")[0].strip('"')
            elif ttype == T_TYPE:
                self.equals()
                value = self.take()[0].upper()
                seq["type"] = next((code for name, code in PARTICLE_TYPES if name in value), None)
                if seq["type"] is None:
                    raise ScriptError("unknown particle type %r" % value)
                st["types"][value] += 1
            elif ttype == T_BLEND_PROP:
                self.equals()
                value = self.take()[0].upper()
                if value not in BLEND_MODES:
                    raise ScriptError("unknown blend mode %r" % value)
                seq["blend"][0 if "SOURCE" in tok else 1] = BLEND_MODES[value]
            elif ttype == T_NUMERIC_PROP:
                name = next((k for k in NUMERIC_PROPS if k in tok), None)
                self.equals()
                lo, hi = self.range_()
                if name == "LIFETIME":
                    seq["lifetime"] = (lo, hi)
                elif name == "SCALEMULTIPLIER":
                    seq["scale_mult"] = hi
                elif name == "VELOCITYMULTIPLIER":
                    seq["vel_mult"] = hi
                elif name == "EMITRATE" and lo != hi:
                    st["quirks"]["emitrate range (only max used)"] += 1
            elif ttype == T_VECTOR_PROP:
                name = next((k for k in VECTOR_PROPS if k in tok), None)
                if name is None:
                    raise ScriptError("Unknown sequence vector property!")
                if name != "GRAVITY" and "STAYATEMITTER" in seq["flags"]:
                    raise ScriptError("%s with StayAtEmitter" % name)
                self.equals()
                self.tuple_(T_XYZ, 3, "xyz")
            elif ttype == T_INHERITED:
                if "LOCAL" in seq["flags"]:
                    raise ScriptError("Inherited with Local")
                self.equals()
                self.tuple_(T_XYZ, 3, "xyz")
            elif ttype == T_BURST:
                self.equals()
                lo, hi = self.int_range()
                if lo != hi:
                    st["quirks"]["burst range (max exclusive)"] += 1
            elif ttype == T_EMISSIONBOOST:
                self.equals()
                self.range_()
                seq["flags"].add("EMISSIONBOOST")
            elif ttype == T_COLLISIONCHECK:
                self.equals()
                self.take(T_OPEN, "(")
                mode = self.take()[0].upper()
                self.take(T_COMMA, ",")
                self.range_()
                self.take(T_CLOSE, ")")
                seq["flags"].add("COLLISION")
                st["quirks"]["COLLISIONCHECK (hangs the client)"] += 1
                if "STOP" not in mode and "REFLECT" not in mode:
                    raise ScriptError("bad collision mode %r" % mode)
            elif ttype == T_CELANIMATE:
                self.equals()
                cel = self.tuple_(T_CELSETUP, CEL_ARG_COUNT, "celsetup")
                seq["flags"].add("CELANIMATE")
                st["cel"]["frames=%g cols=%g rows=%g fps=%g loop=%s" % (
                    cel[0][0], cel[3][0], cel[4][0], cel[5][0], cel[6][0] > 0)] += 1
            elif ttype == T_BOOL_PROP:
                name = next(k for k in BOOL_PROPS if k in tok)
                self.equals()
                if self.bool_():
                    other = {"STAYATEMITTER": "LOCAL", "LOCAL": "STAYATEMITTER"}.get(name)
                    if other in seq["flags"]:
                        raise ScriptError("%s with %s" % (name, other))
                    seq["flags"].add(name)
            else:
                raise ScriptError('Unexpected "%s" in event sequence' % tok)
        self.finish_sequence(seq)
        return seq

    def event(self, seq):
        st = self.stats
        fade = False
        if self.peek()[1] == T_FADE:
            self.take()
            self.take(T_SO, "SO")
            fade = True
        tok, ttype = self.take()
        if ttype == T_INITIAL:
            if fade:
                raise ScriptError("fade so initial")
            when, time = "initial", (0.0, 0.0)
        elif ttype == T_FINAL:
            when, time = "final", (seq["lifetime"][1], seq["lifetime"][1])
            if seq["lifetime"] == DEFAULT_LIFETIME:
                st["quirks"]["final before lifetime (time 0)"] += 1
        elif ttype == T_AT:
            when, time = "at", self.range_()
            if time[0] != time[1]:
                st["quirks"]["at Random (shared by all particles)"] += 1
        else:
            raise ScriptError("expected a time specifier, got %r" % tok)
        prop, ptype = self.take()
        if ptype not in (T_SCALAR_EVENT, T_VECTOR_EVENT, T_COLOR_EVENT):
            raise ScriptError("Expecting particle property after time specifier! (%r)" % prop)
        name, kind = next((k, v) for k, v in EVENT_ORDER if k in prop)
        if name in STAY_FORBIDDEN_EVENTS and "STAYATEMITTER" in seq["flags"]:
            raise ScriptError("%s event with StayAtEmitter" % name)
        self.equals()
        if kind == "scalar":
            self.range_()
        elif kind == "xyz":
            self.tuple_(T_XYZ, 3, "xyz")
        else:
            self.tuple_(T_RGBA, 4, "rgba")
        st["events"]["%s%s %s" % ("fade so " if fade else "", when, name)] += 1
        seq["events"].append({"name": name, "fade": fade, "time": time})

    def finish_sequence(self, seq):
        st = self.stats
        if seq["type"] not in RENDERED_TYPES:
            st["quirks"]["type renders nothing"] += 1
        st["blend pairs"][tuple(seq["blend"])] += 1
        # A fade with no later fade of the same type keeps its step for Scale/Color/Velocity.
        events = sorted(seq["events"], key=lambda e: e["time"][0])
        for k, ev in enumerate(events):
            later = [e for e in events[k + 1:] if e["name"] == ev["name"] and e["fade"]]
            if ev["fade"] and not later and ev["name"] in ("SCALE", "COLOR", "VELOCITY") and ev["time"][0] > 0 \
                    and ev["time"][0] < seq["lifetime"][1]:
                st["quirks"]["fade ends before lifetime max (step persists)"] += 1


def compile_text(text, stats):
    quirks = stats["quirks"]
    lexer_quirks = collections.Counter()
    tokens = tokenize(text, lexer_quirks)
    if not tokens or tokens[0][1] != T_PARTICLESYSTEM:
        return None
    quirks.update(lexer_quirks)
    for tok, ttype in tokens:
        if ttype == T_OTHER and tok == "ALPHA":
            quirks["ALPHA event (unusable)"] += 1
        if "EMITRADIUS" in tok:
            quirks["EMITRADIUS (unusable)"] += 1
    return Parser(tokens, stats).script()


def iter_files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, files in os.walk(p):
                for f in sorted(files):
                    yield os.path.join(root, f)
        else:
            yield p


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--dump", action="store_true", help="per-file results")
    args = ap.parse_args()
    stats = collections.defaultdict(collections.Counter)
    ok = failed = sequences = 0
    for path in iter_files(args.paths):
        with open(path, "rb") as fh:
            text = fh.read().decode("latin-1")
        try:
            system = compile_text(text, stats)
        except ScriptError as e:
            failed += 1
            print("FAIL %s: %s" % (path, e))
            continue
        if system is None:
            continue
        ok += 1
        sequences += len(system["sequences"])
        if args.dump:
            print("ok   %s: %d sequences, MinEmitTime %s" % (path, len(system["sequences"]), system["min_emit_time"]))
    print("scripts compiled: %d, failed: %d, sequences: %d" % (ok, failed, sequences))
    for section in ("versions", "keys", "types", "blend pairs", "events", "values", "cel", "quirks"):
        print("\n[%s]" % section)
        for key, count in sorted(stats[section].items(), key=lambda kv: (-kv[1], str(kv[0]))):
            print("  %5d  %s" % (count, key))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
